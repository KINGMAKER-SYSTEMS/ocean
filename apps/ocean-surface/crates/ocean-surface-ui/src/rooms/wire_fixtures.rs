//! The room wire contract, decoded from the daemon's own fixtures.
//!
//! `platform/ocean-os/crates/ocean-daemon/tests/fixtures/room-wire/*.json` is
//! written by the daemon test `room_wire_fixtures_match_the_daemon_serializers`
//! (`crates/ocean-daemon/src/persistent_rooms/tests/wire_fixtures.rs`) from the
//! real `room_routes()` router, and that test goes red whenever a daemon
//! serializer drifts from the committed files. This side feeds every fixture
//! through the PRODUCTION decoders in `rooms.rs` and `rooms_workspace/` and
//! asserts the fields the panel reads survive the trip, so a daemon change that
//! renames or retypes something the Surface depends on fails here, in the
//! same tree, before it ships.
//!
//! The vendored snapshots of the two-repo era (legacy ocean-surface#225/#233)
//! had no producer; these have one. Regenerate with
//! `OCEAN_UPDATE_WIRE_FIXTURES=1 cargo test -p ocean-daemon room_wire_fixtures`
//! from `platform/ocean-os`, then rerun this module. Every fixture file must
//! be claimed by exactly one of the readers below, so a new daemon fixture
//! forces a decision about what the Surface reads from it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;

/// The daemon-owned fixture directory, four levels up from this crate.
pub(crate) fn fixture_dir() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../platform/ocean-os/crates/ocean-daemon/tests/fixtures/room-wire"
    ))
    .to_path_buf()
}

/// One fixture's JSON text, exactly as the daemon wrote it.
pub(crate) fn fixture_text(name: &str) -> String {
    let path = fixture_dir().join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "room wire fixture {} is missing or unreadable ({e}); regenerate it with \
             `OCEAN_UPDATE_WIRE_FIXTURES=1 cargo test -p ocean-daemon room_wire_fixtures` \
             from platform/ocean-os",
            path.display()
        )
    })
}

fn fixture_value(name: &str) -> serde_json::Value {
    serde_json::from_str(&fixture_text(name))
        .unwrap_or_else(|e| panic!("{name}.json is not JSON: {e}"))
}

fn decode<T: serde::de::DeserializeOwned>(name: &str) -> T {
    serde_json::from_str(&fixture_text(name)).unwrap_or_else(|e| {
        panic!(
            "{name}.json no longer decodes as {}: {e}",
            std::any::type_name::<T>()
        )
    })
}

/// Fixtures the attachments module claims (see `rooms_workspace/attachments.rs`).
pub(crate) const ATTACHMENT_FIXTURES: &[&str] =
    &["room_attachment_upload", "room_attachments_list"];

/// Fixtures this module claims.
const ROOM_FIXTURES: &[&str] = &[
    "owner_identity",
    "room_access",
    "room_agent_bindings",
    "room_agent_settings",
    "room_create",
    "room_events_tail",
    "room_get",
    "room_get_not_found",
    "room_invite",
    "room_invite_redeem",
    "room_join",
    "room_read_cursor_get",
    "room_read_cursor_patch",
    "room_transcript_last_page",
    "room_transcript_page",
    "rooms_list_last_page",
    "rooms_list_page",
];

const ROOM: &str = "wire-room";

