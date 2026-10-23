//! `POST /v1/rooms/persistent/{key}/close` through the proxy (Rooms DoD 4.3).
//!
//! The close route has two authorities and the daemon picks the lane by the
//! PRESENCE of `X-Ocean-Operator`: present means the operator lane, which is
//! deliberately not roster-checked; absent means `?actor_id=` naming a roster
//! member. The surface closes through the member lane only, so the posture the
//! proxy must hold is the one its sibling member-lane writes (post a message,
//! leave, delete an attachment) already have:
//!
//! - it rides the `{*rest}` wildcard, login-gated like every route;
//! - the query string — the member lane's whole authority claim — survives;
//! - no browser header crosses, and the proxy injects NO operator key, because
//!   the daemon would then close for anyone signed in to the proxy whether or
//!   not they are in the room;
//! - so an absent or unreadable operator key file cannot make it a 503;
//! - the daemon's refusal bodies arrive intact, since the surface reads
//!   `code` and the JSON `ok:false` off them;
//! - dot segments are refused before anything reaches the daemon.
//!
//! Ported from Risingtides-dev/ocean-surface#229. The monorepo proxy is
//! single-daemon (no device roster, no `device_unavailable`), its owner-
//! mutation rule is `is_owner_mutation`, and its room wildcard forwards PUT
//! too (prefs, agent settings), so the method test asks the daemon's POST-only
//! route to refuse PUT rather than the proxy.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};

use super::*;
use crate::{is_owner_mutation, SESSION_COOKIE};

async fn spawn_close_daemon() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    #[derive(serde::Deserialize)]
    struct CloseQuery {
        actor_id: Option<String>,
    }

    async fn close(
        axum::extract::State(requests): axum::extract::State<Arc<AtomicUsize>>,
        axum::extract::Path(key): axum::extract::Path<String>,
        axum::extract::Query(query): axum::extract::Query<CloseQuery>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        requests.fetch_add(1, Ordering::Relaxed);
        if key == "gone" {
            // `room_store_error_response(UnknownRoom)`: the daemon's close
            // route answers a not-open room with this exact unmarked shape.
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "error": "unknown room: gone" })),
            );
        }
        if query.actor_id.as_deref() == Some("researcher") {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "code": "forged_closer",
                    "error": "an agent does not close a room",
                })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "closed": true,
                "marker_seq": 9,
                "key": key,
                "actor_id": query.actor_id,
                "operator": headers
                    .get("x-ocean-operator")
                    .and_then(|value| value.to_str().ok()),
                "cookie": headers.contains_key(header::COOKIE),
                "origin": headers.contains_key(header::ORIGIN),
                "referer": headers.contains_key(header::REFERER),
            })),
        )
    }

    let requests = Arc::new(AtomicUsize::new(0));
    // POST-only, exactly like the daemon's registration: a forwarded PUT or
    // GET meets this router's own 405 and is never counted.
    let app = Router::new()
        .route("/v1/rooms/persistent/{key}/close", post(close))
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind close upstream");
    let addr = listener.local_addr().expect("close upstream addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), requests, handle)
}

fn close_state(daemon_url: String, operator_key_path: PathBuf, login: bool) -> Arc<AppState> {
    let mut state = auth_test_state();
    let inner = Arc::get_mut(&mut state).expect("sole state owner");
    inner.daemon_url = daemon_url;
    inner.operator_key_path = operator_key_path;
    if !login {
        inner.basic_auth = None;
    }
    state
}

async fn json_body(response: Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("close body");
    serde_json::from_slice(&body).expect("close json")
}

fn close_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[test]
fn close_is_not_an_owner_mutation_route() {
    assert!(
        !is_owner_mutation(&Method::POST, "/v1/rooms/persistent/team/close"),
        "close must never receive the proxy's operator key"
    );
}

