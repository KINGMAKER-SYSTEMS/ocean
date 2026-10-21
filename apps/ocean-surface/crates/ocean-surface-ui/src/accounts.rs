//! Account connections use native intent and daemon-owned OAuth. No credentials
//! or authorization URLs enter this view. Closing stops reads, not a login the
//! user is completing in their browser; reopening shows its current daemon state.
use crate::{
    daemon::Daemon,
    host::{self, ProviderAccount},
};
use leptos::prelude::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

fn account_name(provider: &str) -> &str {
    match provider {
        "claude" => "Claude",
        "codex" => "Codex",
        "chatgpt" => "ChatGPT",
        _ => provider,
    }
}
fn account_error(code: &str) -> &'static str {
    match code {
        "outcome_unknown" => "Connection status is uncertain. Check before trying again.",
        "login_unavailable" => {
            "Sign-in couldn't start. Close any other sign-in window and try again."
        }
        "account_access_refused" => "Ocean couldn't access account settings.",
        "account_unavailable" => "Update Ocean to connect accounts here.",
        "daemon_unreachable" => "Start Ocean to connect an account.",
        _ => "Couldn't load account settings.",
    }
}
fn is_pending(account: &ProviderAccount) -> bool {
    account
        .login
        .as_ref()
        .is_some_and(|login| login.state == "pending")
}
fn account_detail(account: &ProviderAccount) -> &'static str {
    if is_pending(account) {
        "Waiting for sign-in…"
    } else if account.status == "signed_in" {
        "Connected"
    } else if account
        .login
        .as_ref()
        .is_some_and(|login| login.state == "failed")
    {
        "Sign-in didn't finish"
    } else if account.status == "expired" {
        "Sign-in expired"
    } else if account.status == "unknown" {
        "Status unavailable"
    } else {
        ""
    }
}

#[derive(Clone)]
struct AccountView {
    rows: RwSignal<Vec<ProviderAccount>>,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    loading: RwSignal<bool>,
    alive: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    daemon: StoredValue<Daemon>,
}
impl AccountView {
    fn current(&self, revision: u64) -> bool {
        self.alive.load(Ordering::Relaxed) && self.revision.load(Ordering::Relaxed) == revision
    }
    fn refresh(&self) {
        let view = self.clone();
        let revision = view
            .revision
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        view.loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let result = host::provider_accounts("list", None, None).await;
                if !view.current(revision) {
                    return;
                }
                view.loading.set(false);
                match result {
                    Ok(reply) => {
                        let rows = reply.providers.unwrap_or_default();
                        let pending = rows.iter().any(is_pending);
                        if rows != view.rows.get_untracked() {
                            view.daemon.with_value(|daemon| daemon.fetch_models());
                        }
                        view.rows.set(rows);
                        if !pending {
                            return;
                        }
                    }
                    Err(error) => {
                        view.error.set(Some(account_error(&error).into()));
                        return;
                    }
                }
                gloo_timers::future::TimeoutFuture::new(2_000).await;
                if !view.current(revision) {
                    return;
                }
            }
        });
    }
    fn act(&self, account: ProviderAccount) {
        if self.busy.get_untracked() || self.loading.get_untracked() {
            return;
        }
        let view = self.clone();
        let revision = view
            .revision
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        view.busy.set(true);
        view.error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let attempt = account
                .login
                .as_ref()
                .filter(|login| login.state == "pending");
            let action = if attempt.is_some() { "cancel" } else { "login" };
            let result = host::provider_accounts(
                action,
                Some(&account.provider),
                attempt.map(|login| login.attempt_id.as_str()),
            )
            .await;
            if !view.current(revision) {
                return;
            }
            match result {
                Ok(reply) => {
                    if reply.browser_opened == Some(false) {
                        view.error.set(Some(
                            "Couldn't open your browser. Cancel this sign-in, then try again."
                                .into(),
                        ));
                    }
                    // Render the exact admitted attempt immediately; the next
                    // authoritative list read owns subsequent state.
                    if let Some(login) = reply.login {
                        view.rows.update(|rows| {
                            if let Some(row) =
                                rows.iter_mut().find(|row| row.provider == login.provider)
                            {
                                row.login = Some(login);
                            }
                        });
                    }
                }
                Err(error) => view.error.set(Some(account_error(&error).into())),
            }
            view.busy.set(false);
            view.refresh();
        });
    }
}

#[component]
pub fn AccountsMenuItem(on_open: Callback<()>) -> impl IntoView {
    // Account callbacks need this computer's native host. Keep that capability
    // inside the feature; the shared header and its navigation stay identical.
    view! {
        <Show when=host::running_in_tauri>
            <button class="ocean-more__item" type="button" role="menuitem"
                on:click=move |_| on_open.run(())>"Accounts"</button>
        </Show>
    }
}

