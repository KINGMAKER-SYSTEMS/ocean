//! Room wire fixtures: the daemon's own answers on the room routes, pinned as
//! JSON under `tests/fixtures/room-wire/` so the Surface can decode them with
//! its production decoders (`apps/ocean-surface/crates/ocean-surface-ui`,
//! `src/rooms/wire_fixtures.rs`).
//!
//! In the retired two-repo world the Surface vendored hand-copied snapshots of
//! this wire (legacy ocean-surface#225/#233); nothing regenerated them, so they
//! rotted. Here every fixture is produced by driving the real `room_routes()`
//! router over a disposable store: the handler's own `json!` envelopes and
//! `ocean_core` serializers write the file, never a copy of them typed into a
//! test. Only two things are normalised after the fact so the files are
//! stable: RFC3339 clock readings under the timestamp keys, and the one
//! server-minted attachment id (both asserted to have the real shape first).
//!
//! When a serializer legitimately changes, regenerate from source and commit
//! the diff together with the change:
//!
//! ```sh
//! OCEAN_UPDATE_WIRE_FIXTURES=1 cargo test -p ocean-daemon room_wire_fixtures
//! ```
//!
//! then run `cargo test -p ocean-surface-ui wire_fixtures` from
//! `apps/ocean-surface`, which goes red when the Surface can no longer decode
//! what the daemon now sends. Names the Surface reads are the contract; a
//! field the Surface never decodes may still change shape here without a
//! Surface change, and the Surface test says so per fixture.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::Method;
use chrono::{DateTime, Utc};
use ocean_core::{
    FederatedActorType, FederatedRoomMemberProjection, FederatedRoomRole, MemberPresence,
    RoomAgentRunState, RoomRunPermission,
};
use ocean_store::{AgentBindingStatus, AuthorizeAgentInput, MemoryScope};

use super::*;

/// Where the fixtures live, beside the daemon's other golden files.
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/room-wire");
/// Set to regenerate the fixtures from the live serializers instead of
/// comparing against them.
const UPDATE_ENV: &str = "OCEAN_UPDATE_WIRE_FIXTURES";
/// Every clock reading the store stamped is rewritten to this instant, which
/// is itself a chrono rendering so the fixture still shows the real format.
const FIXED_INSTANT: &str = "2026-10-23T12:00:00.123456Z";
/// The server-minted `[0-9a-f]{32}` attachment id is rewritten to this one.
const FIXED_ATTACHMENT_ID: &str = "0123456789abcdef0123456789abcdef";
/// Keys whose string values are clock readings taken during the run. Values
/// the fixture itself pins (`expires_at`, `joined_at`, the run timestamps) are
/// fixed at construction and are not rewritten.
const CLOCK_KEYS: &[&str] = &["created_at", "updated_at", "uploaded_at", "authorized_at"];

const ROOM: &str = "wire-room";
const ANNEX: &str = "wire-annex";
const OPERATOR: &str = "test-room-operator";

/// Percent-encode one query value (the list cursor is an opaque string that
/// may carry characters a URI cannot).
fn query_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fixed_instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(FIXED_INSTANT)
        .expect("fixed instant parses")
        .with_timezone(&Utc)
}

/// One request through the production room router.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (StatusCode, Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(body.map(Body::from).unwrap_or_else(Body::empty))
        .expect("request builds");
    let response = crate::room_routes()
        .with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body reads");
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("{uri} answered non-JSON ({status}): {e}"));
    (status, value)
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    call(state, Method::GET, uri, &[], None).await
}

async fn send_json(
    state: &AppState,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, Value) {
    call(
        state,
        method,
        uri,
        headers,
        Some(body.to_string().into_bytes()),
    )
    .await
}

fn expect_status((status, value): (StatusCode, Value), expected: StatusCode, what: &str) -> Value {
    assert_eq!(status, expected, "{what} answered {status}: {value}");
    value
}

/// Walk a value and rewrite the run's clock readings to [`FIXED_INSTANT`],
/// asserting first that what the daemon wrote really is RFC3339.
fn normalise_clock(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if CLOCK_KEYS.contains(&key.as_str()) {
                    if let Value::String(text) = inner {
                        DateTime::parse_from_rfc3339(text)
                            .unwrap_or_else(|e| panic!("{key} = {text:?} is not RFC3339: {e}"));
                        *text = FIXED_INSTANT.to_string();
                        continue;
                    }
                }
                normalise_clock(inner);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalise_clock),
        _ => {}
    }
}

