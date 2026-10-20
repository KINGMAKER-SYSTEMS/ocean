//! Native-only initial team setup, using the existing menu and dialog vocabulary.
use crate::host::{self, TeamConnection};
use leptos::prelude::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

fn error_text(code: &str) -> &'static str {
    match code {
        "invalid_team_origin" | "invalid_team_action" => {
            "Enter the team's server address, without a path or sign-in details."
        }
        "team_origin_already_configured" | "existing_team_connection" => {
            "This Ocean already has a team connection."
        }
        "team_setup_unavailable" => "Update Ocean to set up a team here.",
        "team_access_refused" => "Ocean couldn't access team settings.",
        "outcome_unknown" => "The save wasn't confirmed. Check its status before trying again.",
        "daemon_unreachable" => "Start Ocean to load team settings.",
        "restart_failed" => "Ocean couldn't restart. Your server address is saved.",
        _ => "Couldn't load team settings.",
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Save,
    Restart,
    Check,
    None,
}
fn action_for(connection: Option<&TeamConnection>, owned: bool) -> Action {
    match connection {
        None => Action::Check,
        Some(state) if state.can_initialize => Action::Save,
        Some(state) if state.restart_required && owned => Action::Restart,
        Some(state) if state.restart_required => Action::Check,
        _ => Action::None,
    }
}
fn detail(connection: Option<&TeamConnection>, owned: bool) -> &'static str {
    match connection {
        Some(state) if state.configured => "Configured",
        Some(state) if state.restart_required && owned => "Saved · restart required",
        Some(state) if state.restart_required => {
            "Saved · restart the Ocean service, then check again."
        }
        Some(state) if !state.can_initialize => {
            "This Ocean's existing team connection needs attention."
        }
        _ => "",
    }
}
#[derive(Clone)]
struct TeamView {
    connection: RwSignal<Option<TeamConnection>>,
    draft: RwSignal<String>,
    owned: RwSignal<bool>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<&'static str>>,
    alive: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
}
impl TeamView {
    fn current(&self, ticket: u64) -> bool {
        self.alive.load(Ordering::Relaxed) && self.revision.load(Ordering::Relaxed) == ticket
    }
    fn begin(&self) -> Option<u64> {
        if self.busy.get_untracked() {
            return None;
        }
        self.busy.set(true);
        self.error.set(None);
        Some(
            self.revision
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1),
        )
    }
    fn settle(&self, ticket: u64, result: Result<TeamConnection, String>) {
        if !self.current(ticket) {
            return;
        }
        match result {
            Ok(state) => {
                if let Some(origin) = &state.origin {
                    self.draft.set(origin.clone());
                }
                self.connection.set(Some(state));
            }
            Err(code) => {
                // A lost save acknowledgement must be read back before another save.
                if !matches!(code.as_str(), "invalid_team_origin" | "invalid_team_action") {
                    self.connection.set(None);
                }
                self.error.set(Some(error_text(&code)));
            }
        }
        self.busy.set(false);
    }
    fn read(&self) {
        let Some(ticket) = self.begin() else {
            return;
        };
        let view = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let status = host::daemon_status().await;
            if !view.current(ticket) {
                return;
            }
            view.owned.set(status.is_some_and(|s| s.pid.is_some()));
            let result = host::team_connection(None).await;
            view.settle(ticket, result);
        });
    }
    fn act(&self) {
        let action = action_for(
            self.connection.get_untracked().as_ref(),
            self.owned.get_untracked(),
        );
        if action == Action::Check {
            self.read();
            return;
        }
        let origin = self.draft.get_untracked();
        if action == Action::None || (action == Action::Save && origin.trim().is_empty()) {
            return;
        }
        let Some(ticket) = self.begin() else {
            return;
        };
        let view = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if action == Action::Save {
                let result = host::team_connection(Some(origin.trim())).await;
                view.settle(ticket, result);
                return;
            }
            // Re-read ownership before restart; the shell never stops external daemons.
            let status = host::daemon_status().await;
            if !view.current(ticket) {
                return;
            }
            if !status.is_some_and(|s| s.pid.is_some()) {
                view.owned.set(false);
                view.busy.set(false);
                return;
            }
            let restarted = host::daemon_restart().await;
            if !view.current(ticket) {
                return;
            }
            if !restarted {
                view.error.set(Some(error_text("restart_failed")));
                view.busy.set(false);
                return;
            }
            // RPC success is not activation. Only the daemon's new projection is.
            for attempt in 0..8 {
                if attempt > 0 {
                    gloo_timers::future::TimeoutFuture::new(1_000).await;
                }
                if !view.current(ticket) {
                    return;
                }
                let result = host::team_connection(None).await;
                if !view.current(ticket) {
                    return;
                }
                if result.as_ref().is_ok_and(|state| state.configured) || attempt == 7 {
                    view.settle(ticket, result);
                    return;
                }
            }
        });
    }
}
#[component]
pub fn TeamMenuItem(on_open: Callback<()>) -> impl IntoView {
    view! { <Show when=host::running_in_tauri>
        <button class="ocean-more__item" type="button" role="menuitem" on:click=move |_| on_open.run(())>"Team"</button>
    </Show> }
}
#[component]
pub fn TeamSettings(on_close: Callback<()>) -> impl IntoView {
    let state = TeamView {
        connection: RwSignal::new(None),
        draft: RwSignal::new(String::new()),
        owned: RwSignal::new(false),
        busy: RwSignal::new(false),
        error: RwSignal::new(None),
        alive: Arc::new(AtomicBool::new(true)),
        revision: Arc::new(AtomicU64::new(0)),
    };
    let connection = state.connection;
    let draft = state.draft;
    let owned = state.owned;
    let busy = state.busy;
    let error = state.error;
    let dialog = NodeRef::<leptos::html::Dialog>::new();
    Effect::new(move |_| {
        if let Some(dialog) = dialog.get() {
            let _ = dialog.show_modal();
        }
    });
    let cleanup = state.clone();
    on_cleanup(move || {
        cleanup.alive.store(false, Ordering::Relaxed);
        cleanup.revision.fetch_add(1, Ordering::Relaxed);
    });
    state.read();
    let actions = StoredValue::new(state);
    let action = move || action_for(connection.get().as_ref(), owned.get());
    view! {
        <dialog node_ref=dialog class="ocean-accounts ocean-team" aria-labelledby="ocean-team-title"
            on:keydown=move |event| { if event.key()=="Escape" { event.stop_propagation(); } }
            on:cancel=move |event: web_sys::Event| { event.prevent_default(); on_close.run(()); }>
            <header class="ocean-accounts__header">
                <h2 id="ocean-team-title">"Team"</h2>
                <button type="button" class="ocean-accounts__close" aria-label="Close team settings" on:click=move |_| on_close.run(())><crate::icons::Close /></button>
            </header>
            <form on:submit=move |event| { event.prevent_default(); actions.with_value(|s| s.act()); }>
                <label for="ocean-team-origin">"Server"</label>
                <input id="ocean-team-origin" type="url" autocomplete="url" maxlength="2048" spellcheck="false"
                    autofocus=true placeholder="https://" prop:value=move || draft.get()
                    readonly=move || busy.get() || action()!=Action::Save
                    on:input=move |event| { if !busy.get_untracked() && action_for(connection.get_untracked().as_ref(),owned.get_untracked())==Action::Save { draft.set(event_target_value(&event)); } } />
                <p class="ocean-team__status" role="status">{move || if busy.get() { "Working…" } else { detail(connection.get().as_ref(),owned.get()) }}</p>
                <Show when=move || error.get().is_some()><p class="ocean-accounts__error" role="alert">{move || error.get()}</p></Show>
                <Show when=move || action()!=Action::None>
                    <button type="submit" aria-disabled=move || (busy.get() || (action()==Action::Save && draft.get().trim().is_empty())).to_string()>
                        {move || match action() { Action::Save=>"Save", Action::Restart=>"Restart Ocean", _=>"Check again" }}
                    </button>
                </Show>
            </form>
        </dialog>
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_origin_never_means_connected_and_external_services_are_not_restarted() {
        let mut state = TeamConnection {
            origin: Some("https://team.test/".into()),
            configured: false,
            restart_required: true,
            can_initialize: false,
        };
        assert_eq!(action_for(Some(&state), true), Action::Restart);
        assert_eq!(action_for(Some(&state), false), Action::Check);
        assert!(!detail(Some(&state), true).contains("Configured"));
        state.configured = true;
        state.restart_required = false;
        assert_eq!(action_for(Some(&state), true), Action::None);
        assert_eq!(detail(Some(&state), true), "Configured");
    }
    #[test]
    fn stale_reads_and_lost_save_ack_cannot_restore_save_authority() {
        let owner = Owner::new();
        owner.with(|| {
            let state = TeamView {
                connection: RwSignal::new(None),
                draft: RwSignal::new("https://team.test".into()),
                owned: RwSignal::new(true),
                busy: RwSignal::new(false),
                error: RwSignal::new(None),
                alive: Arc::new(AtomicBool::new(true)),
                revision: Arc::new(AtomicU64::new(0)),
            };
            state.connection.set(Some(TeamConnection {
                origin: None,
                configured: false,
                restart_required: false,
                can_initialize: true,
            }));
            let invalid = state.begin().unwrap();
            state.settle(invalid, Err("invalid_team_origin".into()));
            assert_eq!(
                action_for(state.connection.get_untracked().as_ref(), true),
                Action::Save
            );
            let ticket = state.begin().unwrap();
            assert!(state.begin().is_none());
            state.settle(ticket, Err("outcome_unknown".into()));
            assert_eq!(
                action_for(state.connection.get_untracked().as_ref(), true),
                Action::Check
            );
            assert_eq!(state.draft.get_untracked(), "https://team.test");
            let next = state.begin().unwrap();
            state.settle(
                ticket,
                Ok(TeamConnection {
                    origin: None,
                    configured: false,
                    restart_required: false,
                    can_initialize: true,
                }),
            );
            assert!(state.connection.get_untracked().is_none());
            assert!(state.busy.get_untracked());
            state.alive.store(false, Ordering::Relaxed);
            assert!(!state.current(next));
        });
    }
}
