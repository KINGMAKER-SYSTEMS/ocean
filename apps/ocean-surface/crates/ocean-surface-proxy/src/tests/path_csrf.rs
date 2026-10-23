//! Two proxy-boundary holes, pinned against the PRODUCTION router.
//!
//! Path normalisation. Every guard used to reason about the client's path
//! split on `/`, then `reqwest` parsed `format!("{base}{path}")` with the
//! WHATWG rules, which read `\` as `/` and collapse dot segments. hyper
//! accepts a raw `\` in the request target, so
//! `GET /v1/rooms/persistent/team\..\..\..\agent\sessions` looked like one
//! harmless room key to the dot guard and arrived upstream as
//! `GET /v1/agent/sessions`, a route the table never exposed — and on the
//! owner-mutation shapes the operator key rode along. Two independent halves
//! now hold it: `has_dot_segment` refuses `\` and `%5C`, and `upstream_url`
//! refuses any URL whose parsed path differs from the one the proxy decided
//! to forward. Captured `{id}` segments are re-encoded so a decoded `%2F`
//! cannot become a second segment.
//!
//! Auth-off CSRF. With `OCEAN_SURFACE_AUTH=off` there is no session secret,
//! so every non-GET/HEAD request under `/v1/` and `/api/` must meet the
//! Origin/Referer policy: a supplied browser source must name the exact
//! loopback Host that received the request; headerless clients (scripts,
//! CLIs) are still admitted.
//!
//! The upstream here is a recorder that answers 200 to ANY request and keeps
//! what it received, so "refused" is proven by an empty log rather than by
//! the absence of a route that would have 404'd anyway.

use super::*;
use crate::{
    auth_off_mutation_source_allowed, daemon_path_with_segment, is_loopback_authority,
    upstream_url, SESSION_COOKIE,
};
use axum::Json;
use serde_json::{json, Value};

const OPERATOR_KEY: &str = "proxy-owned-authority";
/// The session token `auth_test_state` is built with.
const SESSION: &str = "test-session";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    /// Path and query exactly as the upstream's HTTP server received them.
    target: String,
    operator: Option<String>,
}

type Log = Arc<Mutex<Vec<Seen>>>;