fn member(
    member_id: &str,
    actor_type: FederatedActorType,
    role: FederatedRoomRole,
    display_name: &str,
    descriptor: Option<PublicAgentDescriptor>,
    presence: Option<MemberPresence>,
    local_binding_available: Option<bool>,
) -> FederatedRoomMemberProjection {
    FederatedRoomMemberProjection {
        member_id: member_id.into(),
        owner_member_id: descriptor
            .as_ref()
            .map(|_| "22222222-2222-4222-8222-222222222222".to_string()),
        actor_type,
        role_in_room: role,
        display_name: display_name.into(),
        public_agent_descriptor: descriptor,
        joined_at: "2026-07-17T00:00:00Z".into(),
        derived_presence: presence,
        local_binding_available,
    }
}

fn outbox_item(client_event_id: &str, state: OutboxItemState) -> RoomOutboxItem {
    RoomOutboxItem {
        client_event_id: client_event_id.into(),
        source_id: "ocean-wire".into(),
        source_sequence: 10,
        author_member_id: "22222222-2222-4222-8222-222222222222".into(),
        event_type: "chat.message".into(),
        payload: json!({ "text": "pending on the wire" }),
        mention_member_ids: vec!["33333333-3333-4333-8333-000000000000".into()],
        state,
    }
}

/// The five access states the Surface renders, seeded the way the federation
/// supervisor would leave them in the store.
fn seeded_access_rooms() -> Vec<(&'static str, &'static str, RoomAccessProjection, bool)> {
    let live_members = vec![
        member(
            "11111111-1111-4111-8111-111111111111",
            FederatedActorType::User,
            FederatedRoomRole::Owner,
            "Owner",
            None,
            Some(MemberPresence::Live),
            None,
        ),
        member(
            "22222222-2222-4222-8222-222222222222",
            FederatedActorType::User,
            FederatedRoomRole::Member,
            "Joined Human",
            None,
            Some(MemberPresence::Unavailable),
            None,
        ),
        member(
            "33333333-3333-4333-8333-000000000000",
            FederatedActorType::Agent,
            FederatedRoomRole::Member,
            "Researcher",
            Some(PublicAgentDescriptor {
                display_name: "Researcher".into(),
                description: Some("Reads the room and reports back.".into()),
                model_alias: Some("fable".into()),
                skills_count: 3,
                subagent_names: vec!["scout".into()],
            }),
            Some(MemberPresence::Live),
            Some(true),
        ),
    ];
    vec![
        (
            "live",
            "wire-live",
            RoomAccessProjection {
                state: RoomAccessState::Live,
                caller_member_id: None,
                last_confirmed_global_sequence: Some(42),
                local_member_id: None,
                members: live_members,
                outbox: vec![
                    outbox_item("evt-pending", OutboxItemState::Pending),
                    outbox_item("evt-failed", OutboxItemState::Failed),
                ],
            },
            true,
        ),
        (
            "connecting",
            "wire-connecting",
            RoomAccessProjection {
                state: RoomAccessState::Connecting,
                caller_member_id: None,
                last_confirmed_global_sequence: None,
                local_member_id: None,
                members: vec![],
                outbox: vec![],
            },
            true,
        ),
        (
            "recovering",
            "wire-recovering",
            RoomAccessProjection {
                state: RoomAccessState::Recovering,
                caller_member_id: None,
                last_confirmed_global_sequence: Some(7),
                local_member_id: None,
                members: vec![],
                outbox: vec![],
            },
            true,
        ),
        (
            "revoked",
            "wire-revoked",
            RoomAccessProjection {
                state: RoomAccessState::Revoked,
                caller_member_id: None,
                last_confirmed_global_sequence: Some(7),
                local_member_id: None,
                members: vec![],
                outbox: vec![],
            },
            false,
        ),
    ]
}