#[component]
pub fn Accounts(daemon: Daemon, on_close: Callback<()>) -> impl IntoView {
    let view = AccountView {
        rows: RwSignal::new(Vec::new()),
        error: RwSignal::new(None),
        busy: RwSignal::new(false),
        loading: RwSignal::new(true),
        alive: Arc::new(AtomicBool::new(true)),
        revision: Arc::new(AtomicU64::new(0)),
        daemon: StoredValue::new(daemon),
    };
    let rows = view.rows;
    let error = view.error;
    let busy = view.busy;
    let loading = view.loading;
    let dialog = NodeRef::<leptos::html::Dialog>::new();
    Effect::new(move |_| {
        if let Some(dialog) = dialog.get() {
            let _ = dialog.show_modal();
        }
    });
    let cleanup = view.clone();
    on_cleanup(move || {
        cleanup.alive.store(false, Ordering::Relaxed);
        cleanup.revision.fetch_add(1, Ordering::Relaxed);
    });
    view.refresh();
    let actions = StoredValue::new(view);
    view! {
        <dialog node_ref=dialog class="ocean-accounts" aria-labelledby="ocean-accounts-title"
            on:keydown=move |event| { if event.key()=="Escape" { event.stop_propagation(); } }
            on:cancel=move |event: web_sys::Event| { event.prevent_default(); on_close.run(()); }>
            <header class="ocean-accounts__header">
                <h2 id="ocean-accounts-title">"Accounts"</h2>
                <button type="button" class="ocean-accounts__close" aria-label="Close accounts" on:click=move |_| on_close.run(())><crate::icons::Close /></button>
            </header>
            <Show when=move || loading.get() && rows.get().is_empty()><p role="status">"Loading…"</p></Show>
            <ul class="ocean-accounts__list">
                <For each={move || rows.get().into_iter().map(|row| row.provider).collect::<Vec<_>>()} key=|provider| provider.clone() children=move |provider| {
                    let name=account_name(&provider).to_string();
                    let account=Memo::new(move |_| rows.get().into_iter().find(|row| row.provider==provider));
                    let pending=move || account.get().as_ref().is_some_and(is_pending);
                    let connected=move || account.get().is_some_and(|row| row.status=="signed_in");
                    let accessible_name=name.clone();
                    view! {
                        <li class="ocean-accounts__row">
                            <div><span class="ocean-accounts__name">{name}</span><span class="ocean-accounts__status" role="status">{move || account.get().as_ref().map(account_detail).unwrap_or_default()}</span></div>
                            <Show when=move || pending() || !connected()>
                                <button type="button" aria-disabled=move || (busy.get() || loading.get()).to_string()
                                    aria-label={let name=accessible_name.clone(); move || format!("{} {}",if pending() { "Cancel sign-in for" } else { "Connect" },name)}
                                    on:click=move |_| { if let Some(row)=account.get_untracked() { actions.with_value(|view| view.act(row)); } }>
                                    {move || if pending() { "Cancel" } else { "Connect" }}
                                </button>
                            </Show>
                        </li>
                    }
                } />
            </ul>
            <Show when=move || error.get().is_some()>
                <p class="ocean-accounts__error" role="alert">{move || error.get()}</p>
                <button class="ocean-accounts__retry" type="button" disabled=move || busy.get() || loading.get()
                    on:click=move |_| { error.set(None); actions.with_value(|view| view.refresh()); }>"Check again"</button>
            </Show>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_status_never_claims_success_from_a_pending_or_failed_attempt() {
        let mut row = ProviderAccount {
            provider: "codex".into(),
            status: "signed_out".into(),
            login: None,
        };
        assert_eq!(account_detail(&row), "");
        row.login = Some(crate::host::AccountLogin {
            provider: "codex".into(),
            attempt_id: "id".into(),
            state: "pending".into(),
        });
        assert_eq!(account_detail(&row), "Waiting for sign-in…");
        row.login.as_mut().unwrap().state = "failed".into();
        assert_eq!(account_detail(&row), "Sign-in didn't finish");
        row.status = "signed_in".into();
        assert_eq!(account_detail(&row), "Connected");
    }
    #[test]
    fn account_projection_retires_on_new_intent_or_close() {
        let owner = Owner::new();
        owner.with(|| {
            let view = AccountView {
                rows: RwSignal::new(Vec::new()),
                error: RwSignal::new(None),
                busy: RwSignal::new(false),
                loading: RwSignal::new(false),
                alive: Arc::new(AtomicBool::new(true)),
                revision: Arc::new(AtomicU64::new(1)),
                daemon: StoredValue::new(Daemon::dummy()),
            };
            // Disabled actions retain native keyboard focus through aria-disabled,
            // so handler admission must reject both mutation and read latches.
            for (busy, loading) in [(true, false), (false, true)] {
                view.busy.set(busy);
                view.loading.set(loading);
                view.act(ProviderAccount {
                    provider: "codex".into(),
                    status: "signed_out".into(),
                    login: None,
                });
                assert_eq!(view.revision.load(Ordering::Relaxed), 1);
            }
            view.busy.set(false);
            view.loading.set(false);
            assert!(view.current(1));
            view.revision.fetch_add(1, Ordering::Relaxed);
            assert!(!view.current(1));
            assert!(view.current(2));
            view.alive.store(false, Ordering::Relaxed);
            assert!(!view.current(2));
        });
    }
}