async fn spawn_recorder() -> (String, Log, tokio::task::JoinHandle<()>) {
    async fn record(
        axum::extract::State(log): axum::extract::State<Log>,
        req: Request<Body>,
    ) -> Json<Value> {
        log.lock().unwrap().push(Seen {
            method: req.method().to_string(),
            target: req
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_default(),
            operator: req
                .headers()
                .get("x-ocean-operator")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        });
        Json(json!({ "ok": true }))
    }
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().fallback(record).with_state(log.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind recorder");
    let addr = listener.local_addr().expect("recorder addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), log, handle)
}

fn seen(log: &Log) -> Vec<Seen> {
    log.lock().unwrap().clone()
}

fn write_operator_key(dir: &std::path::Path) -> PathBuf {
    let key_path = dir.join("operator.key");
    std::fs::write(&key_path, format!("{OPERATOR_KEY}\n")).expect("write operator key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("chmod operator key");
    key_path
}

/// `auth_test_state` pointed at `daemon_url`. `login = false` is auth-off
/// (loopback development); `login = true` keeps the operator login, whose
/// session is `SESSION`. A readable operator key is held either way, so a
/// slip would visibly carry it upstream.
fn state_at(daemon_url: &str, login: bool, key_path: PathBuf) -> Arc<AppState> {
    let mut state = auth_test_state();
    let inner = Arc::get_mut(&mut state).expect("sole state owner");
    inner.daemon_url = daemon_url.to_string();
    if !login {
        inner.basic_auth = None;
    }
    inner.operator_key_path = key_path;
    state
}

fn app_for(state: Arc<AppState>) -> (Router, tempfile::TempDir) {
    let dist = tempfile::tempdir().expect("dist tempdir");
    (build_app(state, dist.path()), dist)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(req).await.expect("response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn bare(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// The operator's own signed-in request (auth-on).
fn signed_in(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, format!("{SESSION_COOKIE}={SESSION}"))
        .body(Body::empty())
        .expect("request")
}

fn error_code(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.to_string())
}

// ── upstream_url, the parse-and-compare half ──────────────────────────────

#[test]
fn upstream_url_refuses_every_path_the_parser_would_rewrite() {
    let daemon = "http://127.0.0.1:4780";
    for path in [
        "/v1/rooms/persistent/team/agents/x\\..\\..\\..\\..\\..\\me/settings",
        "/v1/rooms/persistent/team\\..\\..\\..\\agent\\sessions",
        "/v1/rooms/persistent/team\\close",
        "/v1/rooms/persistent/../../v1/agent/turns",
        "/v1/rooms/persistent/%2e%2e/v1/agent/turns",
        "/v1/rooms/persistent/./events",
        "/v1/projects/a\tb",
        "/v1/projects/a\nb",
    ] {
        assert!(
            upstream_url(daemon, path, None).is_none(),
            "{path:?} must be refused: the parser would send a different path"
        );
    }
}

#[test]
fn upstream_url_keeps_a_legitimate_path_and_query_byte_for_byte() {
    let daemon = "http://127.0.0.1:4780/";
    let url = upstream_url(
        daemon,
        "/v1/rooms/persistent/room.v2/transcript",
        Some("after_seq=4&q=ali%20ce"),
    )
    .expect("legitimate path");
    assert_eq!(url.path(), "/v1/rooms/persistent/room.v2/transcript");
    assert_eq!(url.query(), Some("after_seq=4&q=ali%20ce"));

    // Every shape a first-party client actually sends: the Surface bundle
    // percent-encodes ids and room keys with the unreserved alphabet, and
    // the proxy encodes its own LiveKit room id the same way.
    for path in [
        "/v1/rooms/persistent/a%2Fb/messages",
        "/v1/sessions/a%20b",
        "/v1/rooms/project%3Asurface-main/livekit-token",
        "/v1/rooms/persistent/..config/events",
        "/v1/agent/sessions/s_1-x.y~z",
    ] {
        let url = upstream_url(daemon, path, None).expect(path);
        assert_eq!(url.path(), path, "{path} must be forwarded untouched");
    }

    // `%5C` is not rewritten by the parser, so THIS half lets it through —
    // `has_dot_segment` is the half that refuses it (tested below).
    assert!(upstream_url(daemon, "/v1/rooms/persistent/a%5Cb", None).is_some());

    // A daemon mounted under a path prefix keeps that prefix in the check.
    let prefixed = "http://127.0.0.1:4780/ocean";
    let url = upstream_url(prefixed, "/v1/models", None).expect("prefixed");
    assert_eq!(url.path(), "/ocean/v1/models");
    assert!(upstream_url(prefixed, "/v1/../../models", None).is_none());
}

// ── has_dot_segment, the string half ──────────────────────────────────────

#[test]
fn the_dot_guard_refuses_backslashes_raw_and_encoded_in_either_case() {
    for path in [
        "/v1/rooms/persistent/team\\..\\..\\..\\agent\\sessions",
        "/v1/rooms/persistent/team%5C..%5C..%5C..%5Cagent%5Csessions",
        "/v1/rooms/persistent/team%5c..%5c..%5c..%5cagent%5csessions",
        "/v1/rooms/persistent/team\\..%5C..%5c..\\agent\\sessions",
        "/v1/rooms/persistent/a%5Cb",
        "\\",
    ] {
        assert!(has_dot_segment(path), "{path:?} must be refused");
    }
    // Double-encoded is one decode away from `%5C` text, which nothing
    // downstream decodes again — same single-pass rule as `%252e`.
    assert!(!has_dot_segment("/v1/rooms/persistent/a%255Cb"));
    assert!(!has_dot_segment("/v1/rooms/persistent/room.v2/events"));
}

#[test]
fn a_captured_segment_is_rebuilt_as_exactly_one_segment() {
    // What axum hands a handler is already decoded once.
    assert_eq!(
        daemon_path_with_segment("/v1/sessions/", "a/b", "").as_deref(),
        Some("/v1/sessions/a%2Fb")
    );
    assert_eq!(
        daemon_path_with_segment("/v1/permissions/", "p?x=1", "/decision").as_deref(),
        Some("/v1/permissions/p%3Fx%3D1/decision")
    );
    assert_eq!(
        daemon_path_with_segment("/v1/rooms/", "project:surface-main", "/livekit-token").as_deref(),
        Some("/v1/rooms/project%3Asurface-main/livekit-token")
    );
    assert_eq!(
        daemon_path_with_segment("/v1/projects/", "room.v2", "").as_deref(),
        Some("/v1/projects/room.v2")
    );
    for refused in ["..", ".", "a/../b", "x\\..\\y", "%2e%2e", "a%5Cb"] {
        assert!(
            daemon_path_with_segment("/v1/projects/", refused, "").is_none(),
            "{refused:?} must be refused"
        );
    }
}

// ── through the production router ─────────────────────────────────────────

/// The backslash shapes, and their encoded/mixed siblings, for methods that
/// WOULD get the operator key (the owner-mutation shapes, keyed) and methods
/// that would not. Every one must die at the proxy with the dot guard's
/// answer.
const BACKSLASH_PROBES: &[(&str, &str)] = &[
    // keyed: the exact owner-mutation shapes `is_owner_mutation` approves by
    // segment, with a traversal hidden inside one segment
    (
        "PUT",
        "/v1/rooms/persistent/team/agents/x\\..\\..\\..\\..\\..\\me/settings",
    ),
    (
        "PUT",
        "/v1/rooms/persistent/team/agents/x%5C..%5C..%5C..%5C..%5C..%5Cme/settings",
    ),
    (
        "PUT",
        "/v1/rooms/persistent/team/agents/x%5c..\\..%5C..\\..%5c..\\me/settings",
    ),
    (
        "POST",
        "/v1/rooms/persistent/team/runs/x\\..\\..\\..\\..\\agent\\turns/permission",
    ),
    (
        "POST",
        "/v1/rooms/persistent/team/runs/x%5C..%5C..%5C..%5C..%5Cagent%5Cturns/permission",
    ),
    // unkeyed: rooms writes and reads
    ("POST", "/v1/rooms/persistent/team/messages\\..\\close"),
    ("POST", "/v1/rooms/persistent/team/messages%5C..%5Cclose"),
    (
        "GET",
        "/v1/rooms/persistent/team\\..\\..\\..\\agent\\sessions",
    ),
    (
        "GET",
        "/v1/rooms/persistent/team%5C..%5C..%5C..%5Cagent%5Csessions",
    ),
    ("GET", "/v1/rooms/persistent/team\\events"),
    (
        "DELETE",
        "/v1/rooms/persistent/team/participants/bob\\..\\..\\..\\close",
    ),
    // other families forwarding a client path or a captured id
    ("POST", "/v1/longhouse/demo\\..\\..\\agent\\turns"),
    ("POST", "/v1/longhouse/demo%5C..%5C..%5Cagent%5Cturns"),
    ("GET", "/v1/projects/x%5C..%5Cmodels"),
    ("PATCH", "/v1/projects/x\\..\\..\\model"),
    ("DELETE", "/v1/projects/x%5c..%5c..%5cagents%5cx"),
    ("GET", "/v1/sessions/x%5C..%5C..%5Cmodels"),
    ("GET", "/v1/agent/sessions/x\\..\\..\\..\\models"),
    ("POST", "/v1/agent/sessions/x%5C..%5C..%5Cturns/messages"),
    ("POST", "/v1/agent/sessions/x\\..\\..\\turns/steer"),
    ("POST", "/v1/permissions/x\\..\\..\\model/decision"),
    ("POST", "/v1/requests/x%5C..%5C..%5Cmodel/cancel"),
    ("POST", "/v1/rooms/x%5C..%5C..%5Cmodel/livekit-token"),
    ("GET", "/v1/repo/github/x\\..\\..\\agent\\sessions/pulls"),
    (
        "GET",
        "/v1/repo/github/p1/head-sha/x%5C..%5C..%5C..%5Cagent%5Csessions/checks",
    ),
];

#[tokio::test]
async fn backslash_probes_die_at_the_proxy_over_oneshot() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let key_path = write_operator_key(credential_dir.path());
    // Signed in, so the keyed shapes would carry the key if they got through.
    let (app, _dist) = app_for(state_at(&daemon_url, true, key_path));

    for (method, uri) in BACKSLASH_PROBES {
        let (status, body) = send(&app, signed_in(method, uri)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}");
        assert_eq!(
            body, "invalid path",
            "{method} {uri} — the dot guard's answer"
        );
    }
    assert!(
        seen(&log).is_empty(),
        "nothing may reach the daemon: {:?}",
        seen(&log)
    );
    upstream.abort();
}

/// Write one raw HTTP/1.1 request and return (status, body). Raw because the
/// point is a request target no client library would send: hyper on the
/// proxy's side accepts a bare `\`, which is how the probe gets in.
async fn raw_http(
    addr: std::net::SocketAddr,
    method: &str,
    target: &str,
    cookie: Option<&str>,
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect proxy");
    let cookie_line = cookie
        .map(|token| format!("Cookie: {SESSION_COOKIE}={token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\n{cookie_line}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read response");
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

async fn serve(app: Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, handle)
}

#[tokio::test]
async fn backslash_probes_die_at_the_proxy_over_real_tcp() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let key_path = write_operator_key(credential_dir.path());
    let (app, _dist) = app_for(state_at(&daemon_url, true, key_path));
    let (addr, proxy) = serve(app).await;

    // Control first: the raw transport really does forward, the path arrives
    // unchanged and the owner-mutation shape carries the key — so the
    // refusals below are the guard, not a broken harness.
    let (status, _) = raw_http(
        addr,
        "PUT",
        "/v1/rooms/persistent/team/agents/helper/settings",
        Some(SESSION),
    )
    .await;
    assert_eq!(status, 200);
    let control = seen(&log);
    assert_eq!(control.len(), 1);
    assert_eq!(
        control[0].target,
        "/v1/rooms/persistent/team/agents/helper/settings"
    );
    assert_eq!(control[0].operator.as_deref(), Some(OPERATOR_KEY));

    for (method, target) in BACKSLASH_PROBES {
        let (status, body) = raw_http(addr, method, target, Some(SESSION)).await;
        // hyper routes a raw `\` to the handler rather than rejecting it —
        // which is the whole hazard — so the refusal must be the proxy's own.
        assert_eq!(status, 400, "{method} {target}");
        assert_eq!(body, "invalid path", "{method} {target}");
    }
    let after = seen(&log);
    assert_eq!(
        after.len(),
        1,
        "only the control may reach the daemon: {after:?}"
    );
    proxy.abort();
    upstream.abort();
}

#[tokio::test]
async fn the_upstream_only_ever_receives_the_path_the_proxy_approved() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let key_path = write_operator_key(credential_dir.path());
    let (app, _dist) = app_for(state_at(&daemon_url, true, key_path));
    let (addr, proxy) = serve(app.clone()).await;

    // (method, what the client sends, what the daemon must receive, keyed?)
    let cases: &[(&str, &str, &str, bool)] = &[
        (
            "PUT",
            "/v1/rooms/persistent/team/agents/helper/settings",
            "/v1/rooms/persistent/team/agents/helper/settings",
            true,
        ),
        (
            "POST",
            "/v1/rooms/persistent/team/runs/run-1/permission",
            "/v1/rooms/persistent/team/runs/run-1/permission",
            true,
        ),
        ("PUT", "/v1/me", "/v1/me", true),
        (
            "POST",
            "/v1/rooms/persistent/team/messages",
            "/v1/rooms/persistent/team/messages",
            false,
        ),
        (
            "GET",
            "/v1/rooms/persistent/room.v2/transcript?after_seq=3",
            "/v1/rooms/persistent/room.v2/transcript?after_seq=3",
            false,
        ),
        (
            "GET",
            "/v1/rooms/inbox?limit=50",
            "/v1/rooms/inbox?limit=50",
            false,
        ),
        // A captured `{id}` is re-encoded into ONE segment: axum decoded the
        // `%2F`, and formatting it raw used to send `/v1/projects/a/b`.
        ("PATCH", "/v1/projects/a%2Fb", "/v1/projects/a%2Fb", false),
        ("GET", "/v1/sessions/a%2Fb", "/v1/sessions/a%2Fb", false),
        (
            "POST",
            "/v1/permissions/p%3Fx%3D1/decision",
            "/v1/permissions/p%3Fx%3D1/decision",
            false,
        ),
        (
            "POST",
            "/v1/agent/sessions/s%2F1/steer",
            "/v1/agent/sessions/s%2F1/steer",
            false,
        ),
        (
            "POST",
            "/v1/requests/r%2F1/cancel",
            "/v1/requests/r%2F1/cancel",
            false,
        ),
        (
            "POST",
            "/v1/rooms/project%3Asurface-main/livekit-token",
            "/v1/rooms/project%3Asurface-main/livekit-token",
            false,
        ),
        (
            "GET",
            "/v1/repo/github/p%2F1/pulls?state=open",
            "/v1/repo/github/p%2F1/pulls?state=open",
            false,
        ),
        // Longhouse forwards its raw path: `%3F` stays in the path instead of
        // becoming the start of a query.
        (
            "POST",
            "/v1/longhouse/demo%3Fforged=1",
            "/v1/longhouse/demo%3Fforged=1",
            false,
        ),
        // Query-carrying reads keep their query exactly.
        (
            "GET",
            "/v1/requests?state=running",
            "/v1/requests?state=running",
            false,
        ),
        (
            "GET",
            "/v1/fs/file?path=README.md",
            "/v1/fs/file?path=README.md",
            false,
        ),
        (
            "GET",
            "/v1/agent/sessions?limit=20&cursor=a%2Fb",
            "/v1/agent/sessions?limit=20&cursor=a%2Fb",
            false,
        ),
    ];
    for transport in ["oneshot", "tcp"] {
        for (method, sent, received, keyed) in cases {
            let before = seen(&log).len();
            let status = if transport == "tcp" {
                raw_http(addr, method, sent, Some(SESSION)).await.0
            } else {
                send(&app, signed_in(method, sent)).await.0.as_u16()
            };
            assert_eq!(status, 200, "{transport} {method} {sent}");
            let log_now = seen(&log);
            assert_eq!(log_now.len(), before + 1, "{transport} {method} {sent}");
            let got = &log_now[before];
            assert_eq!(&got.method, method, "{transport} {sent}");
            assert_eq!(&got.target, received, "{transport} {method} {sent}");
            assert_eq!(
                got.operator.is_some(),
                *keyed,
                "{transport} {method} {sent}: operator key only on owner-mutation shapes"
            );
        }
    }
    proxy.abort();
    upstream.abort();
}

#[tokio::test]
async fn a_captured_dot_segment_is_refused_on_every_id_family() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let (app, _dist) = app_for(state_at(&daemon_url, false, PathBuf::from("/not-used")));
    for (method, uri) in [
        ("GET", "/v1/sessions/%2e%2e"),
        // decoded by axum to `a/../models`: the guard sees the `..` inside
        ("GET", "/v1/sessions/a%2F..%2Fmodels"),
        ("GET", "/v1/agent/sessions/.."),
        ("GET", "/v1/projects/%2E%2E"),
        ("PATCH", "/v1/projects/.."),
        ("DELETE", "/v1/projects/%2e%2e"),
        ("POST", "/v1/agent/sessions/%2e%2e/messages"),
        ("POST", "/v1/agent/sessions/%2e%2e/steer"),
        ("POST", "/v1/permissions/../decision"),
        ("POST", "/v1/requests/%2e%2e/cancel"),
        ("POST", "/v1/rooms/%2e%2e/livekit-token"),
        ("GET", "/v1/repo/github/%2e%2e/pulls"),
        (
            "GET",
            "/v1/repo/github/p1/head-sha/..%2F..%2Fcommits/checks",
        ),
    ] {
        let (status, body) = send(&app, bare(method, uri)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}");
        assert_eq!(body, "invalid path", "{method} {uri}");
    }
    assert!(seen(&log).is_empty(), "{:?}", seen(&log));
    upstream.abort();
}