/// One run per `RoomAgentRunState` variant, with the optional fields spread
/// across them so every shape the work card reads is on the wire once.
fn seeded_runs(root_seq: u64, reply_seq: u64) -> Vec<RoomAgentRun> {
    let started = fixed_instant();
    let later = started + chrono::Duration::seconds(30);
    let base = |run_id: &str, state: RoomAgentRunState| RoomAgentRun {
        run_id: run_id.into(),
        room_id: RoomKey::new(ROOM),
        agent_id: "researcher".into(),
        session_id: "session-researcher-1".into(),
        trigger_seq: root_seq,
        thread_root_seq: root_seq,
        state,
        started_at: started,
        updated_at: later,
        summary: None,
        files_changed: vec![],
        tool_count: 0,
        reply_seq: None,
        pending_permission: None,
        answer_seq: None,
        request_id: Some(format!("req-{run_id}")),
    };
    vec![
        RoomAgentRun {
            // A run stored before `request_id` existed decodes as absent.
            request_id: None,
            ..base("run-1-queued", RoomAgentRunState::Queued)
        },
        base("run-2-thinking", RoomAgentRunState::Thinking),
        RoomAgentRun {
            tool_count: 2,
            ..base(
                "run-3-running-tool",
                RoomAgentRunState::RunningTool {
                    label: "Reading notes.txt".into(),
                },
            )
        },
        RoomAgentRun {
            tool_count: 3,
            pending_permission: Some(RoomRunPermission {
                permission_id: "perm-1".into(),
                tool: "bash".into(),
                tool_label: "Run the test suite".into(),
            }),
            ..base(
                "run-4-awaiting-permission",
                RoomAgentRunState::AwaitingPermission,
            )
        },
        RoomAgentRun {
            answer_seq: Some(reply_seq),
            ..base("run-5-awaiting-reply", RoomAgentRunState::AwaitingReply)
        },
        RoomAgentRun {
            summary: Some("Fixed the flaky test.".into()),
            files_changed: vec!["crates/ocean-daemon/src/main.rs".into()],
            tool_count: 5,
            reply_seq: Some(reply_seq),
            ..base("run-6-done", RoomAgentRunState::Done)
        },
        base(
            "run-7-failed",
            RoomAgentRunState::Failed {
                reason: "provider unavailable".into(),
            },
        ),
        base("run-8-cancelled", RoomAgentRunState::Cancelled),
    ]
}

/// Read SSE frames off the merged tail until every expected frame kind has
/// arrived (or the deadline passes), grouped by event name.
async fn collect_tail_frames(
    body: &mut Body,
    expected_messages: usize,
    expected_runs: usize,
) -> BTreeMap<String, Vec<Value>> {
    use http_body_util::BodyExt as _;
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let messages = grouped.get("room_message").map_or(0, Vec::len);
        let runs = grouped.get("room_agent_run").map_or(0, Vec::len);
        if grouped.contains_key("room_access")
            && grouped.contains_key("room_read_cursor")
            && messages >= expected_messages
            && runs >= expected_runs
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "merged tail stalled; frames so far: {grouped:?}"
        );
        let frame = tokio::time::timeout(Duration::from_millis(500), body.frame()).await;
        let Ok(Some(Ok(frame))) = frame else {
            continue;
        };
        let text = String::from_utf8_lossy(&frame.into_data().unwrap_or_default()).to_string();
        let parsed = parse_sse_frame(&text);
        if parsed.event.is_empty() {
            // Keep-alive comment.
            continue;
        }
        let mut entry = serde_json::Map::new();
        if let Some(id) = parsed.id {
            entry.insert("id".into(), Value::String(id));
        }
        entry.insert("data".into(), parsed.data);
        grouped
            .entry(parsed.event)
            .or_default()
            .push(Value::Object(entry));
    }
    grouped
}

