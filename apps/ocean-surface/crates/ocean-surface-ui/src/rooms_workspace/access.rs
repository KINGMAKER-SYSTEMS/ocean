//! Access-projection gates: who may write, reply in threads, or share.

use crate::rooms::{FederatedActorType, FederatedRoomRole, RoomAccessProjection, RoomAccessState};

/// Whether writes (composer, join, leave) are permitted under this access
/// projection.
#[allow(dead_code)]
pub(super) fn access_allows_writes(access: Option<&RoomAccessProjection>) -> bool {
    matches!(
        access.map(|a| a.state),
        Some(RoomAccessState::Local) | Some(RoomAccessState::Live)
    )
}

/// Local rooms are always shareable by their daemon owner; a Live federated
/// room only by the member this daemon speaks as, and only if it owns the room.
pub(crate) fn access_allows_sharing(access: Option<&RoomAccessProjection>) -> bool {
    match access {
        Some(RoomAccessProjection {
            state: RoomAccessState::Local,
            ..
        }) => true,
        Some(RoomAccessProjection {
            state: RoomAccessState::Live,
            caller_member_id: Some(caller_member_id),
            members,
            ..
        }) => members.iter().any(|member| {
            member.member_id == *caller_member_id
                && member.actor_type == FederatedActorType::User
                && member.role_in_room == FederatedRoomRole::Owner
        }),
        _ => false,
    }
}

pub(super) fn access_allows_thread_writes(access: Option<&RoomAccessProjection>) -> bool {
    matches!(access.map(|a| a.state), Some(RoomAccessState::Local))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_access;
    use super::*;
    use crate::rooms::{FederatedActorType, FederatedRoomMemberProjection, MemberPresence};

    #[test]
    fn access_allows_writes_local() {
        assert!(access_allows_writes(Some(&test_access(
            RoomAccessState::Local
        ))));
    }

    #[test]
    fn access_allows_writes_live() {
        assert!(access_allows_writes(Some(&test_access(
            RoomAccessState::Live
        ))));
    }

    #[test]
    fn sharing_is_local_or_federated_owner_only() {
        let local = test_access(RoomAccessState::Local);
        assert!(access_allows_sharing(Some(&local)));

        let mut live = test_access(RoomAccessState::Live);
        live.local_member_id = Some("human-local".into());
        live.members.push(FederatedRoomMemberProjection {
            member_id: "bedrock-owner".into(),
            owner_member_id: None,
            actor_type: FederatedActorType::User,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Local human".into(),
            public_agent_descriptor: None,
            joined_at: "2026-08-16T00:00:00Z".into(),
            derived_presence: Some(MemberPresence::Live),
            local_binding_available: None,
        });
        live.caller_member_id = Some("bedrock-owner".into());
        assert!(!access_allows_sharing(Some(&live)));
        live.members[0].role_in_room = FederatedRoomRole::Owner;
        assert!(access_allows_sharing(Some(&live)));

        // Browser-local identities and an owned agent are not the caller.
        live.caller_member_id = Some("human-local".into());
        live.members[0].owner_member_id = Some("human-local".into());
        assert!(!access_allows_sharing(Some(&live)));
        live.caller_member_id = Some("bedrock-owner".into());
        live.members[0].actor_type = FederatedActorType::Agent;
        assert!(!access_allows_sharing(Some(&live)));
        live.members[0].actor_type = FederatedActorType::User;

        // Older daemons omit the additive caller projection and fail closed.
        live.caller_member_id = None;
        assert!(!access_allows_sharing(Some(&live)));
        live.caller_member_id = Some("bedrock-owner".into());
        for state in [
            RoomAccessState::Connecting,
            RoomAccessState::Recovering,
            RoomAccessState::Revoked,
        ] {
            live.state = state;
            assert!(!access_allows_sharing(Some(&live)));
        }
        assert!(!access_allows_sharing(None));
    }

    #[test]
    fn thread_writes_remain_local_until_federation_preserves_parent_sequence() {
        assert!(access_allows_thread_writes(Some(&test_access(
            RoomAccessState::Local
        ))));
        assert!(!access_allows_thread_writes(Some(&test_access(
            RoomAccessState::Live
        ))));
    }

    #[test]
    fn access_blocks_writes_federated_connecting() {
        assert!(!access_allows_writes(Some(&test_access(
            RoomAccessState::Connecting
        ))));
    }

    #[test]
    fn access_blocks_writes_none() {
        assert!(!access_allows_writes(None));
    }

    #[test]
    fn access_blocks_writes_revoked() {
        assert!(!access_allows_writes(Some(&test_access(
            RoomAccessState::Revoked
        ))));
    }

    #[test]
    fn installed_daemon_caller_wire_retains_owner_checks() {
        let owner = serde_json::json!({
            "state": "live", "self_member_id": "native-owner",
            "members": [{"member_id": "native-owner", "actor_type": "user", "role_in_room": "owner", "display_name": "Owner", "joined_at": "2026-10-05T00:00:00Z"}]
        });
        let access: RoomAccessProjection = serde_json::from_value(owner.clone()).unwrap();
        assert!(access_allows_sharing(Some(&access)));
        let mut member = owner.clone();
        member["members"][0]["role_in_room"] = serde_json::json!("member");
        let access: RoomAccessProjection = serde_json::from_value(member).unwrap();
        assert!(!access_allows_sharing(Some(&access)));
        let mut wrong_caller = owner.clone();
        wrong_caller["self_member_id"] = serde_json::json!("someone-else");
        let access: RoomAccessProjection = serde_json::from_value(wrong_caller).unwrap();
        assert!(!access_allows_sharing(Some(&access)));
        let mut duplicate = owner;
        duplicate["caller_member_id"] = serde_json::json!("someone-else");
        assert!(serde_json::from_value::<RoomAccessProjection>(duplicate).is_err());
        let json = serde_json::to_value(
            serde_json::from_str::<RoomAccessProjection>(
                r#"{"state":"live","self_member_id":"native-owner"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(json["caller_member_id"], "native-owner");
        assert!(json.get("self_member_id").is_none());
    }
}