// ── auth-off CSRF across every mutating family ────────────────────────────

#[test]
fn auth_off_source_policy_admits_only_the_loopback_host_it_was_sent_to() {
    for host in [
        "localhost",
        "LOCALHOST:8790",
        "127.0.0.1",
        "127.5.5.5:80",
        "[::1]:8790",
    ] {
        assert!(is_loopback_authority(host), "{host}");
    }
    for host in [
        "",
        "10.0.0.5",
        "attacker.example",
        "localhost.evil",
        "surface.example:443",
        "not a host",
    ] {
        assert!(!is_loopback_authority(host), "{host}");
    }

    let headers = |host: Option<&str>, origin: Option<&str>, referer: Option<&str>| {
        let mut map = HeaderMap::new();
        if let Some(host) = host {
            map.insert(header::HOST, host.parse().unwrap());
        }
        if let Some(origin) = origin {
            map.insert(header::ORIGIN, origin.parse().unwrap());
        }
        if let Some(referer) = referer {
            map.insert(header::REFERER, referer.parse().unwrap());
        }
        map
    };
    // Headerless clients (scripts, CLIs) are admitted whatever the Host.
    assert!(auth_off_mutation_source_allowed(&headers(
        Some("127.0.0.1:8790"),
        None,
        None
    )));
    assert!(auth_off_mutation_source_allowed(&headers(
        Some("surface.example"),
        None,
        None
    )));
    assert!(auth_off_mutation_source_allowed(&headers(None, None, None)));
    // A browser source must name the exact loopback Host, either header.
    for (host, origin, referer) in [
        ("127.0.0.1:8790", Some("http://127.0.0.1:8790"), None),
        (
            "127.0.0.1:8790",
            None,
            Some("http://127.0.0.1:8790/rooms?x=1"),
        ),
        (
            "127.0.0.1:8790",
            Some("http://127.0.0.1:8790"),
            Some("http://127.0.0.1:8790/"),
        ),
        ("localhost:18790", Some("http://localhost:18790"), None),
        ("localhost:18790", Some("http://LOCALHOST:18790"), None),
        ("[::1]:8790", Some("http://[::1]:8790"), None),
        ("localhost:8790", Some("https://localhost:8790"), None),
    ] {
        assert!(
            auth_off_mutation_source_allowed(&headers(Some(host), origin, referer)),
            "{host} {origin:?} {referer:?}"
        );
    }
    for (host, origin, referer) in [
        // another site, or no site at all
        ("127.0.0.1:8790", Some("https://attacker.example"), None),
        (
            "127.0.0.1:8790",
            None,
            Some("https://attacker.example/form"),
        ),
        ("127.0.0.1:8790", Some("null"), None),
        // same host, other port: a different origin
        ("127.0.0.1:8790", Some("http://127.0.0.1:9999"), None),
        // another loopback spelling is still not THIS Host
        ("127.0.0.1:8790", Some("http://localhost:8790"), None),
        // one matching header does not excuse the other
        (
            "127.0.0.1:8790",
            Some("http://127.0.0.1:8790"),
            Some("https://attacker.example/"),
        ),
        // a non-http scheme, or a Host that is not loopback at all
        ("127.0.0.1:8790", Some("ftp://127.0.0.1:8790"), None),
        ("surface.example", Some("http://surface.example"), None),
        ("10.0.0.5:8790", Some("http://10.0.0.5:8790"), None),
    ] {
        assert!(
            !auth_off_mutation_source_allowed(&headers(Some(host), origin, referer)),
            "{host} {origin:?} {referer:?} must be refused"
        );
    }
    // A source header with no Host to compare against is refused.
    assert!(!auth_off_mutation_source_allowed(&headers(
        None,
        Some("http://127.0.0.1:8790"),
        None
    )));
}