fn sort_by_numeric_id(frames: &mut [Value]) {
    frames.sort_by_key(|frame| {
        frame["id"]
            .as_str()
            .and_then(|id| id.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
}

/// Compare (or, under [`UPDATE_ENV`], rewrite) the committed fixtures.
fn settle_fixtures(fixtures: BTreeMap<&'static str, Value>) {
    let dir = std::path::Path::new(FIXTURE_DIR);
    let update = std::env::var_os(UPDATE_ENV).is_some_and(|v| !v.is_empty() && v != "0");
    if update {
        std::fs::create_dir_all(dir).expect("fixture dir");
        // Drop fixtures this run no longer produces so the directory never
        // keeps a file nothing generates.
        for entry in std::fs::read_dir(dir).expect("read fixture dir") {
            let path = entry.expect("dir entry").path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if path.extension().is_some_and(|e| e == "json")
                && !fixtures.contains_key(stem.as_str())
            {
                std::fs::remove_file(&path).expect("remove stale fixture");
            }
        }
    }
    let mut mismatches = Vec::new();
    for (name, value) in &fixtures {
        let path = dir.join(format!("{name}.json"));
        let mut rendered = serde_json::to_string_pretty(value).expect("fixture renders");
        rendered.push('\n');
        if update {
            std::fs::write(&path, &rendered).expect("write fixture");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == rendered => {}
            Ok(on_disk) => mismatches.push(format!(
                "{name}.json differs from what the daemon now serializes.\n--- committed\n{on_disk}\n--- live\n{rendered}"
            )),
            Err(e) => mismatches.push(format!("{name}.json is missing ({e})")),
        }
    }
    if !update {
        let mut stale = Vec::new();
        for entry in std::fs::read_dir(dir).expect("read fixture dir") {
            let path = entry.expect("dir entry").path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if path.extension().is_some_and(|e| e == "json")
                && !fixtures.contains_key(stem.as_str())
            {
                stale.push(stem);
            }
        }
        if !stale.is_empty() {
            mismatches.push(format!("fixtures nothing generates any more: {stale:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "room wire fixtures are out of date. Regenerate with `{UPDATE_ENV}=1 cargo test -p \
         ocean-daemon room_wire_fixtures`, then run `cargo test -p ocean-surface-ui wire_fixtures` \
         from apps/ocean-surface and commit both.\n\n{}",
        mismatches.join("\n\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn room_wire_fixtures_match_the_daemon_serializers() {
    let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
    let _restore = TestEnvRestore::capture(&["OCEAN_CONFIG_DIR", "OCEAN_MODEL", "OCEAN_YOLO"]);
    let tmp = tempfile::tempdir().unwrap();
    let state = fake_convene_state(&tmp);
    let key = RoomKey::new(ROOM);
    let room_uri = format!("/v1/rooms/persistent/{ROOM}");
    let operator = [(crate::room_operator::OPERATOR_HEADER, OPERATOR)];
    let mut fixtures: BTreeMap<&'static str, Value> = BTreeMap::new();

    // GET /v1/me — the daemon owner every human author derives from.
    fixtures.insert(
        "owner_identity",
        expect_status(get(&state, "/v1/me").await, StatusCode::OK, "GET /v1/me"),
    );

    // A second, quieter room so the list pages and carries a muted room. It is
    // stamped a day earlier straight through the store so the list's
    // `updated_at DESC, id ASC` order is settled regardless of how the clock
    // falls during the run.
    let policy = json!({ "on_mention": true, "on_thread_reply": true });
    with_rooms(&state, |store| {
        store.create(
            RoomKey::new(ANNEX),
            "Wire Annex",
            Some(RoomTriggerPolicy {
                on_mention: true,
                ..Default::default()
            }),
            fixed_instant() - chrono::Duration::days(1),
        )
    })
    .expect("annex creates");
    expect_status(
        send_json(
            &state,
            Method::PUT,
            &format!("/v1/rooms/persistent/{ANNEX}/prefs"),
            &[],
            json!({ "muted": true }),
        )
        .await,
        StatusCode::OK,
        "mute annex",
    );

    // The room under test: create, join as the owner, add an agent.
    fixtures.insert(
        "room_create",
        expect_status(
            send_json(
                &state,
                Method::POST,
                "/v1/rooms/persistent",
                &[],
                json!({ "key": ROOM, "name": "Wire Room", "trigger_policy": policy }),
            )
            .await,
            StatusCode::CREATED,
            "create room",
        ),
    );
    let (join_status, join) = send_json(
        &state,
        Method::POST,
        &format!("{room_uri}/participants"),
        &[],
        json!({ "kind": "human" }),
    )
    .await;
    assert!(
        join_status.is_success(),
        "join answered {join_status}: {join}"
    );
    fixtures.insert("room_join", join);
    with_rooms(&state, |store| {
        store
            .add_participant(
                &key,
                RoomParticipant {
                    id: "researcher".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Researcher".into(),
                },
                Utc::now(),
            )
            .expect("agent joins the roster");
    });

    // A root message, a human thread reply over the route, and an agent reply
    // written the way a convened turn writes it (with session attribution).
    let (post_status, post) = send_json(
        &state,
        Method::POST,
        &format!("{room_uri}/messages"),
        &[],
        json!({ "author_kind": "human", "body": "Hello room" }),
    )
    .await;
    assert!(
        post_status.is_success(),
        "post answered {post_status}: {post}"
    );
    let last_seq = |state: &AppState| {
        with_rooms(state, |store| store.transcript_page(&key, None, None))
            .expect("transcript reads")
            .messages
            .last()
            .map(|m| m.seq)
            .expect("transcript has rows")
    };
    let root_seq = last_seq(&state);
    let (reply_status, reply) = send_json(
        &state,
        Method::POST,
        &format!("{room_uri}/messages"),
        &[],
        json!({
            "author_kind": "human",
            "body": "Following up in the thread.",
            "thread_parent_seq": root_seq,
        }),
    )
    .await;
    assert!(
        reply_status.is_success(),
        "reply answered {reply_status}: {reply}"
    );
    let agent_reply_seq = with_rooms(&state, |store| {
        store.append_message_threaded(
            &key,
            "researcher",
            RoomParticipantKind::Agent,
            RoomMessageKind::Message,
            "On it.",
            Utc::now(),
            Some(root_seq),
            Some("session-researcher-1"),
        )
    })
    .expect("agent reply appends")
    .seq;

    // Read cursor: PATCH then GET share one body shape.
    fixtures.insert(
        "room_read_cursor_patch",
        expect_status(
            send_json(
                &state,
                Method::PATCH,
                &format!("{room_uri}/read-cursor"),
                &[],
                json!({ "read_seq": root_seq }),
            )
            .await,
            StatusCode::OK,
            "PATCH read-cursor",
        ),
    );
    fixtures.insert(
        "room_read_cursor_get",
        expect_status(
            get(&state, &format!("{room_uri}/read-cursor")).await,
            StatusCode::OK,
            "GET read-cursor",
        ),
    );

    // Attachments: the upload (metadata in the query string, raw bytes in the
    // body) and the list it lands in.
    let upload = expect_status(
        call(
            &state,
            Method::POST,
            &format!(
                "{room_uri}/attachments?filename=notes.txt&content_type=text/plain&uploader_id=john"
            ),
            &[],
            Some(b"wire fixture bytes\n".to_vec()),
        )
        .await,
        StatusCode::CREATED,
        "upload attachment",
    );
    let attachment_id = upload["attachment"]["id"]
        .as_str()
        .expect("upload returns the attachment id")
        .to_string();
    assert!(
        attachment_id.len() == 32 && attachment_id.chars().all(|c| c.is_ascii_hexdigit()),
        "attachment id {attachment_id:?} is not 32 hex characters"
    );
    fixtures.insert("room_attachment_upload", upload);
    fixtures.insert(
        "room_attachments_list",
        expect_status(
            get(&state, &format!("{room_uri}/attachments")).await,
            StatusCode::OK,
            "list attachments",
        ),
    );

    // Per-room agent settings (operator-gated write, open read).
    expect_status(
        send_json(
            &state,
            Method::PUT,
            &format!("{room_uri}/agents/researcher/settings"),
            &operator,
            json!({ "instructions": "Keep answers short.", "model": "fake-ok" }),
        )
        .await,
        StatusCode::OK,
        "PUT agent settings",
    );
    fixtures.insert(
        "room_agent_settings",
        expect_status(
            get(&state, &format!("{room_uri}/agents/researcher/settings")).await,
            StatusCode::OK,
            "GET agent settings",
        ),
    );

    // A durable agent binding, authorized at a fixed instant so the
    // projection's `authorized_at` needs no rewriting. This is also the LAST
    // write that bumps the room's `updated_at`, which pins it to the fixed
    // instant and makes the list cursor below (an `updated_at`/`id` keyset)
    // reproducible; keep any later mutation of `wire-room` off the clock.
    with_rooms(&state, |store| {
        store.authorize_room_agent(
            &key,
            AuthorizeAgentInput {
                agent_member_id: "researcher".into(),
                agent_package_id: "kingmaker/researcher".into(),
                agent_definition_digest: "sha256:0f1e2d3c4b5a69788796a5b4c3d2e1f0".into(),
                agent_definition_revision: Some("1.2.0".into()),
                display_name: "Researcher".into(),
                owner_member_id: "john".into(),
                authorized_by: "john".into(),
                activation_policy: ActivationPolicy::Mention,
                context_policy: ContextPolicy::RoomRecent,
                memory_scope: MemoryScope::Room,
                requested_capabilities: vec!["room.post".into(), "room.read".into()],
                room_capability_grants: vec!["room.read".into()],
                decision_id: "decision-wire-1".into(),
                request_digest: "digest-wire-1".into(),
            },
            fixed_instant(),
        )
    })
    .expect("binding authorizes");
    let bindings = expect_status(
        get(&state, &format!("{room_uri}/agents")).await,
        StatusCode::OK,
        "GET agent bindings",
    );
    assert_eq!(
        bindings["bindings"][0]["status"],
        json!(AgentBindingStatus::Active.as_str()),
        "the seeded binding is projected"
    );
    fixtures.insert("room_agent_bindings", bindings);

    // One work card per run state, replayed by the merged tail below.
    let runs = seeded_runs(root_seq, agent_reply_seq);
    with_rooms(&state, |store| {
        for run in &runs {
            store.put_room_agent_run(run).expect("run persists");
        }
    });

    // The open response, the 404 the Surface folds into a status line, and
    // both halves of the transcript page pair.
    let room_get = expect_status(get(&state, &room_uri).await, StatusCode::OK, "GET room");
    let transcript_len = room_get["transcript"]
        .as_array()
        .expect("transcript is an array")
        .len();
    fixtures.insert("room_get", room_get);
    fixtures.insert(
        "room_get_not_found",
        expect_status(
            get(&state, "/v1/rooms/persistent/wire-missing").await,
            StatusCode::NOT_FOUND,
            "GET missing room",
        ),
    );
    let first_page = expect_status(
        get(&state, &format!("{room_uri}/transcript?limit=2")).await,
        StatusCode::OK,
        "GET transcript page",
    );
    assert_eq!(first_page["has_more"], json!(true));
    let next_seq = first_page["next_seq"]
        .as_u64()
        .expect("a page with more rows carries next_seq");
    fixtures.insert("room_transcript_page", first_page);
    let last_page = expect_status(
        get(
            &state,
            &format!("{room_uri}/transcript?after_seq={next_seq}"),
        )
        .await,
        StatusCode::OK,
        "GET transcript last page",
    );
    assert_eq!(last_page["has_more"], json!(false));
    fixtures.insert("room_transcript_last_page", last_page);

    // The rooms list, one room per page: the first page names its cursor, the
    // last page carries the explicit `null` pollers rely on.
    let list_page = expect_status(
        get(&state, "/v1/rooms/persistent?limit=1").await,
        StatusCode::OK,
        "GET rooms page",
    );
    assert_eq!(list_page["has_more"], json!(true));
    assert_eq!(
        list_page["rooms"][0]["id"],
        json!(ROOM),
        "most recent room first"
    );
    let cursor = list_page["next_cursor"]
        .as_str()
        .expect("first page names its cursor")
        .to_string();
    assert!(
        cursor.contains("2026-10-23T12:00:00"),
        "the list cursor must carry the fixed instant the binding authorization stamped on \
         the room, not the run's clock: {cursor}"
    );
    fixtures.insert("rooms_list_page", list_page);
    let last_list_page = expect_status(
        get(
            &state,
            &format!(
                "/v1/rooms/persistent?limit=1&cursor={}",
                query_value(&cursor)
            ),
        )
        .await,
        StatusCode::OK,
        "GET rooms last page",
    );
    assert_eq!(last_list_page["has_more"], json!(false));
    assert_eq!(last_list_page["rooms"][0]["muted"], json!(true));
    fixtures.insert("rooms_list_last_page", last_list_page);

    // Every access state the Surface renders, read back through the open
    // response's `access` field.
    let mut access_by_state = serde_json::Map::new();
    access_by_state.insert("local".into(), fixtures["room_get"]["access"].clone());
    for (name, room, projection, credentialed) in seeded_access_rooms() {
        let seeded = RoomKey::new(room);
        seed_access(&state, &seeded, projection);
        if credentialed {
            with_rooms(&state, |store| {
                store.install_room_credential(
                    &seeded,
                    "wire-bearer-never-projected",
                    "22222222-2222-4222-8222-222222222222",
                )
            })
            .expect("credential installs");
        }
        let body = expect_status(
            get(&state, &format!("/v1/rooms/persistent/{room}")).await,
            StatusCode::OK,
            "GET seeded access room",
        );
        assert!(
            !body.to_string().contains("wire-bearer"),
            "a bearer token leaked into the room projection"
        );
        access_by_state.insert(name.into(), body["access"].clone());
    }
    fixtures.insert("room_access", Value::Object(access_by_state));

    // The merged SSE tail: bootstrap access and cursor frames, the id-bearing
    // message replay, and the recent work cards. Merge order between the
    // sub-streams is not part of the contract, so frames are grouped by event
    // and ordered within each group.
    let response = crate::room_routes()
        .with_state(state.clone())
        .oneshot(
            axum::http::Request::get(format!("{room_uri}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut grouped = collect_tail_frames(&mut body, transcript_len, runs.len()).await;
    drop(body);
    if let Some(messages) = grouped.get_mut("room_message") {
        sort_by_numeric_id(messages);
    }
    if let Some(run_frames) = grouped.get_mut("room_agent_run") {
        run_frames.sort_by(|a, b| {
            a["data"]["run_id"]
                .to_string()
                .cmp(&b["data"]["run_id"].to_string())
        });
    }
    assert_eq!(
        grouped.keys().cloned().collect::<Vec<_>>(),
        [
            "room_access",
            "room_agent_run",
            "room_message",
            "room_read_cursor"
        ],
        "the merged tail carries exactly the four room event names"
    );
    fixtures.insert(
        "room_events_tail",
        Value::Object(
            grouped
                .into_iter()
                .map(|(k, v)| (k, Value::Array(v)))
                .collect(),
        ),
    );

    // Invites: minted and redeemed against a disposable Bedrock, each on a
    // state whose supervisor talks to it (the fixture state's is disabled).
    let (base, _bedrock) = start_route_bedrock().await;
    let invite_state = with_route_supervisor(fake_convene_state(&tmp), &base);
    with_rooms(&invite_state, |store| {
        store.create(RoomKey::new("wire-invite"), "Wire Invite", None, Utc::now())
    })
    .expect("invite room");
    fixtures.insert(
        "room_invite",
        expect_status(
            send_json(
                &invite_state,
                Method::POST,
                "/v1/rooms/persistent/wire-invite/invites",
                &[],
                json!({ "recipient_name": "Peer", "ttl_minutes": 1440 }),
            )
            .await,
            StatusCode::CREATED,
            "POST invites",
        ),
    );
    invite_state.room_federation.shutdown().await;
    let redeem_state = with_route_supervisor(fake_convene_state(&tmp), &base);
    let redeemed = expect_status(
        send_json(
            &redeem_state,
            Method::POST,
            "/v1/rooms/persistent/invites/redeem",
            &[],
            json!({ "code": "wire-share-code" }),
        )
        .await,
        StatusCode::OK,
        "POST redeem",
    );
    redeem_state.room_federation.shutdown().await;
    assert!(
        !redeemed.to_string().contains("wire-share-code"),
        "the invite code leaked into the redeem response"
    );
    fixtures.insert("room_invite_redeem", redeemed);

    // Normalise the two run-specific values, then settle against the files.
    for value in fixtures.values_mut() {
        normalise_clock(value);
        let text = value
            .to_string()
            .replace(&attachment_id, FIXED_ATTACHMENT_ID);
        *value = serde_json::from_str(&text).expect("normalised fixture parses");
    }
    settle_fixtures(fixtures);
}
