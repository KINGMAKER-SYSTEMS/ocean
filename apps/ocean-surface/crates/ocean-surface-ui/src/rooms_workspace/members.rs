//! Right rail member roster for Local and federated rooms.

use leptos::prelude::*;

use super::format::{avatar_identity_class, participant_kind_label};
use crate::rooms::{
    AgentSummary, FederatedActorType, FederatedRoomMemberProjection, FederatedRoomRole,
    MemberPresence, RoomAccessState, RoomParticipant, Rooms,
};

pub(super) fn roster_presence_count(members: &[FederatedRoomMemberProjection]) -> usize {
    members
        .iter()
        .filter(|member| {
            matches!(member.actor_type, FederatedActorType::User)
                && matches!(member.derived_presence, Some(MemberPresence::Live))
        })
        .count()
}

/// Member roster for the open room (right rail, when no thread is open).
/// Local rooms list daemon participants plus the add-agent picker; federated
/// rooms list the projected members with role, presence, and descriptor.
#[component]
pub(super) fn MembersPanel(rooms: Rooms) -> impl IntoView {
    move || {
        match rooms.access.get() {
            None => view! {
                <div class="rooms-workspace__right-empty">
                    "Open a room to see members."
                </div>
            }
            .into_any(),
            Some(ref access) if access.state == RoomAccessState::Local => {
                let participants = rooms
                    .open_room
                    .get()
                    .map(|r| r.participants)
                    .unwrap_or_default();
                let show_add_agent = RwSignal::new(false);
                view! {
                    {if participants.is_empty() {
                        view! {
                            <div class="rooms-workspace__right-empty">
                                "No members yet."
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div
                                class="rooms-workspace__member-list"
                                role="list"
                                aria-label="Room members"
                            >
                            <For
                                each=move || rooms.open_room.get()
                                    .map(|r| r.participants)
                                    .unwrap_or_default()
                                key=|p: &RoomParticipant| p.id.clone()
                                children=move |p: RoomParticipant| {
                                    view! {
                                        <div
                                            class="rooms-workspace__member"
                                            role="listitem"
                                        >
                                            <div class=format!(
                                                "rooms-workspace__member-avatar {}",
                                                avatar_identity_class(&p.id)
                                            )>
                                                {crate::rooms::name_initials(&p.display_name)}
                                            </div>
                                            <span class="rooms-workspace__member-name">
                                                {p.display_name.clone()}
                                            </span>
                                            <span class="rooms-workspace__member-kind">
                                                {participant_kind_label(p.kind)}
                                            </span>
                                        </div>
                                    }
                                }
                            />
                            </div>
                        }.into_any()
                    }}

                    <button
                        class="rooms-workspace__addagent"
                        type="button"
                        title="Add an agent participant"
                        aria-controls="rooms-workspace-agent-picker"
                        aria-expanded=move || show_add_agent.get().to_string()
                        on:click=move |_| show_add_agent.update(|v: &mut bool| *v = !*v)
                    >
                        "+ agent"
                    </button>
                    {move || {
                        if show_add_agent.get() {
                            view! {
                                <div
                                    id="rooms-workspace-agent-picker"
                                    class="rooms-workspace__addagent-picker"
                                >
                                    <select
                                        class="rooms-workspace__addagent-select"
                                        aria-label="Choose an agent to add"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            if !val.is_empty() {
                                                rooms.add_agent(val);
                                                show_add_agent.set(false);
                                            }
                                        }
                                    >
                                        <option value="" selected=true>
                                            "-- pick an agent --"
                                        </option>
                                        <For
                                            each=move || {
                                                let present: Vec<String> = rooms
                                                    .open_room
                                                    .get()
                                                    .map(|r| r.participants.into_iter().map(|p| p.id).collect())
                                                    .unwrap_or_default();
                                                rooms.available_agents.get().into_iter().filter(|agent| !present.contains(&agent.name)).collect::<Vec<AgentSummary>>()
                                            }
                                            key=|agent: &AgentSummary| agent.name.clone()
                                            children=move |agent: AgentSummary| {
                                                // Unresolvable folders stay visible but not addable.
                                                let disabled = agent.error.is_some();
                                                let v = agent.name.clone();
                                                view! {
                                                    <option value=v disabled=disabled>{agent.name}</option>
                                                }
                                            }
                                        />
                                    </select>
                                </div>
                            }.into_any()
                        } else {
                            ().into_any()
                        }
                    }}
                }.into_any()
            }
            Some(ref access)
                if matches!(
                    access.state,
                    RoomAccessState::Connecting
                        | RoomAccessState::Live
                        | RoomAccessState::Recovering
                        | RoomAccessState::Revoked
                ) =>
            {
                if access.members.is_empty() {
                    view! {
                        <div class="rooms-workspace__right-empty">
                            "No members visible."
                        </div>
                    }
                    .into_any()
                } else {
                    let members = access.members.clone();
                    let members_for_label = members.clone();
                    view! {
                        // Roster is a real list: give AT an
                        // item count + boundaries instead of
                        // an undifferentiated div run.
                        <div
                            class="rooms-workspace__member-list"
                            role="list"
                            aria-label=move || {
                                let count = roster_presence_count(&members_for_label);
                                if count == 0 {
                                    "Room members".to_string()
                                } else {
                                    format!("Room members, {count} humans live")
                                }
                            }
                        >
                        <For
                            each=move || members.clone()
                            key=|m: &FederatedRoomMemberProjection| m.member_id.clone()
                            children=move |member: FederatedRoomMemberProjection| {
                                let role_label = match member.role_in_room {
                                    FederatedRoomRole::Owner => "owner",
                                    FederatedRoomRole::Member => "member",
                                };
                                let actor_label = match member.actor_type {
                                    FederatedActorType::User => "user",
                                    FederatedActorType::Agent => "agent",
                                };
                                let presence = member.derived_presence;
                                let presence_label = match presence {
                                    Some(MemberPresence::Live) => "Live",
                                    Some(MemberPresence::Unavailable) => "Unavailable",
                                    None => "",
                                };
                                let local_agent = matches!(member.actor_type, FederatedActorType::Agent)
                                    && member.local_binding_available == Some(true);
                                let remote_agent = matches!(member.actor_type, FederatedActorType::Agent)
                                    && member.local_binding_available == Some(false);
                                let desc_line = member
                                    .public_agent_descriptor
                                    .as_ref()
                                    .and_then(|descriptor| {
                                        let mut parts = Vec::new();
                                        if let Some(alias) = descriptor
                                            .model_alias
                                            .as_ref()
                                            .filter(|alias| !alias.is_empty())
                                        {
                                            parts.push(alias.clone());
                                        }
                                        if let Some(description) = descriptor
                                            .description
                                            .as_ref()
                                            .filter(|description| !description.is_empty())
                                        {
                                            parts.push(description.clone());
                                        }
                                        (!parts.is_empty())
                                            .then(|| parts.join(" \u{b7} "))
                                    });
                                let desc_title = member.public_agent_descriptor.as_ref()
                                    .and_then(|d| d.description.clone())
                                    .unwrap_or_default();
                                view! {
                                    <div class="rooms-workspace__member"
                                        role="listitem"
                                        class:rooms-workspace__member--local-agent=local_agent
                                        class:rooms-workspace__member--remote-agent=remote_agent
                                        title=desc_title
                                    >
                                        <div class=format!(
                                            "rooms-workspace__member-avatar {}",
                                            avatar_identity_class(&member.member_id)
                                        )>
                                            {crate::rooms::name_initials(&member.display_name)}
                                        </div>
                                        <span class="rooms-workspace__member-name">
                                            {member.display_name.clone()}
                                        </span>
                                        {desc_line.map(|desc| view! {
                                            <span class="rooms-workspace__member-desc">{desc}</span>
                                        })}
                                        <span class="rooms-workspace__member-kind">
                                            {actor_label}
                                        </span>
                                        <span class="rooms-workspace__member-role">
                                            {role_label}
                                        </span>
                                        {if presence.is_some() {
                                            view! {
                                                <span
                                                    class="rooms-workspace__member-presence"
                                                    class:rooms-workspace__member-presence--live=move || {
                                                        presence == Some(MemberPresence::Live)
                                                    }
                                                    class:rooms-workspace__member-presence--unavailable=move || {
                                                        presence == Some(MemberPresence::Unavailable)
                                                    }
                                                    role="img"
                                                    aria-label=presence_label
                                                ></span>
                                            }.into_any()
                                        } else {
                                            ().into_any()
                                        }}
                                    </div>
                                }
                            }
                        />
                        </div>
                    }.into_any()
                }
            }
            _ => view! {
                <div class="rooms-workspace__right-empty">
                    "Members unavailable."
                </div>
            }
            .into_any(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::FederatedRoomRole;

    #[test]
    fn roster_presence_counts_humans_only() {
        let members = vec![
            FederatedRoomMemberProjection {
                member_id: "user-live-1".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::User,
                role_in_room: FederatedRoomRole::Member,
                display_name: "A".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: Some(MemberPresence::Live),
                local_binding_available: Some(true),
            },
            FederatedRoomMemberProjection {
                member_id: "user-live-2".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::User,
                role_in_room: FederatedRoomRole::Owner,
                display_name: "B".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: Some(MemberPresence::Live),
                local_binding_available: Some(true),
            },
            FederatedRoomMemberProjection {
                member_id: "user-away".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::User,
                role_in_room: FederatedRoomRole::Member,
                display_name: "C".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: Some(MemberPresence::Unavailable),
                local_binding_available: Some(true),
            },
            FederatedRoomMemberProjection {
                member_id: "agent-live".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::Agent,
                role_in_room: FederatedRoomRole::Member,
                display_name: "Flux".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: Some(MemberPresence::Live),
                local_binding_available: Some(true),
            },
        ];

        assert_eq!(roster_presence_count(&members), 2);
    }
}
