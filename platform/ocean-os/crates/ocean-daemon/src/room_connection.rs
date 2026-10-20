//! Operator-owned first team-service selection. No credential or membership
//! changes occur here; the federation supervisor activates configuration at boot.

use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    room_agent_authority::{operator, ApiError, MapOnly},
    room_federation::ConnectionError,
    AppState,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionRequest {
    origin: String,
}

fn failure(error: ConnectionError) -> (StatusCode, Json<Value>) {
    match error {
        ConnectionError::InvalidOrigin => ApiError::bad_request("invalid_team_origin"),
        ConnectionError::AlreadyConfigured => ApiError::conflict("team_origin_already_configured"),
        ConnectionError::ExistingFederation => ApiError::conflict("existing_team_connection"),
        ConnectionError::Unavailable => {
            ApiError::service_unavailable("team_connection_unavailable")
        }
    }
    .response()
}

pub(super) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(error) = operator(&state, &headers) {
        return error.response();
    }
    match state.room_federation.connection_status() {
        Ok(connection) => (
            StatusCode::OK,
            Json(json!({"ok": true, "connection": connection})),
        ),
        Err(error) => failure(error),
    }
}

pub(super) async fn put(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<MapOnly<ConnectionRequest>>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    if let Err(error) = operator(&state, &headers) {
        return error.response();
    }
    let Ok(Json(body)) = body else {
        return ApiError::bad_request("invalid_request").response();
    };
    match state
        .room_federation
        .initialize_connection(&body.into_inner().origin)
    {
        Ok(connection) => (
            StatusCode::OK,
            Json(json!({"ok": true, "connection": connection})),
        ),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request},
        Router,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn fixture(tmp: &tempfile::TempDir) -> (AppState, Router) {
        let state = crate::tests::fake_convene_state(tmp);
        let app = crate::app_router(
            crate::BrowserOrigins::default(),
            crate::AllowedHosts::default(),
        )
        .with_state(state.clone());
        (state, app)
    }

    async fn request(
        app: Router,
        method: Method,
        body: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri("/v1/rooms/connection")
            .header("content-type", "application/json");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = app
            .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    const OWNER: &[(&str, &str)] = &[("x-ocean-operator", "test-room-operator")];
    const BODY: &str = r#"{"origin":"https://team.example"}"#;

    #[tokio::test]
    async fn room_connection_requires_operator_and_refuses_ambient_or_foreign_requests() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, app) = fixture(&tmp);
        for method in [Method::GET, Method::PUT] {
            let (status, _) = request(app.clone(), method.clone(), BODY, &[]).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            for headers in [
                vec![("x-ocean-operator", "wrong")],
                vec![
                    ("x-ocean-operator", "test-room-operator"),
                    ("cookie", "session=fixture"),
                ],
                vec![
                    ("x-ocean-operator", "test-room-operator"),
                    ("origin", "https://evil.example"),
                ],
            ] {
                let (status, _) = request(app.clone(), method.clone(), BODY, &headers).await;
                assert_eq!(status, StatusCode::FORBIDDEN);
            }
        }
        assert!(
            state
                .rooms
                .lock()
                .unwrap()
                .room_connection()
                .unwrap()
                .can_initialize
        );
    }

    #[tokio::test]
    async fn room_connection_saves_normalized_origin_without_claiming_connectivity() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, app) = fixture(&tmp);
        let (status, before) = request(app.clone(), Method::GET, "", OWNER).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(before["connection"]["can_initialize"], true);
        let (status, saved) = request(app.clone(), Method::PUT, BODY, OWNER).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            saved["connection"],
            json!({"origin":"https://team.example/", "configured":false, "restart_required":true, "can_initialize":false})
        );
        let (status, replay) = request(app.clone(), Method::PUT, BODY, OWNER).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved, replay);
        let (status, error) = request(
            app.clone(),
            Method::PUT,
            r#"{"origin":"https://other.example"}"#,
            OWNER,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"], "team_origin_already_configured");
        assert_eq!(request(app, Method::GET, "", OWNER).await.1, saved);
        assert!(state
            .rooms
            .lock()
            .unwrap()
            .list_credentialed_rooms()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn room_connection_rejects_malformed_objects_and_origins_without_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, app) = fixture(&tmp);
        for body in [
            r#"["https://team.example"]"#,
            r#"{"origin":"https://team.example","extra":true}"#,
            r#"{"origin":"https://one.example","origin":"https://two.example"}"#,
            r#"{"origin":null}"#,
        ] {
            assert_eq!(
                request(app.clone(), Method::PUT, body, OWNER).await.0,
                StatusCode::BAD_REQUEST
            );
        }
        for origin in [
            "http://team.example",
            "https://user:secret@team.example",
            "https://team.example/path",
            "https://team.example?token=secret",
            "https://team.example#fragment",
            " https://team.example",
            "file:///tmp/config",
            "",
        ] {
            let body = json!({"origin": origin}).to_string();
            let (status, error) = request(app.clone(), Method::PUT, &body, OWNER).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(error["error"], "invalid_team_origin");
            assert!(!error.to_string().contains("secret"));
        }
        assert!(
            state
                .rooms
                .lock()
                .unwrap()
                .room_connection()
                .unwrap()
                .can_initialize
        );
    }

    #[tokio::test]
    async fn room_connection_refuses_pending_authority_but_unconfigured_join_creates_none() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, app) = fixture(&tmp);
        assert!(matches!(
            state.room_federation.redeem_invite("fixture-code").await,
            Err(crate::room_federation::IntentError::Unavailable)
        ));
        assert!(state
            .rooms
            .lock()
            .unwrap()
            .list_pending_redemptions()
            .unwrap()
            .is_empty());
        state
            .rooms
            .lock()
            .unwrap()
            .get_or_insert_pending_redemption(
                "legacy-code",
                "legacy-redemption",
                "fixture-token",
                chrono::Utc::now(),
            )
            .unwrap();
        let (status, error) = request(app, Method::PUT, BODY, OWNER).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"], "existing_team_connection");
        assert_eq!(
            state
                .rooms
                .lock()
                .unwrap()
                .room_connection()
                .unwrap()
                .origin,
            None
        );
    }
}
