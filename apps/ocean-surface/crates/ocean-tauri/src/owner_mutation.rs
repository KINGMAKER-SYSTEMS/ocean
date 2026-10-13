//! Room owner mutations from the desktop shell (operator decision 2026-10-08).
//!
//! The daemon gates three owner mutations on its header-only Room operator
//! credential (`X-Ocean-Operator`): renaming the owner (`PUT /v1/me`),
//! deciding a room run's pending permission
//! (`POST /v1/rooms/persistent/{key}/runs/{run_id}/permission`) and saving a
//! room agent's settings (`PUT /v1/rooms/persistent/{key}/agents/{agent_id}/settings`).
//! The operator decided that any first-party Ocean surface the operator is
//! signed into may make exactly these three. The desktop shell runs as the
//! operator on the daemon's machine, so it attaches the daemon's owner-only
//! `operator.key` (the same path the Surface proxy reads) on the Rust side:
//!
//! * only these three routes, chosen by a fixed `kind`, never a page-supplied
//!   method or path;
//! * only from the `main` webview window;
//! * through the shared `daemon_transport.rs`, which owns every transport and
//!   custody rule: numeric-loopback `OCEAN_DAEMON_URL` only, key read through
//!   `O_NOFOLLOW` just before each request (owner-only, effective-uid owned,
//!   header-safe; never cached, returned, logged or part of an error), no
//!   page-supplied headers (no `Origin`, no `Cookie`), redirects refused and
//!   replies bounded to 256 KiB JSON objects.
//!
//! Every other operator-gated route stays unreachable from the page.

use serde::Serialize;
use serde_json::Value;

use crate::daemon_transport::{
    encode_segment, require_main_window, DaemonRequest, DaemonTransport, Method,
};

/// One of the three owner mutations. Nothing else can be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerMutation {
    RenameOwner,
    RunPermission { room: String, run_id: String },
    AgentSettings { room: String, agent_id: String },
}

impl OwnerMutation {
    /// Build from the page's invoke arguments. `kind` is a fixed vocabulary;
    /// ids must be non-empty and are percent-encoded into the path.
    pub(crate) fn parse(
        kind: &str,
        room: Option<&str>,
        target: Option<&str>,
    ) -> Result<Self, String> {
        let id = |value: Option<&str>, what: &str| -> Result<String, String> {
            match value {
                Some(v) if !v.is_empty() && v.len() <= 512 => Ok(v.to_string()),
                _ => Err(format!("{what} is required")),
            }
        };
        match kind {
            "rename_owner" if room.is_none() && target.is_none() => Ok(Self::RenameOwner),
            "run_permission" => Ok(Self::RunPermission {
                room: id(room, "room")?,
                run_id: id(target, "run id")?,
            }),
            "agent_settings" => Ok(Self::AgentSettings {
                room: id(room, "room")?,
                agent_id: id(target, "agent id")?,
            }),
            _ => Err("not an owner mutation".to_string()),
        }
    }

    pub(crate) fn method(&self) -> Method {
        match self {
            Self::RenameOwner | Self::AgentSettings { .. } => Method::Put,
            Self::RunPermission { .. } => Method::Post,
        }
    }

    pub(crate) fn path(&self) -> String {
        match self {
            Self::RenameOwner => "/v1/me".to_string(),
            Self::RunPermission { room, run_id } => format!(
                "/v1/rooms/persistent/{}/runs/{}/permission",
                encode_segment(room),
                encode_segment(run_id)
            ),
            Self::AgentSettings { room, agent_id } => format!(
                "/v1/rooms/persistent/{}/agents/{}/settings",
                encode_segment(room),
                encode_segment(agent_id)
            ),
        }
    }

    /// The privileged request for this mutation. The body is the page's JSON
    /// payload; method, path and headers are fixed here and in the transport.
    fn request(&self, body: &str) -> DaemonRequest {
        DaemonRequest::operator(self.method(), self.path(), Some(body.to_string()))
    }
}

