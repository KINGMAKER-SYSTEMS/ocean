//! Desktop intent for daemon-owned provider login. No credentials or login URLs
//! cross IPC; only a fixed provider vocabulary and bounded state projections do.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon_transport::{require_main_window, DaemonRequest, DaemonTransport, Method};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Provider {
    Claude,
    Codex,
    Chatgpt,
}
impl Provider {
    fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Chatgpt => "chatgpt",
        }
    }
    fn authorize_base(self) -> &'static str {
        match self {
            Self::Claude => "https://claude.ai/oauth/authorize?",
            Self::Codex => "https://auth.openai.com/oauth/authorize?",
            Self::Chatgpt => "https://auth.openai.com/api/accounts/authorize?",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Intent {
    List,
    Login(Provider),
    Cancel(Provider, String),
}
impl Intent {
    fn parse(
        action: &str,
        provider: Option<Provider>,
        attempt: Option<String>,
    ) -> Result<Self, String> {
        let valid_attempt = |id: &str| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit());
        match (action, provider, attempt) {
            ("list", None, None) => Ok(Self::List),
            ("login", Some(p), None) => Ok(Self::Login(p)),
            ("cancel", Some(p), Some(id)) if valid_attempt(&id) => Ok(Self::Cancel(p, id)),
            _ => Err("invalid_account_action".into()),
        }
    }
    fn request(&self) -> DaemonRequest {
        let (method, path) = match self {
            Self::List => (Method::Get, "/v1/auth/providers".into()),
            Self::Login(p) => (Method::Post, format!("/v1/auth/providers/{}/login", p.id())),
            Self::Cancel(p, id) => (
                Method::Delete,
                format!("/v1/auth/providers/{}/login/{id}", p.id()),
            ),
        };
        DaemonRequest::operator(method, path, None)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountStatus {
    SignedIn,
    SignedOut,
    Expired,
    Unknown,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoginState {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Login {
    provider: Provider,
    attempt_id: String,
    state: LoginState,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Account {
    provider: Provider,
    status: AccountStatus,
    login: Option<Login>,
}
#[derive(Debug, Serialize, Default)]
pub(crate) struct Reply {
    #[serde(skip_serializing_if = "Option::is_none")]
    providers: Option<Vec<Account>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<Login>,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_opened: Option<bool>,
}

fn checked_login(
    value: Value,
    provider: Provider,
    expected: Option<&str>,
) -> Result<Login, String> {
    let login: Login = serde_json::from_value(value).map_err(|_| "invalid_account_reply")?;
    if login.provider != provider
        || login.attempt_id.len() != 32
        || !login.attempt_id.bytes().all(|b| b.is_ascii_hexdigit())
        || expected.is_some_and(|id| id != login.attempt_id)
    {
        return Err("invalid_account_reply".into());
    }
    Ok(login)
}

fn send(
    transport: &DaemonTransport,
    intent: &Intent,
    open: impl FnOnce(&str) -> bool,
) -> Result<Reply, String> {
    let response = transport
        .send(&intent.request())
        .map_err(|error| error.code().to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(match response.status {
            401 | 403 => "account_access_refused",
            404 => "account_unavailable",
            409 => "login_unavailable",
            _ => "account_request_failed",
        }
        .into());
    }
    let body = Value::Object(response.body);
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("invalid_account_reply".into());
    }
    match intent {
        Intent::List => {
            let providers: Vec<Account> = serde_json::from_value(body["providers"].clone())
                .map_err(|_| "invalid_account_reply")?;
            if providers.is_empty() || providers.len() > 3 {
                return Err("invalid_account_reply".into());
            }
            let mut seen = Vec::new();
            for account in &providers {
                if seen.contains(&account.provider) {
                    return Err("invalid_account_reply".into());
                }
                seen.push(account.provider);
                if let Some(login) = &account.login {
                    checked_login(
                        serde_json::to_value(login).map_err(|_| "invalid_account_reply")?,
                        account.provider,
                        None,
                    )?;
                }
            }
            Ok(Reply {
                providers: Some(providers),
                ..Reply::default()
            })
        }
        Intent::Login(provider) => {
            let login = checked_login(body.clone(), *provider, None)?;
            let url = body["authorize_url"]
                .as_str()
                .ok_or("invalid_account_reply")?;
            if login.state != LoginState::Pending
                || url.len() > 8192
                || !url.starts_with(provider.authorize_base())
                || url.bytes().any(|b| b <= 32 || b == 127)
                || url.contains('#')
            {
                return Err("invalid_account_reply".into());
            }
            Ok(Reply {
                login: Some(login),
                browser_opened: Some(open(url)),
                ..Reply::default()
            })
        }
        Intent::Cancel(provider, id) => Ok(Reply {
            login: Some(checked_login(body, *provider, Some(id))?),
            ..Reply::default()
        }),
    }
}

pub(crate) struct AccountState(pub(crate) DaemonTransport);

#[tauri::command]
pub(crate) async fn provider_accounts(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AccountState>,
    action: String,
    provider: Option<Provider>,
    attempt_id: Option<String>,
) -> Result<Reply, String> {
    require_main_window(window.label(), "account_window_refused")?;
    let intent = Intent::parse(&action, provider, attempt_id)?;
    let transport = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        send(&transport, &intent, |url| {
            tauri_plugin_opener::open_url(url, None::<&str>).is_ok()
        })
    })
    .await
    .map_err(|_| "account_request_failed".to_string())?
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::daemon_transport::test_support::*;
    const ID: &str = "0123456789abcdef0123456789abcdef";
    fn call(body: &str, intent: Intent, opens: bool) -> Result<Reply, String> {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", body))]);
        let transport = DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key);
        let result = send(&transport, &intent, |_| {
            assert!(opens);
            true
        });
        let seen = daemon.join().unwrap();
        assert!(seen[0].contains(&format!("X-Ocean-Operator: {KEY}")));
        assert!(!seen[0].contains("Cookie:") && !seen[0].contains("Origin:"));
        assert!(!format!("{result:?}").contains(KEY));
        result
    }
    #[test]
    fn provider_accounts_allow_only_fixed_routes_and_exact_attempts() {
        for (action, provider, id) in [
            ("logout", Some(Provider::Claude), None),
            ("list", Some(Provider::Codex), None),
            ("login", Some(Provider::Codex), Some(ID.into())),
            ("poll", Some(Provider::Codex), Some("../logout".into())),
            ("poll", None, Some(ID.into())),
        ] {
            assert!(Intent::parse(action, provider, id).is_err());
        }
        assert_eq!(
            Intent::parse("cancel", Some(Provider::Claude), Some(ID.into())).unwrap(),
            Intent::Cancel(Provider::Claude, ID.into())
        );
        assert!(require_main_window("float", "account_window_refused").is_err());
        assert!(serde_json::from_str::<Provider>("\"openai/../x\"").is_err());
    }
    #[test]
    fn provider_accounts_projection_drops_tokens_paths_and_login_urls() {
        let reply=call(r#"{"ok":true,"access_token":"secret","providers":[{"provider":"claude","status":"signed_in","login":null,"source":"/private/path","refresh_token":"secret"}]}"#,Intent::List,false).unwrap();
        let json = serde_json::to_string(&reply).unwrap();
        assert!(!json.contains("secret") && !json.contains("private") && !json.contains("source"));
    }
    #[test]
    fn provider_accounts_opens_only_the_expected_provider_and_never_returns_url() {
        let body = serde_json::json!({"ok":true,"provider":"codex","attempt_id":ID,"state":"pending","authorize_url":"https://auth.openai.com/oauth/authorize?state=ephemeral"});
        let reply = call(&body.to_string(), Intent::Login(Provider::Codex), true).unwrap();
        assert!(!serde_json::to_string(&reply).unwrap().contains("ephemeral"));
        for url in [
            "https://auth.openai.com.evil.test/oauth/authorize?x=1",
            "https://auth.openai.com@evil.test/oauth/authorize?x=1",
            "https://claude.ai/oauth/authorize?x=1",
            "file:///tmp/run",
            "https://auth.openai.com/oauth/authorize?x=1\n",
        ] {
            let mut body = body.clone();
            body["authorize_url"] = url.into();
            assert!(call(&body.to_string(), Intent::Login(Provider::Codex), false).is_err());
        }
    }
    #[test]
    fn provider_accounts_rejects_stale_or_mismatched_results() {
        for (provider, id) in [
            ("claude", ID),
            ("codex", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        ] {
            let body = serde_json::json!({"ok":true,"provider":provider,"attempt_id":id,"state":"succeeded"});
            assert!(call(
                &body.to_string(),
                Intent::Cancel(Provider::Codex, ID.into()),
                false
            )
            .is_err());
        }
    }
    #[test]
    fn provider_accounts_cancel_uses_delete_and_never_retries_lost_ack() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![None]);
        let transport = DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key);
        assert_eq!(
            send(
                &transport,
                &Intent::Cancel(Provider::Codex, ID.into()),
                |_| panic!()
            )
            .unwrap_err(),
            "outcome_unknown"
        );
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].starts_with(&format!(
            "DELETE /v1/auth/providers/codex/login/{ID} HTTP/1.1"
        )));
    }
}
