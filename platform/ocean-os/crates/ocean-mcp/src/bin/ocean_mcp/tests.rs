use super::*;
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
    Router,
};
use std::sync::{Arc, Mutex};

type Records = Arc<Mutex<Vec<(String, String, Value)>>>;

struct Fixture {
    daemon: Daemon,
    records: Records,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture() -> Fixture {
    let records = Records::default();
    let router = Router::new().fallback(respond).with_state(records.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let daemon = Daemon::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fixture {
        daemon,
        records,
        task,
    }
}

async fn respond(State(records): State<Records>, request: Request) -> axum::response::Response {
    let path = request.uri().path().to_string();
    let uri = request.uri().to_string();
    let method = request.method().to_string();
    let headers = request.headers().clone();
    let body: Bytes = axum::body::to_bytes(request.into_body(), MAX_REQUEST)
        .await
        .unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("x-ocean-operator"));
    records.lock().unwrap().push((method, uri, body.clone()));
    if path == "/redirect" {
        return (StatusCode::FOUND, [("location", "/health")], "").into_response();
    }
    if path == "/large" {
        return "x".repeat(MAX_RESPONSE + 1).into_response();
    }
    if path == "/malformed" {
        return "not-json".into_response();
    }
    if path == "/lost" {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let value = if path == "/health" {
        json!({"ok":true,"service":"ocean-daemon","backend":"test","rev":"fixture"})
    } else if path == "/v1/rooms/persistent" {
        json!({"ok":true,"rooms":[{"id":"team","name":"Team","participants":[{"id":"stale-owner"}]},{"id":"local","name":"Local"},{"id":"missing","name":"Missing"}],"has_more":true,"next_cursor":"page+2"})
    } else if path == "/v1/rooms/persistent/team/snapshot" {
        shared()
    } else if path == "/v1/rooms/persistent/local/snapshot" {
        json!({"room":{"id":"local","name":"Local","participants":[{"id":"owner","display_name":"Owner","kind":"human"}]},"access":{"state":"local"}})
    } else if path.ends_with("/transcript") {
        json!({"ok":true,"closed":false,"transcript":[{"seq":31,"author_id":"peer","body":"A real reply","thread_parent_seq":21,"session_id":"session-1"}],"has_more":true,"next_seq":31})
    } else if path.ends_with("/messages") {
        json!({"ok":true,"message":{"seq":32,"author_id":"daemon-owner","body":body["body"]},"triggers_fired":[]})
    } else if path.ends_with("/participants") {
        json!({"ok":true,"participant":{"id":"daemon-owner"}})
    } else if path == "/v1/rooms/persistent/team/agents" {
        json!({"ok":true,"bindings":[{"agent_member_id":"agent","display_name":"Worker","status":"suspended","generation":"42","owner_member_id":"peer","private_debug":"never expose"}]})
    } else if path.ends_with("/resources") {
        json!({"ok":true,"resources":[{"resource_id":"folder-1","display_name":"Code","authorized_agent_member_ids":["agent"],"local_root":"/private/never","access_mode":"read","status":"available"}]})
    } else if path.ends_with("/profile") {
        json!({"ok":true,"credential_slots":[{"name":"KEY","status":"missing","required":true,"value":"never expose"}]})
    } else if path == "/v1/sessions" {
        json!({"ok":true,"sessions":[{"id":"s","title":"Work","turns":7,"model":"fixture","workspace_root":"/workspace"}],"has_more":true,"next_cursor":"next"})
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    axum::Json(value).into_response()
}

fn shared() -> Value {
    json!({"room":{"id":"team","name":"Team","participants":[{"id":"stale-owner","kind":"human"}]},
        "access":{"state":"live","local_member_id":"me","members":[{"member_id":"peer","display_name":"Peer","actor_type":"user","status":"active"},{"member_id":"agent","display_name":"Worker","actor_type":"agent","status":"active"}],"outbox":[{"bearer_token":"never expose"}]}})
}

#[test]
fn room_rosters_use_daemon_access_and_never_fall_back_to_stale_local_members() {
    let mut value = shared();
    let view = roster(&value).unwrap();
    assert_eq!(view["access"], "live");
    assert_eq!(view["members"].as_array().unwrap().len(), 2);
    assert!(!view.to_string().contains("stale-owner"));
    assert!(!view.to_string().contains("never expose"));
    for state in ["connecting", "recovering", "revoked"] {
        value["access"] = json!({"state":state});
        let view = roster(&value).unwrap();
        assert_eq!(view["access"], state);
        assert_eq!(view["members"], json!([]));
    }
    value["access"]["state"] = json!("unexpected");
    assert!(roster(&value).is_err());
    value["access"] = json!({"state":"live","members":"invalid"});
    assert!(roster(&value).is_err());
}

#[test]
fn daemon_origin_is_numeric_loopback_without_credentials_or_url_overrides() {
    for url in ["http://127.0.0.1:4780", "http://[::1]:4780"] {
        assert!(Daemon::new(url).is_ok(), "{url}");
    }
    for url in [
        "https://127.0.0.1:4780",
        "http://localhost:4780",
        "http://example.com",
        "http://user@127.0.0.1",
        "http://127.0.0.1/v1",
        "http://127.0.0.1?token=x",
        "http://127.0.0.1#x",
    ] {
        assert!(Daemon::new(url).is_err(), "{url}");
    }
    assert!(room_path("..", "snapshot").is_err());
    assert_eq!(
        room_path("a/b?c#d", "snapshot").unwrap(),
        "/v1/rooms/persistent/a%2Fb%3Fc%23d/snapshot"
    );
}

#[tokio::test]
async fn list_keeps_pagination_and_partial_failures_without_inventing_empty_rosters() {
    let f = fixture().await;
    let value = call_tool(
        &f.daemon,
        "ocean_rooms",
        json!({"cursor":"page+1","limit":3}),
    )
    .await
    .unwrap();
    assert_eq!(value["next_cursor"], "page+2");
    assert_eq!(value["rooms"][0]["members"][0]["display_name"], "Peer");
    assert_eq!(value["rooms"][1]["members"][0]["display_name"], "Owner");
    assert_eq!(value["rooms"][2]["error"], "room access unavailable");
    let records = f.records.lock().unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(records[0].1, "/v1/rooms/persistent?limit=3&cursor=page%2B1");
}

#[tokio::test]
async fn inspect_reads_actual_binding_resource_and_slot_routes_and_projects_no_secrets() {
    let f = fixture().await;
    let value = call_tool(&f.daemon, "ocean_room_inspect", json!({"room":"team"}))
        .await
        .unwrap();
    assert_eq!(value["access"], "live");
    assert_eq!(value["bindings"][0]["status"], "suspended");
    assert_eq!(
        value["resources"][0]["authorized_agent_member_ids"],
        json!(["agent"])
    );
    assert_eq!(value["credential_slots"][0]["status"], "missing");
    assert!(!value.to_string().contains("never"));
    assert!(f
        .records
        .lock()
        .unwrap()
        .iter()
        .all(|r| !r.1.contains("/inspect")));
    let local = call_tool(&f.daemon, "ocean_room_inspect", json!({"room":"local"}))
        .await
        .unwrap();
    assert_eq!(local["bindings_error"], "unavailable");
    assert!(local.get("bindings").is_none());
}

#[tokio::test]
async fn human_mutations_send_no_claimed_identity_and_reject_extra_authority() {
    let f = fixture().await;
    let reply = call_tool(
        &f.daemon,
        "ocean_room_post",
        json!({"room":"team","body":"hello","thread_parent_seq":21}),
    )
    .await
    .unwrap();
    assert_eq!(reply["message"]["author_id"], "daemon-owner");
    call_tool(&f.daemon, "ocean_room_join", json!({"room":"team"}))
        .await
        .unwrap();
    for args in [
        json!({"room":"team","body":"forged","author_id":"peer"}),
        json!({"room":"team","body":"forged","author_kind":"agent"}),
    ] {
        assert!(call_tool(&f.daemon, "ocean_room_post", args).await.is_err());
    }
    let records = f.records.lock().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].2, json!({"body":"hello","thread_parent_seq":21}));
    assert_eq!(records[1].2, json!({}));
}

#[tokio::test]
async fn transcript_uses_supported_forward_cursor_and_keeps_session_thread_links() {
    let f = fixture().await;
    let value = call_tool(
        &f.daemon,
        "ocean_room_read",
        json!({"room":"team","after_seq":30,"limit":10}),
    )
    .await
    .unwrap();
    assert_eq!(value["next_seq"], 31);
    assert_eq!(value["transcript"][0]["thread_parent_seq"], 21);
    assert_eq!(value["transcript"][0]["session_id"], "session-1");
    assert_eq!(
        f.records.lock().unwrap()[0].1,
        "/v1/rooms/persistent/team/transcript?limit=10&after_seq=30"
    );
    assert!(call_tool(
        &f.daemon,
        "ocean_room_read",
        json!({"room":"team","before_seq":30})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn session_turn_count_and_cursor_match_current_daemon() {
    let f = fixture().await;
    let value = call_tool(
        &f.daemon,
        "ocean_sessions",
        json!({"limit":2,"cursor":"old"}),
    )
    .await
    .unwrap();
    assert_eq!(value["sessions"][0]["turns"], 7);
    assert_eq!(value["next_cursor"], "next");
}

#[tokio::test]
async fn direct_prompts_are_unadvertised_and_refused_before_daemon_contact() {
    let f = fixture().await;
    let listed = handle(
        &f.daemon,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await
    .unwrap();
    assert!(listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool["name"] != "ocean_prompt"));

    for arguments in [
        json!({}),
        Value::Null,
        json!("malformed"),
        json!({"prompt":"write a file"}),
        json!({"prompt":"write a file","yolo":true}),
        json!({"prompt":"write a file","decision_token":"caller-supplied"}),
    ] {
        let reply = handle(
            &f.daemon,
            json!({"jsonrpc":"2.0","id":"turn","method":"tools/call",
                "params":{"name":"ocean_prompt","arguments":arguments}}),
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Direct prompts are unavailable"));
    }
    let omitted = handle(
        &f.daemon,
        json!({"jsonrpc":"2.0","id":"omitted","method":"tools/call",
            "params":{"name":"ocean_prompt"}}),
    )
    .await
    .unwrap();
    assert_eq!(omitted["result"]["isError"], true);
    assert!(f.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn redirects_large_invalid_and_failed_replies_are_visible_and_not_retried() {
    let f = fixture().await;
    for path in ["/redirect", "/large", "/malformed", "/lost"] {
        assert!(f
            .daemon
            .request(Method::POST, path, &[], Some(json!({})))
            .await
            .is_err());
    }
    assert_eq!(f.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn notifications_and_invalid_requests_never_execute_a_tool() {
    let f = fixture().await;
    assert!(handle(&f.daemon, json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"ocean_room_join","arguments":{"room":"team"}}})).await.is_none());
    let reply = handle(&f.daemon, json!({"id":1,"method":"tools/call","params":{"name":"ocean_room_join","arguments":{"room":"team"}}})).await.unwrap();
    assert_eq!(reply["error"]["code"], -32600);
    assert!(f.records.lock().unwrap().is_empty());
}
