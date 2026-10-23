//! Room attachments through the proxy's `/v1/rooms/persistent/{*rest}`
//! wildcard: the raw-bytes upload lane and the header-preserving download
//! lane (ported from Risingtides-dev/ocean-surface's proxy).
//!
//! Two things the generic JSON forwarder got wrong for attachments, each
//! pinned here against the PRODUCTION router over a real loopback upstream:
//!
//! - the 1 MiB body ceiling made the daemon's 8 MiB cap unreachable from a
//!   browser, so every upload over 1 MiB died here as an untyped 413. The
//!   upload lane buffers up to the daemon's cap PLUS its slack, so a body a
//!   little over the cap still arrives and comes back as the daemon's typed
//!   `attachment_too_large`; a gigabyte is still refused here. Every other
//!   shape keeps the JSON ceiling.
//! - re-stamping every buffered reply `application/json` destroyed the
//!   download's derived content type, `Content-Disposition: attachment` and
//!   `X-Content-Type-Options: nosniff` — the three headers that stop an
//!   uploader-declared `text/html` from executing on this origin. The
//!   download lane copies exactly those three across.
//!
//! The lanes are chosen from path SEGMENTS, never a substring probe, because
//! `{key}` is caller-supplied and a room keyed `attachments` must not inherit
//! a route's forwarding rules. Attachments are member-lane writes: the proxy
//! injects no operator key, and the session gate applies like everywhere.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::DefaultBodyLimit;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::*;
use crate::{
    is_owner_mutation, rooms_persistent_shape, RoomsPersistentShape, ATTACHMENT_UPLOAD_BODY_LIMIT,
    ROOMS_JSON_BODY_LIMIT, SESSION_COOKIE,
};

/// What the recording upstream saw of the last upload.
#[derive(Clone, Debug, Default)]
struct SeenUpload {
    target: String,
    content_type: Option<String>,
    body_len: usize,
    first_byte: Option<u8>,
    last_byte: Option<u8>,
    operator: Option<String>,
    cookie: bool,
    origin: bool,
}

type UploadLog = Arc<Mutex<Option<SeenUpload>>>;

#[derive(Clone)]
struct Upstream {
    requests: Arc<AtomicUsize>,
    uploads: UploadLog,
}

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nnot really a png";