fn from_source(method: &str, uri: &str, source: Option<(&str, &str)>) -> Request<Body> {
    from_source_typed(method, uri, source, "text/plain")
}

fn from_source_typed(
    method: &str,
    uri: &str,
    source: Option<(&str, &str)>,
    content_type: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "127.0.0.1:8790")
        // `text/plain` is what a cross-site form can send: no preflight.
        .header(header::CONTENT_TYPE, content_type);
    if let Some((name, value)) = source {
        builder = builder.header(name, value);
    }
    builder.body(Body::from("{}")).unwrap()
}

/// No-preflight mutations a hostile page can fire at a loopback proxy: every
/// non-GET route the table carries.
const CROSS_SITE_TARGETS: &[(&str, &str)] = &[
    ("POST", "/v1/rooms/persistent/team/close"),
    ("POST", "/v1/rooms/persistent/team/messages"),
    ("POST", "/v1/rooms/persistent"),
    (
        "DELETE",
        "/v1/rooms/persistent/team/participants/surface-operator",
    ),
    ("PATCH", "/v1/rooms/persistent/team"),
    ("PATCH", "/v1/rooms/persistent/team/read-cursor"),
    ("POST", "/v1/rooms/persistent/team/runs/run-1/permission"),
    ("PUT", "/v1/rooms/persistent/team/agents/helper/settings"),
    ("PUT", "/v1/me"),
    ("POST", "/v1/agent/turns"),
    ("POST", "/v1/agent/sessions"),
    ("POST", "/v1/agent/sessions/s1/messages"),
    ("POST", "/v1/agent/sessions/s1/steer"),
    ("POST", "/v1/permissions/p1/decision"),
    ("POST", "/v1/model"),
    ("POST", "/v1/projects"),
    ("PATCH", "/v1/projects/p1"),
    ("DELETE", "/v1/projects/p1"),
    ("POST", "/v1/requests/r1/cancel"),
    ("POST", "/v1/component/event"),
    ("POST", "/v1/calls/place"),
    ("POST", "/v1/voice/realtime/client-secret"),
    ("POST", "/v1/rooms/main/livekit-token"),
    ("POST", "/v1/longhouse/demo"),
    ("POST", "/v1/browser/input"),
    ("POST", "/api/stt"),
    ("POST", "/api/tts"),
];