#[test]
fn every_daemon_fixture_is_claimed_by_a_surface_decoder() {
    let on_disk: BTreeSet<String> = std::fs::read_dir(fixture_dir())
        .unwrap_or_else(|e| panic!("fixture dir {}: {e}", fixture_dir().display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    let claimed: BTreeSet<String> = ROOM_FIXTURES
        .iter()
        .chain(ATTACHMENT_FIXTURES)
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        on_disk, claimed,
        "every room wire fixture must be read by exactly one Surface decoder test (and every \
         claimed fixture must exist)"
    );
}

#[test]
fn owner_identity_decodes() {
    let owner: OwnerIdentity = decode("owner_identity");
    assert_eq!(owner.participant_id, "john");
    assert_eq!(owner.display_name, "John");
}

#[test]
fn create_and_join_envelopes_decode_as_mutations() {
    let created: RoomMutateResponse = decode("room_create");
    assert!(created.ok);
    assert_eq!(created.error, None);
    let room = created.room.expect("create returns the room");
    assert_eq!(room.id, ROOM);
    assert_eq!(room.name, "Wire Room");
    assert!(room.participants.is_empty());
    assert!(!room.created_at.is_empty() && !room.updated_at.is_empty());
    let policy = room.trigger_policy.expect("create echoes the policy");
    assert!(policy.on_mention && policy.on_thread_reply && !policy.on_ci_failure);
    assert_eq!(room.workspace_root, None);

    let joined: RoomMutateResponse = decode("room_join");
    assert!(joined.ok);
    let room = joined.room.expect("join returns the room");
    assert_eq!(
        room.participants,
        vec![RoomParticipant {
            id: "john".into(),
            kind: RoomParticipantKind::Human,
            display_name: "John".into(),
        }]
    );
}

#[test]
fn room_open_response_decodes_with_transcript_and_access() {
    let open: RoomGetResponse = decode("room_get");
    assert!(open.ok);
    assert_eq!(open.error, None);
    let room = open.room.expect("open carries the room");
    assert_eq!(room.id, ROOM);
    assert_eq!(
        room.participants
            .iter()
            .map(|p| (p.id.as_str(), p.kind))
            .collect::<Vec<_>>(),
        vec![
            ("john", RoomParticipantKind::Human),
            ("researcher", RoomParticipantKind::Agent)
        ]
    );
    assert!(!open.has_more);
    assert_eq!(open.next_seq, None);
    assert_eq!(open.access.state, RoomAccessState::Local);
    assert_eq!(open.access.caller_member_id, None);
    assert!(open.access.members.is_empty() && open.access.outbox.is_empty());

    let transcript = open.transcript;
    assert_eq!(transcript.len(), 7);
    assert_eq!(
        transcript.iter().map(|m| m.seq).collect::<Vec<_>>(),
        (0..7).collect::<Vec<_>>()
    );
    // Join markers, the root, a human reply, an agent reply, two daemon notes.
    assert_eq!(transcript[0].kind, RoomMessageKind::ParticipantJoined);
    assert_eq!(transcript[0].author_kind, RoomParticipantKind::Human);
    assert_eq!(transcript[1].kind, RoomMessageKind::ParticipantJoined);
    assert_eq!(transcript[1].author_kind, RoomParticipantKind::Agent);
    assert_eq!(transcript[2].kind, RoomMessageKind::Message);
    assert_eq!(transcript[2].body, "Hello room");
    assert_eq!(transcript[2].thread_parent_seq, None);
    assert_eq!(transcript[3].thread_parent_seq, Some(2));
    assert_eq!(transcript[3].author_id, "john");
    assert_eq!(transcript[4].author_kind, RoomParticipantKind::Agent);
    assert_eq!(transcript[4].author_id, "researcher");
    assert_eq!(transcript[4].thread_parent_seq, Some(2));
    assert_eq!(transcript[5].kind, RoomMessageKind::System);
    assert_eq!(transcript[5].author_kind, RoomParticipantKind::System);
    assert!(transcript[5].body.contains("attached 'notes.txt'"));
    assert_eq!(transcript[6].kind, RoomMessageKind::System);
    for message in &transcript {
        assert!(
            !message.created_at.is_empty(),
            "seq {} has no created_at",
            message.seq
        );
        assert_eq!(
            message.federated, None,
            "local rows carry no federation meta"
        );
    }
}

#[test]
fn room_open_error_decodes() {
    let error: RoomErrorResponse = decode("room_get_not_found");
    assert_eq!(
        error.error.as_deref(),
        Some("no room with key 'wire-missing'")
    );
}

#[test]
fn transcript_pages_decode_with_the_cursor_pair() {
    let first: RoomTranscriptPageResponse = decode("room_transcript_page");
    assert!(first.ok);
    assert_eq!(first.error, None);
    assert_eq!(first.transcript.len(), 2);
    assert!(first.has_more);
    assert_eq!(first.next_seq, Some(1));
    assert_eq!(first.transcript.last().map(|m| m.seq), first.next_seq);

    let last: RoomTranscriptPageResponse = decode("room_transcript_last_page");
    assert!(last.ok);
    assert_eq!(last.transcript.first().map(|m| m.seq), Some(2));
    assert_eq!(last.transcript.len(), 5);
    assert!(!last.has_more);
    assert_eq!(last.next_seq, None);
}

#[test]
fn rooms_list_pages_decode_with_read_states_and_mute() {
    let first: RoomsListResponse = decode("rooms_list_page");
    assert!(first.ok);
    assert_eq!(first.error, None);
    assert!(first.has_more);
    assert!(first.next_cursor.as_deref().is_some_and(|c| !c.is_empty()));
    assert_eq!(first.rooms.len(), 1);
    assert_eq!(first.rooms[0].id, ROOM);
    assert!(!first.rooms[0].muted);
    assert_eq!(first.rooms[0].participants.len(), 2);
    assert_eq!(
        first.read_states,
        vec![RoomReadStateWire {
            room_id: ROOM.into(),
            latest_seq: Some("6".into()),
            read_seq: Some("2".into()),
        }]
    );

    let last: RoomsListResponse = decode("rooms_list_last_page");
    assert!(last.ok);
    assert!(!last.has_more);
    assert_eq!(last.next_cursor, None);
    assert_eq!(last.rooms.len(), 1);
    assert_eq!(last.rooms[0].id, "wire-annex");
    assert!(last.rooms[0].muted);
    assert_eq!(
        last.read_states,
        vec![RoomReadStateWire {
            room_id: "wire-annex".into(),
            latest_seq: None,
            read_seq: None,
        }]
    );
}

#[test]
fn read_cursor_envelopes_decode_to_the_local_projection() {
    for name in ["room_read_cursor_patch", "room_read_cursor_get"] {
        let envelope: ReadCursorPatchEnvelope = decode(name);
        assert!(envelope.ok, "{name}");
        assert_eq!(envelope.cursor.room_id, ROOM, "{name}");
        assert_eq!(envelope.cursor.read_seq.as_deref(), Some("2"), "{name}");
        let projection = parse_patch_read_cursor_response(ROOM, envelope.cursor)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            projection,
            RoomReadCursorProjection {
                read_seq: Some(2),
                mirrored_upstream_read_seq: None,
            },
            "{name}"
        );
    }
}

