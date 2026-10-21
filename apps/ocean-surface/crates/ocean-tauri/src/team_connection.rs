//! Initial team-server intent. The daemon owns origin validation and the
//! first-write fence; this broker exposes no credentials or arbitrary routes.
use crate::daemon_transport::{require_main_window, DaemonRequest, DaemonTransport, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
enum Intent {
    Read,
    Save(String),
}
impl Intent {
    fn parse(action: &str, origin: Option<String>) -> Result<Self, String> {
        match (action, origin) {
            ("read", None) => Ok(Self::Read),
            ("save", Some(origin)) if bounded_origin(&origin) => Ok(Self::Save(origin)),
            _ => Err("invalid_team_action".into()),
        }
    }
    fn request(&self) -> DaemonRequest {
        let (method, body) = match self {
            Self::Read => (Method::Get, None),
            Self::Save(origin) => (
                Method::Put,
                Some(serde_json::json!({"origin":origin}).to_string()),
            ),
        };
        DaemonRequest::operator(method, "/v1/rooms/connection".into(), body)
    }
}
fn bounded_origin(origin: &str) -> bool {
    !origin.is_empty()
        && origin.len() <= 2048
        && origin.trim() == origin
        && !origin.chars().any(char::is_control)
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Connection {
    origin: Option<String>,
    configured: bool,
    restart_required: bool,
    can_initialize: bool,
}
fn project(body: Value) -> Result<Connection, String> {
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("invalid_team_reply".into());
    }
    let connection: Connection =
        serde_json::from_value(body["connection"].clone()).map_err(|_| "invalid_team_reply")?;
    if let Some(origin) = &connection.origin {
        let url = tauri::Url::parse(origin).map_err(|_| "invalid_team_reply")?;
        // Return only a bare origin, never userinfo or a token-bearing URL.
        if !bounded_origin(origin)
            || url.host_str().is_none()
            || !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid_team_reply".into());
        }
    }
    if (connection.configured && (connection.restart_required || connection.can_initialize))
        || (connection.restart_required && connection.can_initialize)
        || ((connection.configured || connection.restart_required) != connection.origin.is_some())
    {
        return Err("invalid_team_reply".into());
    }
    Ok(connection)
}
fn send(transport: &DaemonTransport, intent: &Intent) -> Result<Connection, String> {
    let response = transport
        .send(&intent.request())
        .map_err(|error| error.code().to_string())?;
    if !(200..300).contains(&response.status) {
        let code = response
            .body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(match (response.status, code) {
            (400, "invalid_team_origin") => "invalid_team_origin",
            (409, "team_origin_already_configured") => "team_origin_already_configured",
            (409, "existing_team_connection") => "existing_team_connection",
            (401 | 403, _) => "team_access_refused",
            (404, _) => "team_setup_unavailable",
            _ => "team_request_failed",
        }
        .into());
    }
    project(Value::Object(response.body))
}
pub(crate) struct ConnectionState(pub(crate) DaemonTransport);
#[tauri::command]
pub(crate) async fn team_connection(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConnectionState>,
    action: String,
    origin: Option<String>,
) -> Result<Connection, String> {
    require_main_window(window.label(), "team_window_refused")?;
    let intent = Intent::parse(&action, origin)?;
    let transport = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || send(&transport, &intent))
        .await
        .map_err(|_| "team_request_failed".to_string())?
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::daemon_transport::test_support::*;
    fn fixture(
        status: &str,
        body: &str,
        intent: Intent,
    ) -> (Result<Connection, String>, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http(status, body))]);
        let result = send(
            &DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key),
            &intent,
        );
        (result, daemon.join().unwrap())
    }
    #[test]
    fn team_connection_fixed_intents_and_main_window_only() {
        for (action, origin) in [
            ("delete", None),
            ("read", Some("https://team.test")),
            ("save", None),
            ("save", Some("https://team.test\n")),
            ("save", Some("")),
        ] {
            assert!(Intent::parse(action, origin.map(String::from)).is_err());
        }
        assert!(Intent::parse("save", Some("x".repeat(2049))).is_err());
        assert!(require_main_window("float", "team_window_refused").is_err());
    }
    #[test]
    fn team_connection_save_is_operator_gated_and_projects_only_state() {
        let body = r#"{"ok":true,"secret":"private","connection":{"origin":"https://team.test/","configured":false,"restart_required":true,"can_initialize":false,"token":"private"}}"#;
        let (reply, requests) = fixture("200 OK", body, Intent::Save("https://team.test".into()));
        assert!(!serde_json::to_string(&reply.unwrap())
            .unwrap()
            .contains("private"));
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("PUT /v1/rooms/connection HTTP/1.1"));
        assert!(requests[0].contains(&format!("X-Ocean-Operator: {KEY}")));
        assert!(requests[0].ends_with(r#"{"origin":"https://team.test"}"#));
        assert!(!requests[0].contains("Cookie:") && !requests[0].contains("Origin:"));
    }
    #[test]
    fn team_connection_rejects_secret_urls_and_contradictory_states() {
        for origin in [
            "https://user:secret@team.test/",
            "https://team.test/?token=secret",
            "https://team.test/private",
            "file:///private",
            "https://team.test/#secret",
        ] {
            assert!(project(serde_json::json!({"ok":true,"connection":{"origin":origin,"configured":true,"restart_required":false,"can_initialize":false}})).is_err());
        }
        for (origin, configured, restart, initialize) in [
            (None, true, false, false),
            (Some("https://team.test"), true, true, false),
            (Some("https://team.test"), false, true, true),
            (Some("https://team.test"), false, false, false),
        ] {
            assert!(project(serde_json::json!({"ok":true,"connection":{"origin":origin,"configured":configured,"restart_required":restart,"can_initialize":initialize}})).is_err());
        }
    }
    #[test]
    fn team_connection_errors_do_not_relay_server_details() {
        let (result, requests) = fixture(
            "409 Conflict",
            r#"{"error":"existing_team_connection","detail":"secret"}"#,
            Intent::Read,
        );
        assert_eq!(result.unwrap_err(), "existing_team_connection");
        assert!(requests[0].starts_with("GET /v1/rooms/connection HTTP/1.1"));
        let (result, _) = fixture(
            "500 Internal Server Error",
            r#"{"error":"secret"}"#,
            Intent::Read,
        );
        assert_eq!(result.unwrap_err(), "team_request_failed");
    }
    #[test]
    fn team_connection_lost_save_ack_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![None]);
        assert_eq!(
            send(
                &DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key),
                &Intent::Save("https://team.test".into())
            )
            .unwrap_err(),
            "outcome_unknown"
        );
        assert_eq!(daemon.join().unwrap().len(), 1);
    }
}