/// The daemon's reply, relayed to the page (status + JSON object body text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OwnerMutationReply {
    pub(crate) status: u16,
    pub(crate) body: String,
}

/// Send one owner mutation through the shared loopback transport. Blocking;
/// call off the async runtime. Errors are fixed transport codes
/// (`daemon_not_loopback`, `operator_credential_*`, `daemon_unreachable`,
/// `request_too_large` when nothing was sent; `outcome_unknown` for a lost
/// acknowledgement; `redirect_refused`, `invalid_reply`, `reply_too_large`
/// for an unacceptable reply). None carries the key.
pub(crate) fn send(
    transport: &DaemonTransport,
    mutation: &OwnerMutation,
    body: &str,
) -> Result<OwnerMutationReply, String> {
    let reply = transport
        .send(&mutation.request(body))
        .map_err(|error| error.code().to_string())?;
    Ok(OwnerMutationReply {
        status: reply.status,
        body: Value::Object(reply.body).to_string(),
    })
}

/// Managed transport for `room_owner_mutation`.
pub(crate) struct OwnerMutationState(pub(crate) DaemonTransport);

/// One of the three Room owner mutations (`PUT /v1/me`, run permission
/// decision, room agent settings), sent to the loopback daemon with the
/// operator key read on the Rust side. The page picks a fixed `kind` and the
/// ids; it never supplies a method, path or header, and never sees the key.
/// Only the `main` window may call it.
#[tauri::command]
pub(crate) async fn room_owner_mutation(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, OwnerMutationState>,
    kind: String,
    room: Option<String>,
    target: Option<String>,
    body: String,
) -> Result<OwnerMutationReply, String> {
    require_main_window(window.label(), "owner_mutation_window_refused")?;
    let mutation = OwnerMutation::parse(&kind, room.as_deref(), target.as_deref())?;
    let transport = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || send(&transport, &mutation, &body))
        .await
        .map_err(|_| "owner mutation failed".to_string())?
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
    fn only_the_three_owner_routes_can_be_built() {
        assert_eq!(
            OwnerMutation::parse("rename_owner", None, None)
                .unwrap()
                .path(),
            "/v1/me"
        );
        let decision =
            OwnerMutation::parse("run_permission", Some("team room"), Some("run/1")).unwrap();
        assert_eq!(decision.method(), Method::Post);
        assert_eq!(
            decision.path(),
            "/v1/rooms/persistent/team%20room/runs/run%2F1/permission"
        );
        let settings = OwnerMutation::parse("agent_settings", Some("r"), Some("..")).unwrap();
        assert_eq!(settings.method(), Method::Put);
        assert_eq!(
            settings.path(),
            "/v1/rooms/persistent/r/agents/%2E%2E/settings"
        );
        assert!(settings.request("{}").is_operator());
        for (kind, room, target) in [
            ("bindings", Some("r"), Some("a")),
            ("rename_owner", Some("r"), None),
            ("run_permission", Some("r"), None),
            ("run_permission", None, Some("x")),
            ("agent_settings", Some(""), Some("a")),
            ("/v1/me", None, None),
        ] {
            assert!(
                OwnerMutation::parse(kind, room, target).is_err(),
                "{kind} {room:?} {target:?}"
            );
        }
    }

    #[test]
    fn owner_mutation_carries_the_operator_key_and_nothing_page_supplied() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) =
            fake_daemon(vec![Some(http("409 Conflict", "{\"ok\":false,\"x\":1}"))]);
        let mutation = OwnerMutation::parse("run_permission", Some("room"), Some("run-1")).unwrap();
        let body = r#"{"permission_id":"p","decision":"allow"}"#;
        let reply = send(&transport(port, &key), &mutation, body).unwrap();
        let seen = daemon.join().unwrap();
        assert_eq!(reply.status, 409);
        assert_eq!(reply.body, "{\"ok\":false,\"x\":1}");
        let request = &seen[0];
        assert!(request
            .starts_with("POST /v1/rooms/persistent/room/runs/run-1/permission HTTP/1.1\r\n"));
        assert!(request.contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        assert!(request.ends_with(body));
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("\r\norigin:") && !lower.contains("\r\ncookie:"));
        assert!(
            !format!("{reply:?}").contains(KEY),
            "the key never reaches the page"
        );
    }

    #[test]
    fn chunked_replies_are_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n"
                .to_string(),
        )]);
        let mutation = OwnerMutation::parse("rename_owner", None, None).unwrap();
        let reply = send(&transport(port, &key), &mutation, "{}").unwrap();
        daemon.join().unwrap();
        assert_eq!(
            reply,
            OwnerMutationReply {
                status: 200,
                body: "{\"a\":1}".into()
            }
        );
    }

    #[test]
    fn the_key_is_refused_unless_private_and_never_leaves_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let mutation = OwnerMutation::parse("rename_owner", None, None).unwrap();
        let (port, listener) = silent_daemon();
        // Missing, group-readable, or symlinked keys refuse before any connect.
        let missing = dir.path().join("absent.key");
        let error = send(&transport(port, &missing), &mutation, "{}").unwrap_err();
        assert_eq!(error, "operator_credential_unavailable");
        let loose = key_file(dir.path(), 0o644);
        let error = send(&transport(port, &loose), &mutation, "{}").unwrap_err();
        assert_eq!(error, "operator_credential_not_private");
        assert!(!error.contains(KEY));
        let private = key_file(dir.path(), 0o600);
        let link = dir.path().join("link.key");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert_eq!(
            send(&transport(port, &link), &mutation, "{}").unwrap_err(),
            "operator_credential_not_regular"
        );
        // An oversized page body is refused unsent.
        let huge = "x".repeat(64 * 1024 + 1);
        assert_eq!(
            send(&transport(port, &private), &mutation, &huge).unwrap_err(),
            "request_too_large"
        );
        assert_never_contacted(&listener);
        // A non-loopback or hostname daemon never receives the key: since the
        // shared transport, `localhost` disables owner mutations too.
        for url in [
            "http://192.0.2.1:4780",
            "http://localhost:4780",
            "https://127.0.0.1:4780",
        ] {
            let transport = DaemonTransport::new(url, private.clone());
            assert_eq!(
                send(&transport, &mutation, "{}").unwrap_err(),
                "daemon_not_loopback",
                "{url}"
            );
        }
    }

    #[test]
    fn an_owner_only_0400_key_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o400);
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", r#"{"ok":true}"#))]);
        let mutation = OwnerMutation::parse("rename_owner", None, None).unwrap();
        let reply = send(&transport(port, &key), &mutation, "{}").unwrap();
        assert_eq!(reply.status, 200);
        assert!(daemon.join().unwrap()[0].contains(KEY));
    }

    #[test]
    fn redirects_non_json_and_lost_acknowledgements_are_not_relayed() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let mutation = OwnerMutation::parse("agent_settings", Some("r"), Some("a")).unwrap();
        for (reply, code) in [
            (
                Some(
                    "HTTP/1.1 302 Found\r\nlocation: http://evil.example/\r\ncontent-length: 2\r\n\r\n{}"
                        .to_string(),
                ),
                "redirect_refused",
            ),
            (Some(http("400 Bad Request", "plain text")), "invalid_reply"),
            (Some(http("200 OK", "[]")), "invalid_reply"),
            (None, "outcome_unknown"),
        ] {
            let (port, daemon) = fake_daemon(vec![reply]);
            assert_eq!(
                send(&transport(port, &key), &mutation, "{}").unwrap_err(),
                code
            );
            assert_eq!(daemon.join().unwrap().len(), 1, "never retried");
        }
    }
}