#[test]
fn agent_settings_envelope_decodes() {
    let envelope: RoomAgentSettingsEnvelope = decode("room_agent_settings");
    assert_eq!(
        envelope.settings,
        RoomAgentSettings {
            instructions: Some("Keep answers short.".into()),
            model: Some("fake-ok".into()),
        }
    );
}

#[test]
fn agent_bindings_projection_decodes() {
    let bindings = parse_agent_bindings(&fixture_value("room_agent_bindings"))
        .expect("the bindings envelope is `ok` with a `bindings` array");
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.agent_member_id, "researcher");
    assert_eq!(binding.agent_package_id, "kingmaker/researcher");
    assert_eq!(
        binding.agent_definition_digest,
        "sha256:0f1e2d3c4b5a69788796a5b4c3d2e1f0"
    );
    assert_eq!(binding.display_name, "Researcher");
    assert_eq!(binding.owner_member_id, "john");
    assert_eq!(binding.status, "active");
    // The daemon serializes the generation as a decimal string.
    assert_eq!(binding.generation, "1");
    assert!(!binding.owner_eligible);
}

#[test]
fn invite_and_redeem_responses_decode() {
    let invite: RoomInvite = decode("room_invite");
    assert_eq!(invite.code, "route-share-code");
    assert_eq!(invite.expires_at, "2026-07-18T00:00:00Z");
    assert_eq!(invite.room_key, "wire-invite");
    assert_eq!(invite.room_name, "Wire Invite");

    let redeemed: RedeemInviteResponse = decode("room_invite_redeem");
    assert_eq!(redeemed.room_key, "route-redeem");
    // The redeem body is the access projection with `room_key` beside it.
    let access: RoomAccessProjection = decode("room_invite_redeem");
    assert_eq!(access.state, RoomAccessState::Connecting);
    assert_eq!(
        access.caller_member_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(access.local_member_id, access.caller_member_id);
}

#[test]
fn every_access_state_decodes() {
    let by_state = fixture_value("room_access");
    let by_state = by_state.as_object().expect("room_access is keyed by state");
    let expected = [
        ("local", RoomAccessState::Local),
        ("connecting", RoomAccessState::Connecting),
        ("live", RoomAccessState::Live),
        ("recovering", RoomAccessState::Recovering),
        ("revoked", RoomAccessState::Revoked),
    ];
    assert_eq!(
        by_state.keys().cloned().collect::<BTreeSet<_>>(),
        expected
            .iter()
            .map(|(k, _)| k.to_string())
            .collect::<BTreeSet<_>>()
    );
    for (name, state) in expected {
        let access: RoomAccessProjection = serde_json::from_value(by_state[name].clone())
            .unwrap_or_else(|e| panic!("access {name}: {e}"));
        assert_eq!(access.state, state, "{name}");
    }

    let local: RoomAccessProjection = serde_json::from_value(by_state["local"].clone()).unwrap();
    assert_eq!(local.caller_member_id, None);
    assert_eq!(local.local_member_id, None);
    assert_eq!(local.last_confirmed_global_sequence, None);

    let live: RoomAccessProjection = serde_json::from_value(by_state["live"].clone()).unwrap();
    assert_eq!(live.last_confirmed_global_sequence, Some(42));
    assert_eq!(
        live.local_member_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(live.members.len(), 3);
    let owner = &live.members[0];
    assert_eq!(owner.actor_type, FederatedActorType::User);
    assert_eq!(owner.role_in_room, FederatedRoomRole::Owner);
    assert_eq!(owner.derived_presence, Some(MemberPresence::Live));
    assert_eq!(owner.public_agent_descriptor, None);
    assert_eq!(
        live.members[1].derived_presence,
        Some(MemberPresence::Unavailable)
    );
    let agent = &live.members[2];
    assert_eq!(agent.actor_type, FederatedActorType::Agent);
    assert_eq!(agent.role_in_room, FederatedRoomRole::Member);
    assert_eq!(
        agent.owner_member_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(agent.local_binding_available, Some(true));
    assert_eq!(agent.joined_at, "2026-07-17T00:00:00Z");
    let descriptor = agent
        .public_agent_descriptor
        .as_ref()
        .expect("the agent member carries its descriptor");
    assert_eq!(descriptor.display_name, "Researcher");
    assert_eq!(
        descriptor.description.as_deref(),
        Some("Reads the room and reports back.")
    );
    assert_eq!(descriptor.model_alias.as_deref(), Some("fable"));
    assert_eq!(descriptor.skills_count, 3);
    assert_eq!(descriptor.subagent_names, vec!["scout".to_string()]);
    assert_eq!(
        live.outbox
            .iter()
            .map(|item| (item.client_event_id.as_str(), item.state))
            .collect::<Vec<_>>(),
        vec![
            ("evt-pending", OutboxItemState::Pending),
            ("evt-failed", OutboxItemState::Failed)
        ]
    );
    assert_eq!(live.outbox[0].event_type, "chat.message");
    assert_eq!(live.outbox[0].payload["text"], "pending on the wire");
    assert_eq!(
        live.outbox[0].mention_member_ids,
        vec!["33333333-3333-4333-8333-000000000000".to_string()]
    );

    let recovering: RoomAccessProjection =
        serde_json::from_value(by_state["recovering"].clone()).unwrap();
    assert_eq!(recovering.last_confirmed_global_sequence, Some(7));
    let revoked: RoomAccessProjection =
        serde_json::from_value(by_state["revoked"].clone()).unwrap();
    assert_eq!(revoked.local_member_id, None);
}

/// Feed one captured SSE frame through the production tail decoder.
fn decode_frame(event: &str, frame: &serde_json::Value) -> RoomTailFrame {
    let data = serde_json::to_string(&frame["data"]).expect("frame data re-serializes");
    decode_room_tail_frame(event, &data, ROOM)
        .unwrap_or_else(|| panic!("{event} frame no longer decodes: {data}"))
}

#[test]
fn merged_tail_frames_decode_through_the_production_tail_decoder() {
    let tail = fixture_value("room_events_tail");
    let tail = tail.as_object().expect("tail is keyed by event name");
    assert_eq!(
        tail.keys().cloned().collect::<Vec<_>>(),
        [
            "room_access",
            "room_agent_run",
            "room_message",
            "room_read_cursor"
        ],
        "the four event names the tail decoder matches on"
    );
    let frames = |event: &str| tail[event].as_array().expect("frames are an array");

    // Bootstrap access frame: Local, no id.
    let access = frames("room_access");
    assert_eq!(access.len(), 1);
    assert_eq!(access[0].get("id"), None);
    match decode_frame("room_access", &access[0]) {
        RoomTailFrame::Access(projection) => assert_eq!(projection.state, RoomAccessState::Local),
        other => panic!("room_access decoded as {other:?}"),
    }

    // Bootstrap cursor frame: decimal-string seq, mirrored projection, no id.
    let cursor = frames("room_read_cursor");
    assert_eq!(cursor.len(), 1);
    assert_eq!(cursor[0].get("id"), None);
    assert_eq!(cursor[0]["data"]["read_seq"], "2");
    match decode_frame("room_read_cursor", &cursor[0]) {
        RoomTailFrame::ReadCursor(projection) => assert_eq!(
            projection,
            RoomReadCursorProjection {
                read_seq: None,
                mirrored_upstream_read_seq: Some(2),
            }
        ),
        other => panic!("room_read_cursor decoded as {other:?}"),
    }
    // A cursor frame naming another room is dropped by construction.
    let foreign = serde_json::to_string(&cursor[0]["data"]).unwrap();
    assert!(decode_room_tail_frame("room_read_cursor", &foreign, "other-room").is_none());

    // Message replay: `id` is the row's seq, the payload is the transcript row.
    let messages = frames("room_message");
    assert_eq!(messages.len(), 7);
    for (index, frame) in messages.iter().enumerate() {
        assert_eq!(frame["id"], index.to_string(), "message frame {index} id");
        match decode_frame("room_message", frame) {
            RoomTailFrame::Message(message) => {
                assert_eq!(message.seq as usize, index);
                assert!(!message.created_at.is_empty());
            }
            other => panic!("room_message decoded as {other:?}"),
        }
    }
    let RoomTailFrame::Message(reply) = decode_frame("room_message", &messages[4]) else {
        unreachable!()
    };
    assert_eq!(reply.author_kind, RoomParticipantKind::Agent);
    assert_eq!(reply.thread_parent_seq, Some(2));

    // Work cards: one per run state; none falls into the `Unknown` arm.
    let runs = frames("room_agent_run");
    assert_eq!(runs.len(), 8);
    let mut seen = Vec::new();
    for frame in runs {
        assert_eq!(frame.get("id"), None, "run frames carry no Last-Event-ID");
        let RoomTailFrame::Run(run) = decode_frame("room_agent_run", frame) else {
            panic!("room_agent_run decoded as another frame kind");
        };
        assert_eq!(run.room_id, ROOM);
        assert_eq!(run.agent_id, "researcher");
        assert_eq!(run.session_id, "session-researcher-1");
        assert_eq!((run.trigger_seq, run.thread_root_seq), (2, 2));
        assert!(!run.started_at.is_empty() && !run.updated_at.is_empty());
        assert_ne!(run.state, RoomAgentRunState::Unknown, "{}", run.run_id);
        match &run.state {
            RoomAgentRunState::Queued => assert_eq!(run.request_id, None, "legacy run"),
            RoomAgentRunState::RunningTool { label } => {
                assert_eq!(label, "Reading notes.txt");
                assert_eq!(run.tool_count, 2);
            }
            RoomAgentRunState::AwaitingPermission => assert_eq!(
                run.pending_permission,
                Some(RoomRunPermission {
                    permission_id: "perm-1".into(),
                    tool: "bash".into(),
                    tool_label: "Run the test suite".into(),
                })
            ),
            RoomAgentRunState::Done => {
                assert_eq!(run.summary.as_deref(), Some("Fixed the flaky test."));
                assert_eq!(run.files_changed, vec!["crates/ocean-daemon/src/main.rs"]);
                assert_eq!(run.reply_seq, Some(4));
                assert_eq!(run.tool_count, 5);
            }
            RoomAgentRunState::Failed { reason } => assert_eq!(reason, "provider unavailable"),
            _ => {}
        }
        if !matches!(run.state, RoomAgentRunState::Queued) {
            assert_eq!(
                run.request_id.as_deref(),
                Some(format!("req-{}", run.run_id).as_str())
            );
        }
        seen.push(run.state.clone());
    }
    assert_eq!(
        seen,
        vec![
            RoomAgentRunState::Queued,
            RoomAgentRunState::Thinking,
            RoomAgentRunState::RunningTool {
                label: "Reading notes.txt".into()
            },
            RoomAgentRunState::AwaitingPermission,
            RoomAgentRunState::AwaitingReply,
            RoomAgentRunState::Done,
            RoomAgentRunState::Failed {
                reason: "provider unavailable".into()
            },
            RoomAgentRunState::Cancelled,
        ]
    );
    // A run naming another room is dropped by construction.
    let foreign = serde_json::to_string(&runs[0]["data"]).unwrap();
    assert!(decode_room_tail_frame("room_agent_run", &foreign, "other-room").is_none());
}