#[tokio::test]
async fn close_rides_the_member_lane_and_never_carries_operator_authority() {
    let (daemon_url, requests, upstream) = spawn_close_daemon().await;
    // A VALID operator key is on disk and login is ON: if close were an
    // owner-mutation route the proxy would inject the key, and the upstream
    // would echo it back.
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let key_path = credential_dir.path().join("operator.key");
    std::fs::write(&key_path, "proxy-owned-authority\n").expect("write operator key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("chmod operator key");
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(close_state(daemon_url, key_path, true), dist.path());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rooms/persistent/team/close?actor_id=alice%26bob")
                // The operator's own session: admitted by the gate.
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}=test-session; ambient=browser"),
                )
                // Browser-supplied authority and ambient headers never cross.
                .header("x-ocean-operator", "browser-forged")
                .header(header::HOST, "127.0.0.1:8790")
                .header(header::ORIGIN, "http://127.0.0.1:8790")
                .header(header::REFERER, "http://127.0.0.1:8790/rooms")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("close response");
    assert_eq!(response.status(), StatusCode::OK);
    let seen = json_body(response).await;
    assert_eq!(seen["closed"], true);
    assert_eq!(seen["key"], "team");
    assert_eq!(
        seen["actor_id"], "alice&bob",
        "the query string is the member lane's authority claim and must arrive whole"
    );
    assert_eq!(
        seen["operator"],
        Value::Null,
        "neither the browser's header nor the proxy's key may select the operator lane"
    );
    assert_eq!(seen["cookie"], false);
    assert_eq!(seen["origin"], false);
    assert_eq!(seen["referer"], false);
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    upstream.abort();
}

#[tokio::test]
async fn close_does_not_depend_on_the_operator_credential() {
    let (daemon_url, requests, upstream) = spawn_close_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    // No key file at all, login on: an owner-mutation route answers 503 here.
    let app = build_app(
        close_state(daemon_url, PathBuf::from("/no/such/operator.key"), true),
        dist.path(),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rooms/persistent/team/close?actor_id=alice")
                .header(header::COOKIE, format!("{SESSION_COOKIE}=test-session"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("close response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    upstream.abort();
}

#[tokio::test]
async fn close_is_login_gated_like_every_room_route() {
    let (daemon_url, requests, upstream) = spawn_close_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        close_state(daemon_url, PathBuf::from("/not-used"), true),
        dist.path(),
    );

    let refused = app
        .clone()
        .oneshot(close_request(
            "POST",
            "/v1/rooms/persistent/team/close?actor_id=alice",
        ))
        .await
        .expect("unauthenticated close");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        requests.load(Ordering::Relaxed),
        0,
        "an unauthenticated close must not reach the daemon"
    );

    let admitted = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rooms/persistent/team/close?actor_id=alice")
                .header(header::COOKIE, format!("{SESSION_COOKIE}=test-session"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("authenticated close");
    assert_eq!(admitted.status(), StatusCode::OK);
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    upstream.abort();
}

#[tokio::test]
async fn nothing_closes_on_a_method_other_than_post() {
    let (daemon_url, requests, upstream) = spawn_close_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        close_state(daemon_url, PathBuf::from("/not-used"), false),
        dist.path(),
    );

    // The room wildcard forwards GET and PUT (reads; prefs and agent
    // settings), so both reach the daemon — whose close route is POST-only
    // — and its own 405 comes back. The upstream's counter proves the
    // handler never ran.
    for method in ["GET", "PUT"] {
        let response = app
            .clone()
            .oneshot(close_request(
                method,
                "/v1/rooms/persistent/team/close?actor_id=alice",
            ))
            .await
            .expect("non-POST close");
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} must not close a room"
        );
        assert_eq!(requests.load(Ordering::Relaxed), 0, "{method}");
    }
    upstream.abort();
}

#[tokio::test]
async fn close_refusals_reach_the_surface_with_their_body_intact() {
    let (daemon_url, _, upstream) = spawn_close_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        close_state(daemon_url, PathBuf::from("/not-used"), false),
        dist.path(),
    );

    let forged = app
        .clone()
        .oneshot(close_request(
            "POST",
            "/v1/rooms/persistent/team/close?actor_id=researcher",
        ))
        .await
        .expect("forged close");
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(forged).await["code"], "forged_closer");

    // The surface reads a JSON `ok:false` 404 as "already closed" and an
    // EMPTY 404 as "no such route", so the daemon's body must survive the
    // hop byte for byte.
    let gone = app
        .oneshot(close_request(
            "POST",
            "/v1/rooms/persistent/gone/close?actor_id=alice",
        ))
        .await
        .expect("already-closed close");
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    let body = json_body(gone).await;
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "unknown room: gone");
    upstream.abort();
}

#[tokio::test]
async fn close_refuses_dot_segments_before_the_daemon() {
    let (daemon_url, requests, upstream) = spawn_close_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        close_state(daemon_url, PathBuf::from("/not-used"), false),
        dist.path(),
    );
    for uri in [
        "/v1/rooms/persistent/other/../team/close?actor_id=alice",
        "/v1/rooms/persistent/%2e%2e/team/close?actor_id=alice",
    ] {
        let response = app
            .clone()
            .oneshot(close_request("POST", uri))
            .await
            .expect("dot-segment close");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(requests.load(Ordering::Relaxed), 0);
    upstream.abort();
}
