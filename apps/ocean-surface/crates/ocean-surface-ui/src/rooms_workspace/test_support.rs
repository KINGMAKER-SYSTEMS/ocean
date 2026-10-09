//! Shared fixtures for the Rooms workspace unit tests.

use crate::rooms::{
    RoomAccessProjection, RoomAccessState, RoomMessage, RoomMessageKind, RoomParticipantKind,
};

pub(super) fn test_access(state: RoomAccessState) -> RoomAccessProjection {
    RoomAccessProjection {
        state,
        caller_member_id: None,
        last_confirmed_global_sequence: None,
        local_member_id: None,
        members: vec![],
        outbox: vec![],
    }
}

pub(super) fn test_msg(seq: u64, body: &str, thread_parent_seq: Option<u64>) -> RoomMessage {
    RoomMessage {
        seq,
        kind: RoomMessageKind::Message,
        author_id: "user".into(),
        author_kind: RoomParticipantKind::Human,
        body: body.into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        federated: None,
        thread_parent_seq,
    }
}
