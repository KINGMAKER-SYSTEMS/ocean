//! The room header's single overflow menu (team-platform P4): share, per-room
//! agent settings, leave. Secondary actions live here, never as a row of
//! header buttons.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::rooms::{
    fetch_agent_settings, save_agent_settings, RoomAgentSettings, RoomParticipant,
    RoomParticipantKind, Rooms,
};

/// Agent participants of the open room.
fn room_agents(rooms: Rooms) -> Vec<RoomParticipant> {
    rooms
        .open_room
        .get()
        .map(|room| {
            room.participants
                .into_iter()
                .filter(|p| p.kind == RoomParticipantKind::Agent)
                .collect()
        })
        .unwrap_or_default()
}

/// The open room's mute pref, as carried by the room list.
fn room_muted(rooms: Rooms) -> bool {
    let key = rooms.open_key.get();
    rooms.list.with(|list| {
        list.iter()
            .find(|r| Some(&r.id) == key.as_ref())
            .map(|r| r.muted)
            .unwrap_or(false)
    })
}

/// Settings body from the two fields; blanks clear.
pub(crate) fn settings_from_fields(instructions: &str, model: &str) -> RoomAgentSettings {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    RoomAgentSettings {
        instructions: clean(instructions),
        model: clean(model),
    }
}

#[component]
pub fn RoomOverflow(rooms: Rooms, joined: bool) -> impl IntoView {
    let open = RwSignal::new(false);
    let settings_open = RwSignal::new(false);
    let can_share =
        move || crate::rooms_workspace::access_allows_sharing(rooms.access.get().as_ref());
    let has_agents = move || !room_agents(rooms).is_empty();

    view! {
        <div class="room-overflow">
            <button
                class="room-overflow__trigger"
                type="button"
                aria-label="Room options"
                aria-haspopup="menu"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <crate::icons::More />
            </button>
            <Show when=move || open.get()>
                <div class="room-overflow__scrim" on:click=move |_| open.set(false)></div>
                <div
                    class="room-overflow__menu"
                    role="menu"
                    on:keydown=move |ev| {
                        if ev.key() == "Escape" {
                            open.set(false);
                        }
                    }
                >
                    {move || can_share().then(|| view! {
                        <button
                            class="room-overflow__item"
                            role="menuitem"
                            type="button"
                            disabled=move || rooms.invite_loading.get()
                            on:click=move |_| {
                                open.set(false);
                                rooms.create_invite();
                            }
                        >
                            "Share"
                        </button>
                    })}
                    {move || has_agents().then(|| view! {
                        <button
                            class="room-overflow__item"
                            role="menuitem"
                            type="button"
                            on:click=move |_| {
                                open.set(false);
                                settings_open.set(true);
                            }
                        >
                            "Agents"
                        </button>
                    })}
                    {move || {
                        let muted = room_muted(rooms);
                        view! {
                            <button
                                class="room-overflow__item"
                                role="menuitem"
                                type="button"
                                on:click=move |_| {
                                    open.set(false);
                                    crate::room_attention::toggle_mute(rooms, muted);
                                }
                            >
                                {if muted { "Unmute" } else { "Mute" }}
                            </button>
                        }
                    }}
                    {joined.then(|| view! {
                        <button
                            class="room-overflow__item room-overflow__item--danger"
                            role="menuitem"
                            type="button"
                            on:click=move |_| {
                                open.set(false);
                                rooms.leave_open();
                            }
                        >
                            "Leave"
                        </button>
                    })}
                </div>
            </Show>
            <Show when=move || settings_open.get()>
                <RoomAgentSettingsPanel rooms=rooms open=settings_open />
            </Show>
        </div>
    }
}

/// Per-room overrides for each agent in the room: an instructions overlay and
/// a model. Local to this Ocean; never federated.
#[component]
fn RoomAgentSettingsPanel(rooms: Rooms, open: RwSignal<bool>) -> impl IntoView {
    let agents = Memo::new(move |_| room_agents(rooms));
    let selected = RwSignal::new(agents.get_untracked().first().map(|agent| agent.id.clone()));
    let instructions = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);
    let saved = RwSignal::new(false);

    Effect::new(move |_| {
        let Some(agent) = selected.get() else { return };
        let Some(key) = rooms.open_key.get_untracked() else {
            return;
        };
        let base = rooms.url.get_untracked();
        instructions.set(String::new());
        model.set(String::new());
        error.set(None);
        saved.set(false);
        spawn_local(async move {
            let result = fetch_agent_settings(&base, &key, &agent).await;
            if selected.get_untracked().as_deref() != Some(agent.as_str()) {
                return;
            }
            match result {
                Ok(settings) => {
                    instructions.set(settings.instructions.unwrap_or_default());
                    model.set(settings.model.unwrap_or_default());
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });

    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        let (Some(agent), Some(key)) = (selected.get_untracked(), rooms.open_key.get_untracked())
        else {
            return;
        };
        let base = rooms.url.get_untracked();
        let body = settings_from_fields(&instructions.get_untracked(), &model.get_untracked());
        saving.set(true);
        error.set(None);
        spawn_local(async move {
            let result = save_agent_settings(&base, &key, &agent, &body).await;
            saving.set(false);
            match result {
                Ok(_) => saved.set(true),
                Err(e) => error.set(Some(e)),
            }
        });
    };

    view! {
        <div class="room-agent-settings" role="dialog" aria-label="Agent settings">
            <div class="room-agent-settings__head">
                <div class="room-agent-settings__agents" role="group" aria-label="Agent">
                    <For
                        each=move || agents.get()
                        key=|agent| agent.id.clone()
                        children=move |agent| {
                            let id = agent.id.clone();
                            let pick = agent.id.clone();
                            view! {
                                <button
                                    class="room-agent-settings__agent"
                                    type="button"
                                    aria-pressed=move || {
                                        (selected.get().as_deref() == Some(id.as_str())).to_string()
                                    }
                                    on:click=move |_| selected.set(Some(pick.clone()))
                                >
                                    {agent.display_name.clone()}
                                </button>
                            }
                        }
                    />
                </div>
                <button
                    class="room-agent-settings__close"
                    type="button"
                    aria-label="Close agent settings"
                    on:click=move |_| open.set(false)
                >
                    <crate::icons::Close />
                </button>
            </div>
            <textarea
                class="room-agent-settings__instructions"
                aria-label="Instructions"
                placeholder="Instructions"
                rows="5"
                prop:value=move || instructions.get()
                on:input=move |ev| {
                    instructions.set(event_target_value(&ev));
                    saved.set(false);
                }
            ></textarea>
            <input
                class="room-agent-settings__model"
                type="text"
                aria-label="Model"
                placeholder="Model"
                spellcheck="false"
                prop:value=move || model.get()
                on:input=move |ev| {
                    model.set(event_target_value(&ev));
                    saved.set(false);
                }
            />
            <div class="room-agent-settings__foot">
                {move || error.get().map(|e| view! {
                    <span class="room-agent-settings__error" role="alert">{e}</span>
                })}
                <button
                    class="room-agent-settings__save"
                    type="button"
                    disabled=move || saving.get()
                    on:click=save
                >
                    {move || if saved.get() { "Saved" } else { "Save" }}
                </button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_fields_trims_and_clears_blanks() {
        assert_eq!(
            settings_from_fields("  be brief ", " glm-5.3 "),
            RoomAgentSettings {
                instructions: Some("be brief".into()),
                model: Some("glm-5.3".into()),
            }
        );
        assert_eq!(settings_from_fields("  ", ""), RoomAgentSettings::default());
    }
}
