//! Room agent consent and explicit invocation (ocean-private #65).
//!
//! The consent sheet renders a native-frozen preview; binding rows get one
//! overflow with Review/Revoke; the message action rail gets one invoke
//! trigger. Every privileged step is brokered by the native shell through
//! `crate::host`, so browser hosts render bindings but never emulate consent.
//! Menus reuse the room header's `.room-overflow__*` styles.

use leptos::prelude::*;

use super::format::avatar_identity_class;
use crate::rooms::{AgentSummary, InvocationState, RoomAgentBinding, Rooms};

/// `sha256:` plus the first 12 hex digits; the full digest stays in the title.
fn short_digest(digest: &str) -> String {
    match digest.strip_prefix("sha256:") {
        Some(hex) if hex.len() > 12 => format!("sha256:{}", &hex[..12]),
        _ => digest.to_string(),
    }
}

fn invocation_label(state: &InvocationState) -> &'static str {
    match state {
        InvocationState::Pending => "sending",
        InvocationState::Acknowledged { .. } => "queued",
        InvocationState::Refused(_) => "refused",
        InvocationState::Unknown => "unknown",
    }
}

/// Binding for a roster member, matched on the daemon agent member id.
fn binding_for(bindings: &[RoomAgentBinding], member_id: &str) -> Option<RoomAgentBinding> {
    bindings
        .iter()
        .find(|b| b.agent_member_id == member_id)
        .cloned()
}

/// Picker entries: agents not already in the room. Unresolvable folders stay
/// listed (rendered disabled) so the operator can see why they are missing.
fn addable_agents(available: Vec<AgentSummary>, present: &[String]) -> Vec<AgentSummary> {
    available
        .into_iter()
        .filter(|agent| !present.contains(&agent.name))
        .collect()
}

/// Agent picker: one ghost trigger, one select. Selecting registers (or
/// adds) the agent; native hosts then open the consent sheet.
#[component]
pub(super) fn AddAgentPicker(rooms: Rooms) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <button
            class="rooms-workspace__addagent"
            type="button"
            aria-controls="rooms-workspace-agent-picker"
            aria-expanded=move || open.get().to_string()
            on:click=move |_| open.update(|v| *v = !*v)
        >
            "+ agent"
        </button>
        {move || {
            open.get().then(|| view! {
                <div id="rooms-workspace-agent-picker" class="rooms-workspace__addagent-picker">
                    <select
                        class="rooms-workspace__addagent-select"
                        aria-label="Agent"
                        on:change=move |ev| {
                            let val = event_target_value(&ev);
                            if !val.is_empty() {
                                rooms.add_agent(val);
                                open.set(false);
                            }
                        }
                    >
                        <option value="" selected=true>"Agent"</option>
                        <For
                            each=move || {
                                let present: Vec<String> = rooms
                                    .open_room
                                    .get()
                                    .map(|r| r.participants.into_iter().map(|p| p.id).collect())
                                    .unwrap_or_default();
                                addable_agents(rooms.available_agents.get(), &present)
                            }
                            key=|agent: &AgentSummary| agent.name.clone()
                            children=move |agent: AgentSummary| {
                                let v = agent.name.clone();
                                view! {
                                    <option value=v disabled=agent.error.is_some()>{agent.name}</option>
                                }
                            }
                        />
                    </select>
                </div>
            })
        }}
    }
}

/// Frozen native consent: name, digest, requested capabilities, one primary
/// action and a dismiss. Absent unless the native shell froze a preview.
#[component]
pub(super) fn ConsentSheet(rooms: Rooms) -> impl IntoView {
    move || {
        let draft = rooms.consent.get()?;
        let preview = draft.preview.clone();
        let busy = draft.busy;
        let initials = preview
            .display_name
            .chars()
            .take(2)
            .collect::<String>()
            .to_uppercase();
        let action = if preview.mode == "reauthorize" {
            "Reauthorize"
        } else {
            "Authorize"
        };
        let caps = preview.requested_capabilities.clone();
        Some(view! {
            <div class="rooms-workspace__consent" role="group" aria-label=format!("{action} {}", preview.display_name)>
                <div class="rooms-workspace__consent-head">
                    <span class=format!(
                        "rooms-workspace__member-avatar {}",
                        avatar_identity_class(&preview.agent_member_id)
                    )>{initials}</span>
                    <span class="rooms-workspace__consent-name">{preview.display_name.clone()}</span>
                    <button
                        class="rooms-workspace__right-close"
                        type="button"
                        aria-label="Dismiss"
                        disabled=busy
                        on:click=move |_| rooms.dismiss_consent()
                    >
                        <svg viewBox="0 0 16 16" width="14" height="14"
                            fill="none" stroke="currentColor" stroke-width="1.6"
                            stroke-linecap="round">
                            <path d="M3 3l10 10M13 3L3 13"/>
                        </svg>
                    </button>
                </div>
                <code class="rooms-workspace__consent-digest" title=preview.definition_digest.clone()>
                    {short_digest(&preview.definition_digest)}
                </code>
                {(!caps.is_empty()).then(|| view! {
                    <div class="rooms-workspace__consent-caps" role="list" aria-label="Capabilities">
                        {caps.into_iter().map(|cap| view! {
                            <span class="rooms-workspace__consent-cap" role="listitem">{cap}</span>
                        }).collect_view()}
                    </div>
                })}
                <button
                    class="rooms-workspace__consent-primary"
                    type="button"
                    disabled=busy
                    on:click=move |_| rooms.authorize_consent()
                >
                    {action}
                </button>
            </div>
        })
    }
}