#[tokio::test]
async fn auth_off_refuses_foreign_browser_sources_on_every_mutating_family() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let (app, _dist) = app_for(state_at(&daemon_url, false, PathBuf::from("/not-used")));

    for (method, uri) in CROSS_SITE_TARGETS {
        for source in [
            (header::ORIGIN.as_str(), "https://attacker.example"),
            (header::REFERER.as_str(), "https://attacker.example/form"),
            (header::ORIGIN.as_str(), "null"),
            // same host, other port: a different origin
            (header::ORIGIN.as_str(), "http://127.0.0.1:9999"),
        ] {
            let (status, body) = send(&app, from_source(method, uri, Some(source))).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri} {source:?}");
            assert_eq!(
                error_code(&body),
                "cross_site_mutation_refused",
                "{method} {uri} {source:?}"
            );
        }
    }
    assert!(seen(&log).is_empty(), "{:?}", seen(&log));

    // The login pair and the CSP sink sit outside the gate on purpose.
    for (method, uri) in [("POST", "/csp-report"), ("POST", "/logout")] {
        let (status, _) = send(
            &app,
            from_source(
                method,
                uri,
                Some((header::ORIGIN.as_str(), "https://attacker.example")),
            ),
        )
        .await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    upstream.abort();
}

#[tokio::test]
async fn auth_off_admits_same_origin_and_headerless_mutations_and_never_gates_reads() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let (app, _dist) = app_for(state_at(&daemon_url, false, PathBuf::from("/not-used")));

    for (method, uri) in CROSS_SITE_TARGETS {
        for source in [
            Some((header::ORIGIN.as_str(), "http://127.0.0.1:8790")),
            Some((header::REFERER.as_str(), "http://127.0.0.1:8790/rooms")),
            None,
        ] {
            let before = seen(&log).len();
            // The same-origin PWA sends JSON as JSON.
            let (status, body) = send(
                &app,
                from_source_typed(method, uri, source, "application/json"),
            )
            .await;
            assert_ne!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {uri} {source:?} {body}"
            );
            // `/api/*` handlers are not pass-throughs, so only the forwarded
            // families are asserted on the log.
            if uri.starts_with("/v1/") {
                assert_eq!(seen(&log).len(), before + 1, "{method} {uri} {source:?}");
            }
        }
    }

    // The other loopback spellings work when Host and Origin agree.
    for (host, origin) in [
        ("localhost:18790", "http://localhost:18790"),
        ("[::1]:8790", "http://[::1]:8790"),
    ] {
        let (status, body) = send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/v1/rooms/persistent/team/messages")
                .header(header::HOST, host)
                .header(header::ORIGIN, origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{host} {origin} {body}");
    }

    // Reads are never gated, whatever the source says.
    for uri in [
        "/v1/rooms/persistent/team/transcript",
        "/v1/agent/sessions",
        "/api/config",
    ] {
        let (status, _) = send(
            &app,
            Request::builder()
                .method("GET")
                .uri(uri)
                .header(header::HOST, "127.0.0.1:8790")
                .header(header::ORIGIN, "https://attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    upstream.abort();
}

#[tokio::test]
async fn auth_on_leaves_cross_site_defence_to_the_strict_session_cookie() {
    let (daemon_url, log, upstream) = spawn_recorder().await;
    let (app, _dist) = app_for(state_at(&daemon_url, true, PathBuf::from("/not-used")));

    // The cookie a cross-site request would need is never sent cross-site.
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=ocean&password=surface"))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");

    // Without it a cross-site POST is simply unauthenticated.
    let (status, _) = send(
        &app,
        from_source(
            "POST",
            "/v1/rooms/persistent/team/close",
            Some((header::ORIGIN.as_str(), "https://attacker.example")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(seen(&log).is_empty());

    // And with it, Origin is transport metadata, not a gate: a tunnel or
    // tailnet name is how auth-on is legitimately reached.
    let (status, _) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v1/rooms/persistent/team/close")
            .header(header::HOST, "surface.example")
            .header(header::ORIGIN, "https://surface.example")
            .header(header::COOKIE, format!("{SESSION_COOKIE}={SESSION}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen(&log).len(), 1);
    upstream.abort();
}