async fn spawn_attachments_daemon() -> (String, Upstream, tokio::task::JoinHandle<()>) {
    async fn upload(
        axum::extract::State(upstream): axum::extract::State<Upstream>,
        req: Request<Body>,
    ) -> (StatusCode, Json<Value>) {
        upstream.requests.fetch_add(1, Ordering::Relaxed);
        let target = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_default();
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let operator = req
            .headers()
            .get("x-ocean-operator")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let cookie = req.headers().contains_key(header::COOKIE);
        let origin = req.headers().contains_key(header::ORIGIN);
        let body = to_bytes(req.into_body(), usize::MAX)
            .await
            .expect("upload body");
        *upstream.uploads.lock().unwrap() = Some(SeenUpload {
            target,
            content_type,
            body_len: body.len(),
            first_byte: body.first().copied(),
            last_byte: body.last().copied(),
            operator,
            cookie,
            origin,
        });
        (
            StatusCode::CREATED,
            Json(json!({
                "ok": true,
                "attachment": { "id": "0123456789abcdef0123456789abcdef", "byte_len": body.len() }
            })),
        )
    }

    async fn list(axum::extract::State(upstream): axum::extract::State<Upstream>) -> Json<Value> {
        upstream.requests.fetch_add(1, Ordering::Relaxed);
        Json(json!({ "ok": true, "attachments": [], "closed": false }))
    }

    async fn download(
        axum::extract::State(upstream): axum::extract::State<Upstream>,
        axum::extract::Path((_key, id)): axum::extract::Path<(String, String)>,
    ) -> Response {
        upstream.requests.fetch_add(1, Ordering::Relaxed);
        if id == "missing" {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "code": "unknown_attachment", "error": "no such file" })),
            )
                .into_response();
        }
        // Exactly the daemon's download headers: a type derived from the
        // bytes, nosniff, and an attachment disposition.
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/png"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"shot.png\"",
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                // Noise the proxy must NOT relay: only the three above cross.
                (header::SERVER, "upstream-daemon"),
            ],
            Bytes::from_static(PNG_BYTES),
        )
            .into_response()
    }

    async fn message(
        axum::extract::State(upstream): axum::extract::State<Upstream>,
    ) -> Json<Value> {
        upstream.requests.fetch_add(1, Ordering::Relaxed);
        Json(json!({ "ok": true }))
    }

    let upstream = Upstream {
        requests: Arc::new(AtomicUsize::new(0)),
        uploads: Arc::new(Mutex::new(None)),
    };
    let app = Router::new()
        .route(
            "/v1/rooms/persistent/{key}/attachments",
            post(upload).get(list),
        )
        .route("/v1/rooms/persistent/{key}/attachments/{id}", get(download))
        .route("/v1/rooms/persistent/{key}/messages", post(message))
        // The daemon's own layer: cap plus slack, so the proxy's ceiling is
        // what decides here, never axum's 2 MiB default on this fixture.
        .layer(DefaultBodyLimit::max(ATTACHMENT_UPLOAD_BODY_LIMIT))
        .with_state(upstream.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind attachments upstream");
    let addr = listener.local_addr().expect("attachments upstream addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), upstream, handle)
}

fn attachments_state(daemon_url: String, operator_key_path: PathBuf) -> Arc<AppState> {
    let mut state = auth_test_state();
    let inner = Arc::get_mut(&mut state).expect("sole state owner");
    inner.daemon_url = daemon_url;
    inner.operator_key_path = operator_key_path;
    state
}

/// A readable operator key on disk, login on: if an attachment route were an
/// owner-mutation route the proxy would inject this and the upstream would
/// echo it back.
fn write_operator_key(dir: &std::path::Path) -> PathBuf {
    let key_path = dir.join("operator.key");
    std::fs::write(&key_path, "proxy-owned-authority\n").expect("write operator key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("chmod operator key");
    key_path
}

fn session_request(method: &str, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}=test-session; ambient=browser"),
        )
        .header("x-ocean-operator", "browser-forged")
        .header(header::HOST, "127.0.0.1:8790")
        .header(header::ORIGIN, "http://127.0.0.1:8790")
        .body(body)
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("json body");
    serde_json::from_slice(&body).expect("json")
}

/// The lane a persistent-rooms request takes must come from reconstructed
/// SEGMENTS, never a substring probe: a room named after a route must not
/// steal that route's forwarding rules.
#[test]
fn attachment_lanes_are_classified_by_segment_shape() {
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/team/events"),
        RoomsPersistentShape::EventsTail
    );
    assert_eq!(
        rooms_persistent_shape(&Method::POST, "/v1/rooms/persistent/team/attachments"),
        RoomsPersistentShape::AttachmentUpload
    );
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/team/attachments/abc123"),
        RoomsPersistentShape::AttachmentDownload
    );

    // The LIST shares the upload's path and is ordinary JSON both ways.
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/team/attachments"),
        RoomsPersistentShape::Json
    );
    // A delete answers JSON; only the GET of the bytes needs the header
    // passthrough.
    assert_eq!(
        rooms_persistent_shape(
            &Method::DELETE,
            "/v1/rooms/persistent/team/attachments/abc123"
        ),
        RoomsPersistentShape::Json
    );
    // Everything else in the subtree stays on the JSON lane.
    assert_eq!(
        rooms_persistent_shape(&Method::POST, "/v1/rooms/persistent/team/messages"),
        RoomsPersistentShape::Json
    );
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/team"),
        RoomsPersistentShape::Json
    );
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent"),
        RoomsPersistentShape::Json
    );
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/inbox"),
        RoomsPersistentShape::Json
    );

    // A room KEYED after a route must not inherit that route's lane — the
    // exact class of mistake a loose ends_with would make.
    assert_eq!(
        rooms_persistent_shape(&Method::POST, "/v1/rooms/persistent/attachments"),
        RoomsPersistentShape::Json
    );
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/events"),
        RoomsPersistentShape::Json
    );
    // Deeper than the route: not ours to special-case.
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/t/attachments/a/b"),
        RoomsPersistentShape::Json
    );
    // An empty id is not an id.
    assert_eq!(
        rooms_persistent_shape(&Method::GET, "/v1/rooms/persistent/t/attachments/"),
        RoomsPersistentShape::Json
    );
}