/// One overflow per bound agent row; Review and Revoke live inside it.
/// Native hosts only — browser hosts never emulate consent.
#[component]
pub(super) fn AgentBindingMenu(rooms: Rooms, member_id: String) -> impl IntoView {
    let open = RwSignal::new(false);
    move || {
        if !crate::host::room_consent_supported() {
            return None;
        }
        let binding = binding_for(&rooms.agent_bindings.get(), &member_id)?;
        let status = binding.status.clone();
        let review_package = binding.agent_package_id.clone();
        let revoke_binding = binding.clone();
        let revocable = status != "revoked";
        Some(view! {
            {(status != "active").then(|| view! {
                <span class="rooms-workspace__member-binding">{status.clone()}</span>
            })}
            <span class="room-overflow">
                <button
                    class="room-overflow__trigger"
                    type="button"
                    aria-label=format!("{} options", binding.display_name)
                    aria-haspopup="menu"
                    aria-expanded=move || open.get().to_string()
                    on:click=move |_| open.update(|v| *v = !*v)
                >
                    <crate::icons::More />
                </button>
                {move || open.get().then(|| {
                    let review_package = review_package.clone();
                    let revoke_binding = revoke_binding.clone();
                    view! {
                        <div class="room-overflow__menu" role="menu">
                            <button
                                class="room-overflow__item"
                                type="button"
                                role="menuitem"
                                on:click=move |_| {
                                    open.set(false);
                                    rooms.review_agent(review_package.clone());
                                }
                            >
                                "Review"
                            </button>
                            {revocable.then(|| view! {
                                <button
                                    class="room-overflow__item room-overflow__item--danger"
                                    type="button"
                                    role="menuitem"
                                    on:click=move |_| {
                                        open.set(false);
                                        rooms.revoke_agent(revoke_binding.clone());
                                    }
                                >
                                    "Revoke"
                                </button>
                            })}
                        </div>
                    }
                })}
            </span>
        })
    }
}

/// Explicit invocation from a committed message the authoritative caller
/// authored. One trigger; active agents and their invocation state inside.
#[component]
pub(super) fn InvokeMenu(rooms: Rooms, message_seq: u64) -> impl IntoView {
    let open = RwSignal::new(false);
    move || {
        let caller = crate::rooms::invoke_caller(rooms.access.get().as_ref())?;
        let transcript = rooms.transcript.get();
        let message = transcript.iter().find(|m| m.seq == message_seq)?;
        if !crate::rooms::message_invocable(message, &caller) {
            return None;
        }
        let agents = crate::rooms::invocable_bindings(&rooms.agent_bindings.get());
        if agents.is_empty() {
            return None;
        }
        Some(view! {
            <span class="room-overflow">
                <button
                    class="rooms-workspace__thread-toggle rooms-workspace__invoke-trigger"
                    type="button"
                    aria-label=format!("Invoke an agent on message {message_seq}")
                    aria-haspopup="menu"
                    aria-expanded=move || open.get().to_string()
                    on:click=move |_| open.update(|v| *v = !*v)
                >
                    <crate::icons::Robot />
                </button>
                {move || open.get().then(|| {
                    let invocations = rooms.invocations.get();
                    view! {
                        <div class="room-overflow__menu" role="menu">
                            {agents.iter().map(|agent| {
                                let state = invocations
                                    .iter()
                                    .find(|i| i.agent_member_id == agent.agent_member_id && i.message_seq == message_seq)
                                    .map(|i| invocation_label(&i.state));
                                let agent_id = agent.agent_member_id.clone();
                                let name = if agent.display_name.is_empty() {
                                    agent.agent_member_id.clone()
                                } else {
                                    agent.display_name.clone()
                                };
                                view! {
                                    <button
                                        class="room-overflow__item"
                                        type="button"
                                        role="menuitem"
                                        disabled=state.is_some()
                                        on:click=move |_| {
                                            open.set(false);
                                            rooms.invoke_agent(agent_id.clone(), message_seq);
                                        }
                                    >
                                        {name}
                                        {state.map(|label| view! {
                                            <span class="rooms-workspace__member-binding">{label}</span>
                                        })}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    }
                })}
            </span>
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_digest_is_shortened_and_bindings_match_member_ids() {
        assert_eq!(
            short_digest("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "sha256:0123456789ab"
        );
        assert_eq!(short_digest("sha256:abc"), "sha256:abc");
        assert_eq!(short_digest("opaque"), "opaque");
        let binding = RoomAgentBinding {
            agent_member_id: "agent-x".into(),
            agent_package_id: "scout".into(),
            agent_definition_digest: String::new(),
            display_name: "Scout".into(),
            owner_member_id: "human-a".into(),
            status: "active".into(),
            generation: "1".into(),
            owner_eligible: true,
        };
        let bindings = vec![binding.clone()];
        assert_eq!(binding_for(&bindings, "agent-x"), Some(binding));
        assert_eq!(binding_for(&bindings, "scout"), None);
        assert_eq!(invocation_label(&InvocationState::Unknown), "unknown");
    }

    #[test]
    fn picker_hides_present_agents_and_keeps_unresolved_ones() {
        let agent = |name: &str, error: Option<&str>| AgentSummary {
            name: name.into(),
            description: None,
            model: None,
            skills: 0,
            subagents: Vec::new(),
            error: error.map(Into::into),
        };
        let available = vec![
            agent("scout", None),
            agent("flux", None),
            agent("broken", Some("unresolved")),
        ];
        let names: Vec<String> = addable_agents(available, &["scout".into()])
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["flux", "broken"]);
    }
}
