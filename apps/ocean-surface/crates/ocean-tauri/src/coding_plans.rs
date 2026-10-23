//! Coding-plan logins from the desktop shell (web identity M3).
//!
//! The daemon gates its five `/v1/auth/providers*` routes
//! (`platform/ocean-os/crates/ocean-daemon/src/provider_auth.rs`) on the same
//! header-only Room operator credential as the owner mutations. The shell runs
//! as the operator on the daemon's machine, so it attaches `operator.key` on
//! the Rust side exactly as `owner_mutation.rs` does:
//!
//! * only these five routes, chosen by a fixed `kind`, never a page-supplied
//!   method or path; provider and attempt ids are restricted to the daemon's
//!   own alphabet (`[A-Za-z0-9_-]{1,64}`) before they become path segments;
//! * only from the `main` webview window;
//! * through the shared `daemon_transport.rs` (numeric-loopback daemon only,
//!   key read just before each request, no page-supplied headers, redirects
//!   refused, replies bounded to JSON objects).
//!
//! The browser flow itself is unchanged: the daemon binds the provider's
//! localhost callback on this machine, which is where the shell runs.

use serde_json::Value;

use crate::daemon_transport::{require_main_window, DaemonRequest, DaemonTransport, Method};
use crate::owner_mutation::OwnerMutationReply;

/// One of the five coding-plan routes. Nothing else can be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanRoute {
    List,
    LoginStart { provider: String },
    LoginPoll { provider: String, attempt: String },
    LoginCancel { provider: String, attempt: String },
    Logout { provider: String },
}

/// The daemon's provider and attempt ids: alphanumeric, `-` and `_`, at most
/// 64 bytes. Anything else is refused before it can become a path segment.
fn segment(value: Option<&str>, what: &str) -> Result<String, String> {
    match value {
        Some(v)
            if !v.is_empty()
                && v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
        {
            Ok(v.to_string())
        }
        _ => Err(format!("{what} is required")),
    }
}