/// The ceiling the upload lane buys: the daemon's 8 MiB cap plus its 4096
/// byte slack, so a just-over-cap body reaches the daemon's typed 413. The
/// JSON ceiling stays at 1 MiB for every other shape.
#[test]
fn the_upload_ceiling_clears_the_daemon_cap_and_the_json_ceiling_does_not() {
    assert_eq!(ATTACHMENT_UPLOAD_BODY_LIMIT, 8 * 1024 * 1024 + 4096);
    assert_eq!(ROOMS_JSON_BODY_LIMIT, 1 << 20);
    assert!(ATTACHMENT_UPLOAD_BODY_LIMIT > ROOMS_JSON_BODY_LIMIT);
}

#[test]
fn attachment_routes_are_not_owner_mutation_routes() {
    for (method, path) in [
        (Method::POST, "/v1/rooms/persistent/team/attachments"),
        (Method::GET, "/v1/rooms/persistent/team/attachments"),
        (Method::GET, "/v1/rooms/persistent/team/attachments/abc"),
        (Method::DELETE, "/v1/rooms/persistent/team/attachments/abc"),
    ] {
        assert!(
            !is_owner_mutation(&method, path),
            "{method} {path} must never receive the proxy's operator key"
        );
    }
}

#[tokio::test]
async fn an_upload_over_the_json_ceiling_reaches_the_daemon_whole() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let key_path = write_operator_key(credential_dir.path());
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(attachments_state(daemon_url, key_path), dist.path());

    // Three MiB: over the JSON ceiling that used to kill it, under the cap.
    let mut body = vec![0x89u8; 3 * 1024 * 1024];
    *body.last_mut().unwrap() = 0x42;
    let response = app
        .oneshot({
            let mut req = session_request(
                "POST",
                "/v1/rooms/persistent/team/attachments\
                 ?filename=spec%20v2.pdf&content_type=application%2Fpdf&uploader_id=alice",
                Body::from(body),
            );
            req.headers_mut()
                .insert(header::CONTENT_TYPE, "application/pdf".parse().unwrap());
            req
        })
        .await
        .expect("upload response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let answer = json_body(response).await;
    assert_eq!(
        answer["ok"], true,
        "the daemon's JSON answer arrives intact"
    );
    assert_eq!(answer["attachment"]["byte_len"], 3 * 1024 * 1024);

    let seen = upstream
        .uploads
        .lock()
        .unwrap()
        .clone()
        .expect("upstream saw the upload");
    assert_eq!(
        seen.target,
        "/v1/rooms/persistent/team/attachments\
         ?filename=spec%20v2.pdf&content_type=application%2Fpdf&uploader_id=alice",
        "the metadata rides the query string and must arrive whole"
    );
    assert_eq!(seen.body_len, 3 * 1024 * 1024, "every byte crosses");
    assert_eq!(seen.first_byte, Some(0x89));
    assert_eq!(seen.last_byte, Some(0x42));
    assert_eq!(
        seen.content_type.as_deref(),
        Some("application/pdf"),
        "raw bytes are forwarded under the declared type, never stamped application/json"
    );
    assert_eq!(
        seen.operator, None,
        "neither the browser's forged header nor the proxy's key may cross"
    );
    assert!(!seen.cookie, "browser cookies never pass upstream");
    assert!(!seen.origin, "the browser Origin never passes upstream");
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    handle.abort();
}

#[tokio::test]
async fn an_upload_with_no_declared_type_is_forwarded_as_opaque_bytes() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "POST",
            "/v1/rooms/persistent/team/attachments?filename=a.bin&content_type=application%2Foctet-stream&uploader_id=alice",
            Body::from(vec![7u8; 16]),
        ))
        .await
        .expect("upload response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let seen = upstream
        .uploads
        .lock()
        .unwrap()
        .clone()
        .expect("upstream saw the upload");
    assert_eq!(
        seen.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(seen.body_len, 16);
    handle.abort();
}

#[tokio::test]
async fn a_body_past_the_upload_ceiling_is_refused_here_and_never_buffered_upstream() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "POST",
            "/v1/rooms/persistent/team/attachments?filename=huge.bin&content_type=application%2Foctet-stream&uploader_id=alice",
            Body::from(vec![0u8; ATTACHMENT_UPLOAD_BODY_LIMIT + 1]),
        ))
        .await
        .expect("oversize upload response");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        upstream.requests.load(Ordering::Relaxed),
        0,
        "a body the proxy refuses must not reach the daemon"
    );
    handle.abort();
}

