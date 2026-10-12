//! Pure presentation helpers: wire clock labels, avatar hues, kind labels,
//! day keys, and agent descriptor lines.

use crate::rooms::{RoomAccessProjection, RoomParticipantKind};

/// The client's current UTC day key (`YYYY-MM-DD`), matching the daemon's
/// ISO-8601 UTC timestamps, for humanizing day separators.
pub(super) fn today_day_key() -> String {
    js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
        .chars()
        .take(10)
        .collect()
}

/// Row timestamp: show only the canonical wire clock (HH:MM) for RFC3339
/// timestamps while preserving the full wire value for machine-readable and
/// accessible render paths. Accepts only ASCII canonical structure at the
/// byte positions we actually rely on, never panics on Unicode/invalid input,
/// and returns the original string unchanged when the wire value is not the
/// expected RFC3339 shape.
pub(super) fn canonical_wire_clock_time(full: &str) -> String {
    let bytes = full.as_bytes();
    let is_digit = |idx: usize| bytes.get(idx).is_some_and(|b| b.is_ascii_digit());

    if bytes.len() < 16
        || !full.is_ascii()
        || !is_digit(0)
        || !is_digit(1)
        || !is_digit(2)
        || !is_digit(3)
        || bytes[4] != b'-'
        || !is_digit(5)
        || !is_digit(6)
        || bytes[7] != b'-'
        || !is_digit(8)
        || !is_digit(9)
        || bytes[10] != b'T'
        || !is_digit(11)
        || !is_digit(12)
        || bytes[13] != b':'
        || !is_digit(14)
        || !is_digit(15)
    {
        return full.to_string();
    }

    full[11..16].to_string()
}
pub(super) fn avatar_identity_class(author_id: &str) -> &'static str {
    const HUES: [&str; 5] = [
        "rooms-workspace__msg-avatar--hue0",
        "rooms-workspace__msg-avatar--hue1",
        "rooms-workspace__msg-avatar--hue2",
        "rooms-workspace__msg-avatar--hue3",
        "rooms-workspace__msg-avatar--hue4",
    ];
    let h = author_id.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    HUES[h % HUES.len()]
}

/// Short human label for a participant kind, shown as the suggestion badge.
pub(super) fn participant_kind_label(kind: RoomParticipantKind) -> &'static str {
    match kind {
        RoomParticipantKind::Human => "human",
        RoomParticipantKind::Agent => "agent",
        RoomParticipantKind::Bot => "bot",
        RoomParticipantKind::Tool => "tool",
        RoomParticipantKind::System => "system",
    }
}