impl PlanRoute {
    /// Build from the page's invoke arguments. `kind` is a fixed vocabulary;
    /// an id the route does not take must be absent.
    pub(crate) fn parse(
        kind: &str,
        provider: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Self, String> {
        match kind {
            "list" if provider.is_none() && attempt.is_none() => Ok(Self::List),
            "login_start" if attempt.is_none() => Ok(Self::LoginStart {
                provider: segment(provider, "provider")?,
            }),
            "login_poll" => Ok(Self::LoginPoll {
                provider: segment(provider, "provider")?,
                attempt: segment(attempt, "attempt id")?,
            }),
            "login_cancel" => Ok(Self::LoginCancel {
                provider: segment(provider, "provider")?,
                attempt: segment(attempt, "attempt id")?,
            }),
            "logout" if attempt.is_none() => Ok(Self::Logout {
                provider: segment(provider, "provider")?,
            }),
            _ => Err("not a coding plan route".to_string()),
        }
    }

    pub(crate) fn method(&self) -> Method {
        match self {
            Self::List | Self::LoginPoll { .. } => Method::Get,
            Self::LoginStart { .. } | Self::Logout { .. } => Method::Post,
            Self::LoginCancel { .. } => Method::Delete,
        }
    }

    pub(crate) fn path(&self) -> String {
        match self {
            Self::List => "/v1/auth/providers".to_string(),
            Self::LoginStart { provider } => format!("/v1/auth/providers/{provider}/login"),
            Self::LoginPoll { provider, attempt } | Self::LoginCancel { provider, attempt } => {
                format!("/v1/auth/providers/{provider}/login/{attempt}")
            }
            Self::Logout { provider } => format!("/v1/auth/providers/{provider}/logout"),
        }
    }

    /// The privileged request: method, path and headers are fixed here and
    /// in the transport, and every route is bodiless.
    fn request(&self) -> DaemonRequest {
        DaemonRequest::operator(self.method(), self.path(), None)
    }
}

/// Send one coding-plan request through the shared loopback transport.
/// Blocking; call off the async runtime. Errors are the transport's fixed
/// codes; none carries the key.
pub(crate) fn send(
    transport: &DaemonTransport,
    route: &PlanRoute,
) -> Result<OwnerMutationReply, String> {
    let reply = transport
        .send(&route.request())
        .map_err(|error| error.code().to_string())?;
    Ok(OwnerMutationReply {
        status: reply.status,
        body: Value::Object(reply.body).to_string(),
    })
}

/// Managed transport for `coding_plan_request`.
pub(crate) struct CodingPlansState(pub(crate) DaemonTransport);

/// One of the five coding-plan routes (`kind`: `list`, `login_start`,
/// `login_poll`, `login_cancel`, `logout`), sent to the loopback daemon with
/// the operator key read on the Rust side. The page picks the kind and the
/// ids; it never supplies a method, path or header, and never sees the key.
/// Only the `main` window may call it.
#[tauri::command]
pub(crate) async fn coding_plan_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, CodingPlansState>,
    kind: String,
    provider: Option<String>,
    attempt: Option<String>,
) -> Result<OwnerMutationReply, String> {
    require_main_window(window.label(), "coding_plans_window_refused")?;
    let route = PlanRoute::parse(&kind, provider.as_deref(), attempt.as_deref())?;
    let transport = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || send(&transport, &route))
        .await
        .map_err(|_| "coding plan request failed".to_string())?
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::daemon_transport::test_support::*;
    use std::path::Path;

    fn transport(port: u16, key: &Path) -> DaemonTransport {
        DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key.to_path_buf())
    }

    #[test]
    fn only_the_five_contract_shapes_can_be_built() {
        let cases = [
            ("list", None, None, Method::Get, "/v1/auth/providers"),
            (
                "login_start",
                Some("claude"),
                None,
                Method::Post,
                "/v1/auth/providers/claude/login",
            ),
            (
                "login_poll",
                Some("claude"),
                Some("0a1b"),
                Method::Get,
                "/v1/auth/providers/claude/login/0a1b",
            ),
            (
                "login_cancel",
                Some("codex"),
                Some("0a1b"),
                Method::Delete,
                "/v1/auth/providers/codex/login/0a1b",
            ),
            (
                "logout",
                Some("chatgpt"),
                None,
                Method::Post,
                "/v1/auth/providers/chatgpt/logout",
            ),
        ];
        for (kind, provider, attempt, method, path) in cases {
            let route = PlanRoute::parse(kind, provider, attempt).unwrap();
            assert_eq!(route.method(), method, "{kind}");
            assert_eq!(route.path(), path, "{kind}");
            assert!(route.request().is_operator());
        }
        for (kind, provider, attempt) in [
            ("list", Some("claude"), None),
            ("login_start", None, None),
            ("login_start", Some("claude"), Some("x")),
            ("login_poll", Some("claude"), None),
            ("login_cancel", None, Some("x")),
            ("logout", Some("claude"), Some("x")),
            ("logout", Some(""), None),
            ("logout", Some("cl/aude"), None),
            ("logout", Some(".."), None),
            ("login_poll", Some("claude"), Some("a b")),
            ("login_poll", Some("claude"), Some(&"x".repeat(65))),
            ("rename_owner", None, None),
            ("/v1/auth/providers", None, None),
        ] {
            assert!(
                PlanRoute::parse(kind, provider, attempt).is_err(),
                "{kind} {provider:?} {attempt:?}"
            );
        }
    }

    #[test]
    fn coding_plan_requests_carry_the_operator_key_and_nothing_page_supplied() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http(
            "202 Accepted",
            r#"{"ok":true,"attempt_id":"a1","authorize_url":"https://claude.ai/x"}"#,
        ))]);
        let route = PlanRoute::parse("login_start", Some("claude"), None).unwrap();
        let reply = send(&transport(port, &key), &route).unwrap();
        let seen = daemon.join().unwrap();
        assert_eq!(reply.status, 202);
        let body: Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(body["attempt_id"], "a1");
        assert_eq!(body["authorize_url"], "https://claude.ai/x");
        assert_eq!(body["ok"], true);
        let request = &seen[0];
        assert!(request.starts_with("POST /v1/auth/providers/claude/login HTTP/1.1\r\n"));
        assert!(request.contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("\r\norigin:") && !lower.contains("\r\ncookie:"));
        assert!(
            !lower.contains("content-length:"),
            "every route is bodiless"
        );
        assert!(
            !format!("{reply:?}").contains(KEY),
            "the key never reaches the page"
        );
    }

    #[test]
    fn a_cancel_is_a_delete_and_a_missing_key_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http(
            "200 OK",
            r#"{"ok":true,"state":"cancelled"}"#,
        ))]);
        let route = PlanRoute::parse("login_cancel", Some("claude"), Some("a1")).unwrap();
        let reply = send(&transport(port, &key), &route).unwrap();
        assert_eq!(reply.status, 200);
        let seen = daemon.join().unwrap();
        assert!(seen[0].starts_with("DELETE /v1/auth/providers/claude/login/a1 HTTP/1.1\r\n"));

        let (port, listener) = silent_daemon();
        let missing = dir.path().join("absent.key");
        let error = send(&transport(port, &missing), &PlanRoute::List).unwrap_err();
        assert_eq!(error, "operator_credential_unavailable");
        assert_never_contacted(&listener);
    }
}