#[tokio::test]
async fn a_just_over_cap_body_still_reaches_the_daemon_for_its_typed_refusal() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    // One byte over the daemon's cap, inside the slack: the DAEMON decides,
    // so the operator reads attachment_too_large instead of a proxy 413.
    let response = app
        .oneshot(session_request(
            "POST",
            "/v1/rooms/persistent/team/attachments?filename=edge.bin&content_type=application%2Foctet-stream&uploader_id=alice",
            Body::from(vec![0u8; 8 * 1024 * 1024 + 1]),
        ))
        .await
        .expect("edge upload response");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    handle.abort();
}

#[tokio::test]
async fn a_message_post_keeps_the_json_ceiling() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "POST",
            "/v1/rooms/persistent/team/messages",
            Body::from(vec![b' '; ROOMS_JSON_BODY_LIMIT + 1]),
        ))
        .await
        .expect("oversize message response");
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "the raised ceiling is the upload lane's alone"
    );
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 0);
    handle.abort();
}

#[tokio::test]
async fn a_download_keeps_the_daemons_type_disposition_and_nosniff() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "GET",
            "/v1/rooms/persistent/team/attachments/0123456789abcdef0123456789abcdef",
            Body::empty(),
        ))
        .await
        .expect("download response");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "image/png",
        "the derived type crosses verbatim — re-stamping application/json broke <img> and lost the defence"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"shot.png\""
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(
        !headers.contains_key(header::SERVER),
        "only the three security headers are copied; the rest of the response is ours"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("download bytes");
    assert_eq!(bytes.as_ref(), PNG_BYTES, "the bytes cross unchanged");
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    handle.abort();
}

#[tokio::test]
async fn a_download_refusal_arrives_as_the_daemons_json() {
    let (daemon_url, _upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "GET",
            "/v1/rooms/persistent/team/attachments/missing",
            Body::empty(),
        ))
        .await
        .expect("missing download response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("application/json"),
        "a refusal carries the daemon's own JSON type"
    );
    assert_eq!(json_body(response).await["code"], "unknown_attachment");
    handle.abort();
}

#[tokio::test]
async fn the_list_stays_on_the_json_lane() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    let response = app
        .oneshot(session_request(
            "GET",
            "/v1/rooms/persistent/team/attachments",
            Body::empty(),
        ))
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let body = json_body(response).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["closed"], false);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    handle.abort();
}

#[tokio::test]
async fn attachment_routes_are_login_gated_like_every_room_route() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    for (method, uri) in [
        (
            "POST",
            "/v1/rooms/persistent/team/attachments?filename=a&content_type=b&uploader_id=c",
        ),
        ("GET", "/v1/rooms/persistent/team/attachments"),
        (
            "GET",
            "/v1/rooms/persistent/team/attachments/0123456789abcdef0123456789abcdef",
        ),
    ] {
        let refused = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::from("x"))
                    .unwrap(),
            )
            .await
            .expect("unauthenticated attachment request");
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    assert_eq!(
        upstream.requests.load(Ordering::Relaxed),
        0,
        "an unauthenticated attachment request must not reach the daemon"
    );
    handle.abort();
}

#[tokio::test]
async fn attachment_routes_refuse_dot_segments_before_the_daemon() {
    let (daemon_url, upstream, handle) = spawn_attachments_daemon().await;
    let dist = tempfile::tempdir().expect("dist tempdir");
    let app = build_app(
        attachments_state(daemon_url, PathBuf::from("/not-used")),
        dist.path(),
    );
    for uri in [
        "/v1/rooms/persistent/other/../team/attachments?filename=a&content_type=b&uploader_id=c",
        "/v1/rooms/persistent/team/attachments/%2e%2e",
    ] {
        let response = app
            .clone()
            .oneshot(session_request("POST", uri, Body::from("x")))
            .await
            .expect("dot-segment attachment request");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 0);
    handle.abort();
}