/// One-line identity summary for an agent member, from the federated roster
/// descriptor when available: model alias and description. `None` for humans
/// and for agents without a published descriptor — never fabricated.
pub(super) fn agent_descriptor_line(
    access: Option<&RoomAccessProjection>,
    member_id: &str,
) -> Option<String> {
    let member = access?.members.iter().find(|m| m.member_id == member_id)?;
    let descriptor = member.public_agent_descriptor.as_ref()?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(alias) = descriptor.model_alias.as_ref().filter(|a| !a.is_empty()) {
        parts.push(alias.clone());
    }
    if let Some(desc) = descriptor.description.as_ref().filter(|d| !d.is_empty()) {
        parts.push(desc.clone());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" \u{b7} "))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_access;
    use super::*;
    use crate::rooms::{
        FederatedActorType, FederatedRoomMemberProjection, FederatedRoomRole, RoomAccessState,
    };

    #[test]
    fn canonical_wire_clock_time_extracts_hhmm_from_rfc3339_z() {
        assert_eq!(canonical_wire_clock_time("2026-07-25T03:43:12Z"), "03:43");
    }

    #[test]
    fn canonical_wire_clock_time_extracts_hhmm_from_rfc3339_fractional() {
        assert_eq!(
            canonical_wire_clock_time("2026-07-25T03:43:12.987Z"),
            "03:43"
        );
    }

    #[test]
    fn canonical_wire_clock_time_extracts_hhmm_from_rfc3339_offset() {
        assert_eq!(
            canonical_wire_clock_time("2026-07-25T03:43:12+07:00"),
            "03:43"
        );
    }

    #[test]
    fn canonical_wire_clock_time_passthrough_short_string() {
        assert_eq!(canonical_wire_clock_time("abc"), "abc");
    }

    #[test]
    fn canonical_wire_clock_time_passthrough_noncanonical_separator() {
        assert_eq!(
            canonical_wire_clock_time("2026-07-25 03:43:12Z"),
            "2026-07-25 03:43:12Z"
        );
    }

    #[test]
    fn canonical_wire_clock_time_passthrough_unicode_without_panic() {
        assert_eq!(
            canonical_wire_clock_time("２０２６-07-25T03:43:12Z"),
            "２０２６-07-25T03:43:12Z"
        );
    }

    #[test]
    fn room_timestamp_markup_preserves_full_wire_datetime_and_visible_clock() {
        let ts = "2026-07-25T03:43:12.987+07:00";
        let clock = canonical_wire_clock_time(ts);
        let markup = format!(
            "<time class=\"rooms-workspace__msg-time\" datetime=\"{ts}\" aria-label=\"{ts}\" title=\"{ts}\">{clock}</time>"
        );
        assert!(markup.contains("<time"));
        assert!(markup.contains("datetime=\"2026-07-25T03:43:12.987+07:00\""));
        assert!(markup.contains("aria-label=\"2026-07-25T03:43:12.987+07:00\""));
        assert!(markup.contains("title=\"2026-07-25T03:43:12.987+07:00\""));
        assert!(markup.ends_with(">03:43</time>"));
    }

    #[test]
    fn agent_descriptor_line_reads_projected_member_descriptor() {
        use crate::rooms::PublicAgentDescriptor;
        let mut access = test_access(RoomAccessState::Live);
        access.members = vec![FederatedRoomMemberProjection {
            member_id: "flux".into(),
            owner_member_id: None,
            actor_type: FederatedActorType::Agent,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Flux".into(),
            public_agent_descriptor: Some(PublicAgentDescriptor {
                display_name: "Flux".into(),
                description: Some("Rapid implementation".into()),
                model_alias: Some("sonnet".into()),
                skills_count: 0,
                subagent_names: vec![],
            }),
            joined_at: String::new(),
            derived_presence: None,
            local_binding_available: Some(false),
        }];
        assert_eq!(
            agent_descriptor_line(Some(&access), "flux"),
            Some("sonnet \u{b7} Rapid implementation".to_string())
        );
    }

    #[test]
    fn agent_descriptor_line_is_truthful_only() {
        use crate::rooms::PublicAgentDescriptor;
        let mut access = test_access(RoomAccessState::Local);
        access.members = vec![FederatedRoomMemberProjection {
            member_id: "flux".into(),
            owner_member_id: None,
            actor_type: FederatedActorType::Agent,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Flux".into(),
            public_agent_descriptor: Some(PublicAgentDescriptor {
                display_name: "Flux".into(),
                description: Some("Rapid implementation".into()),
                model_alias: Some("sonnet".into()),
                skills_count: 0,
                subagent_names: vec![],
            }),
            joined_at: String::new(),
            derived_presence: None,
            local_binding_available: None,
        }];
        assert_eq!(
            agent_descriptor_line(Some(&access), "flux"),
            Some("sonnet \u{b7} Rapid implementation".to_string())
        );
        // Unknown member / no descriptor / no access — never fabricated.
        assert_eq!(agent_descriptor_line(Some(&access), "ghost"), None);
        assert_eq!(agent_descriptor_line(None, "flux"), None);
    }

    #[test]
    fn canonical_wire_clock_time_strips_redundant_date_for_rfc3339() {
        assert_eq!(canonical_wire_clock_time("2026-06-05T12:34:56Z"), "12:34");
        // Non-canonical inputs fall back to the full string — never lie.
        assert_eq!(canonical_wire_clock_time(""), "");
        assert_eq!(canonical_wire_clock_time("12:34"), "12:34");
        assert_eq!(canonical_wire_clock_time("2026-06-05T12"), "2026-06-05T12");
        assert_eq!(
            canonical_wire_clock_time("2026-06-05 12:34"),
            "2026-06-05 12:34"
        );
    }

    #[test]
    fn avatar_identity_is_deterministic_and_bounded() {
        let a = avatar_identity_class("ada");
        assert_eq!(a, avatar_identity_class("ada"), "same id, same hue");
        assert!(a.starts_with("rooms-workspace__msg-avatar--hue"));
        // Different ids may collide (5 hues) but must all stay in range.
        for id in ["ada", "grace", "linus", "smaths", "ocean-agent-7", ""] {
            let c = avatar_identity_class(id);
            assert!(
                (0..5).any(|n| c == format!("rooms-workspace__msg-avatar--hue{n}")),
                "out of palette: {c}"
            );
        }
    }

    #[test]
    fn avatar_identity_distributes_across_hues() {
        use std::collections::HashSet;
        let hues: HashSet<_> = (0..50)
            .map(|n| avatar_identity_class(&format!("agent-{n}")))
            .collect();
        assert!(hues.len() >= 3, "degenerate distribution: {hues:?}");
    }
}
