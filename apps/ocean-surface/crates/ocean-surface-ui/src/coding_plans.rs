//! Coding plans — the Claude and Codex subscription logins on the machine
//! running this Ocean (web identity program, milestone M3).
//!
//! The daemon owns the credentials and the OAuth flow
//! (`platform/ocean-os/crates/ocean-daemon/src/provider_auth.rs`); the proxy
//! forwards `/v1/auth/providers/*` to it and attaches the operator credential
//! server side, and the Tauri shell does the same natively. This module only
//! renders what the daemon reports and collects intent: sign in, cancel a
//! sign-in, sign out.
//!
//! Three rules shape it.
//!
//! 1. **The sign-in finishes on the daemon's machine, not here.** The
//!    provider redirects to a loopback callback on the machine running the
//!    daemon, so the panel says so plainly. It never pretends the browser in
//!    front of you completes the flow.
//! 2. **Nothing raw reaches the screen.** Every refusal is a typed `error`
//!    code; each one becomes a sentence here, and an unknown code becomes a
//!    generic sentence rather than a code or a JSON body.
//! 3. **Polling is owned by the panel.** A sign-in attempt is polled only
//!    while the panel is open; closing it, cancelling, or starting another
//!    attempt retires the loop by generation, so no timer outlives its reason.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;
use wasm_bindgen::JsValue;

/// Seconds between attempt polls.
const POLL_INTERVAL_MS: u32 = 2_000;
/// Give up waiting after this many polls (~10 minutes).
const POLL_LIMIT: u32 = 300;
/// Consecutive transport failures tolerated before a poll loop gives up.
const POLL_FAILURE_LIMIT: u32 = 5;

/// The one sentence fragment naming the machine whose plans these are. The
/// monorepo proxy fronts exactly one daemon, so there is no device to name.
const MACHINE: &str = "the machine running this Ocean";

// ── wire shapes ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderRow {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub expires_at_ms: Option<f64>,
    #[serde(default)]
    pub login: Option<LoginAttempt>,
}

impl ProviderRow {
    /// The name shown for the row; falls back to the provider id rather than
    /// rendering an empty heading.
    pub fn display_label(&self) -> String {
        if !self.label.trim().is_empty() {
            self.label.clone()
        } else {
            match self.provider.as_str() {
                "claude" => "Claude".to_string(),
                "codex" => "Codex".to_string(),
                "chatgpt" => "ChatGPT".to_string(),
                other => other.to_string(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoginAttempt {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub attempt_id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProvidersResponse {
    #[serde(default)]
    providers: Vec<ProviderRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LoginStarted {
    #[serde(default)]
    attempt_id: String,
    #[serde(default)]
    authorize_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LogoutResult {
    #[serde(default)]
    removed: bool,
}

/// `{ok:false, error, reason?}` — every refusal on these routes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ApiError {
    #[serde(skip)]
    pub status: u16,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub reason: String,
}

impl ApiError {
    fn transport() -> Self {
        Self {
            error: "network".into(),
            ..Self::default()
        }
    }
}

// ── pure decisions ────────────────────────────────────────────────

/// A provider's login status, closed over the daemon's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    SignedIn,
    Expired,
    SignedOut,
    Unknown,
}

impl PlanStatus {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "signed_in" => Self::SignedIn,
            "expired" => Self::Expired,
            "signed_out" => Self::SignedOut,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SignedIn => "Signed in",
            Self::Expired => "Expired",
            Self::SignedOut => "Signed out",
            Self::Unknown => "Unknown",
        }
    }

    /// The `data-status` value the row carries: signed in is healthy,
    /// expired needs attention, and signed out is a neutral fact rather than
    /// a failure.
    pub fn tone(self) -> &'static str {
        match self {
            Self::SignedIn => "ok",
            Self::Expired => "degraded",
            Self::SignedOut | Self::Unknown => "unknown",
        }
    }

    /// The sign-in button's words, or `None` when there is nothing to sign in
    /// to (already signed in).
    pub fn sign_in_label(self) -> Option<&'static str> {
        match self {
            Self::SignedIn => None,
            Self::Expired => Some("Sign in again"),
            Self::SignedOut | Self::Unknown => Some("Sign in"),
        }
    }
}

/// Where a credential came from, in words — only when it is not the saved
/// sign-in this panel manages (the daemon's `source` vocabulary).
pub fn source_hint(source: Option<&str>) -> Option<&'static str> {
    match source {
        Some("env") => Some("via environment"),
        Some("codex_cli") => Some("via Codex CLI login"),
        Some("claude_cli") => Some("via Claude Code login"),
        Some("claude_keychain") => Some("via Claude Code keychain"),
        _ => None,
    }
}

/// May this row offer "Sign out"? Only when there is a credential, and not
/// when it comes from the machine's environment — removing a saved file
/// cannot unset an environment variable, so the button would lie.
pub fn can_sign_out(status: PlanStatus, source: Option<&str>) -> bool {
    matches!(status, PlanStatus::SignedIn | PlanStatus::Expired) && source != Some("env")
}

/// Expiry is only worth a line once it has passed: a live token refreshes
/// itself, and counting down to that would read as a warning it is not.
pub fn expiry_note(status: PlanStatus, expires_at_ms: Option<f64>, now_ms: f64) -> Option<String> {
    if status != PlanStatus::Expired {
        return None;
    }
    let at = expires_at_ms.filter(|at| at.is_finite() && *at > 0.0)?;
    let ago = ((now_ms - at) / 1000.0).max(0.0);
    let text = if ago < 60.0 {
        "expired just now".to_string()
    } else if ago < 3_600.0 {
        plural("expired", (ago / 60.0).floor(), "minute")
    } else if ago < 86_400.0 {
        plural("expired", (ago / 3_600.0).floor(), "hour")
    } else {
        plural("expired", (ago / 86_400.0).floor(), "day")
    };
    Some(text)
}

fn plural(prefix: &str, n: f64, unit: &str) -> String {
    let n = n as u64;
    let s = if n == 1 { "" } else { "s" };
    format!("{prefix} {n} {unit}{s} ago")
}

/// The same-machine caveat, stated once per attempt.
pub fn same_machine_caveat() -> String {
    format!(
        "Finish sign-in in a browser on {MACHINE} \u{2014} the provider sends you back to \
         that machine."
    )
}

/// Every refusal as a sentence.
pub fn error_message(error: &ApiError) -> String {
    match error.error.as_str() {
        "operator_credential_unavailable" | "operator_credential_missing" => format!(
            "Ocean can't manage sign-ins on {MACHINE} yet \u{2014} its operator key isn't set up \
             for this surface."
        ),
        "login_unavailable" => format!(
            "Sign-in isn't available for this provider on {MACHINE} right now. Try again in a \
             moment."
        ),
        "login_in_progress" => {
            "A sign-in for this provider is already in progress. Finish or cancel it first."
                .to_string()
        }
        "unknown_attempt" => "That sign-in attempt is no longer active. Start again.".to_string(),
        "unknown_provider" => format!("{MACHINE} doesn't support this provider."),
        "logout_failed" => format!("Ocean couldn't remove that sign-in from {MACHINE}. Try again."),
        "network" => "Couldn't reach Ocean. Check your connection and try again.".to_string(),
        _ => match error.status {
            401 => "Your Ocean session has ended. Reload the page and sign in again.".to_string(),
            403 => format!("This surface isn't allowed to manage sign-ins on {MACHINE}."),
            404 => {
                format!("{MACHINE} doesn't support coding plan sign-in yet. Update Ocean there.")
            }
            502 => format!("Ocean's daemon on {MACHINE} isn't answering. Try again."),
            _ => format!("Something went wrong talking to {MACHINE}. Try again."),
        },
    }
}

/// A sign-in attempt's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptState {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

impl AttemptState {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" | "canceled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(self) -> bool {
        self != Self::Pending
    }
}

/// The line shown once an attempt ends. `error` is the daemon's fixed
/// failure code for a failed attempt; it is never rendered verbatim.
pub fn attempt_outcome(label: &str, state: AttemptState, error: Option<&str>) -> Notice {
    match state {
        AttemptState::Succeeded => Notice::ok(format!("{label} is signed in.")),
        AttemptState::Cancelled => Notice::ok(format!("{label} sign-in cancelled.")),
        AttemptState::Pending => Notice::ok(String::new()),
        AttemptState::Failed => {
            let why = match error.unwrap_or_default() {
                "timeout" | "expired" => " It took too long \u{2014} start again.",
                "access_denied" | "denied" => " Access was declined in the browser.",
                "state_mismatch" | "invalid_state" => " The browser returned a stale response.",
                "exchange_failed" => " The provider refused to finish the sign-in.",
                "write_failed" => " Ocean couldn't save the sign-in on its machine.",
                _ => " Try again.",
            };
            Notice::err(format!("{label} sign-in didn't finish.{why}"))
        }
    }
}

/// A line under the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

impl Notice {
    fn ok(text: String) -> Self {
        Self {
            text,
            is_error: false,
        }
    }
    fn err(text: String) -> Self {
        Self {
            text,
            is_error: true,
        }
    }
}

/// The result line after a sign-out.
pub fn logout_outcome(label: &str, removed: bool) -> Notice {
    if removed {
        Notice::ok(format!("Signed out of {label} on {MACHINE}."))
    } else {
        Notice::ok(format!(
            "There was no saved {label} sign-in on {MACHINE} to remove."
        ))
    }
}

/// Path-segment encoding for ids the daemon minted; they are expected to be
/// URL-safe already, so anything else is percent-encoded rather than trusted.
fn encode_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// One of the five coding-plan routes. The web transport builds the method
/// and path from it; the native shell gets only its fixed `kind` and the raw
/// ids and builds its own (the same shapes, `crates/ocean-tauri/src/coding_plans.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PlanRoute {
    List,
    LoginStart { provider: String },
    LoginPoll { provider: String, attempt: String },
    LoginCancel { provider: String, attempt: String },
    Logout { provider: String },
}

impl PlanRoute {
    fn kind(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::LoginStart { .. } => "login_start",
            Self::LoginPoll { .. } => "login_poll",
            Self::LoginCancel { .. } => "login_cancel",
            Self::Logout { .. } => "logout",
        }
    }

    fn method(&self) -> &'static str {
        match self {
            Self::List | Self::LoginPoll { .. } => "GET",
            Self::LoginStart { .. } | Self::Logout { .. } => "POST",
            Self::LoginCancel { .. } => "DELETE",
        }
    }

    fn path(&self) -> String {
        match self {
            Self::List => "/v1/auth/providers".to_string(),
            Self::LoginStart { provider } => {
                format!("/v1/auth/providers/{}/login", encode_segment(provider))
            }
            Self::LoginPoll { provider, attempt } | Self::LoginCancel { provider, attempt } => {
                format!(
                    "/v1/auth/providers/{}/login/{}",
                    encode_segment(provider),
                    encode_segment(attempt)
                )
            }
            Self::Logout { provider } => {
                format!("/v1/auth/providers/{}/logout", encode_segment(provider))
            }
        }
    }

    /// The raw ids the native shell encodes into its own fixed path.
    fn ids(&self) -> (Option<&str>, Option<&str>) {
        match self {
            Self::List => (None, None),
            Self::LoginStart { provider } | Self::Logout { provider } => (Some(provider), None),
            Self::LoginPoll { provider, attempt } | Self::LoginCancel { provider, attempt } => {
                (Some(provider), Some(attempt))
            }
        }
    }
}

// ── reactive handle ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadPhase {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

/// A sign-in the panel is waiting on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAttempt {
    pub provider: String,
    pub label: String,
    pub attempt_id: String,
    /// `None` when the attempt was adopted from the list (started earlier,
    /// before the panel was reopened) — its URL is not re-served.
    pub authorize_url: Option<String>,
}

/// The panel's state, created once at app scope.
#[derive(Clone, Copy)]
pub struct CodingPlansState {
    pub open: RwSignal<bool>,
    pub providers: RwSignal<Vec<ProviderRow>>,
    pub phase: RwSignal<LoadPhase>,
    pub attempt: RwSignal<Option<ActiveAttempt>>,
    /// Provider with a request (login start / logout) in flight.
    pub busy: RwSignal<Option<String>>,
    /// Provider whose sign-out is armed and waiting for confirmation.
    pub confirm_logout: RwSignal<Option<String>>,
    pub notice: RwSignal<Option<Notice>>,
    /// Daemon base URL (same origin behind the proxy).
    base: RwSignal<String>,
    /// Retires poll loops and stale list replies.
    generation: RwSignal<u64>,
    /// Latest provider-list request. Every refresh takes a new ticket, so a
    /// slower earlier GET (say, one sent before a sign-out) can never land
    /// after a newer one and repaint rows the daemon has since changed.
    refresh_ticket: RwSignal<u64>,
    /// Attempt ids this panel stopped watching on its own bounds (time or
    /// failure limit). The list still reports them pending, and re-adopting
    /// one would restart the very loop the bound just ended.
    retired: RwSignal<Vec<String>>,
}

impl CodingPlansState {
    pub fn new(base: RwSignal<String>) -> Self {
        Self {
            open: RwSignal::new(false),
            providers: RwSignal::new(Vec::new()),
            phase: RwSignal::new(LoadPhase::Idle),
            attempt: RwSignal::new(None),
            busy: RwSignal::new(None),
            confirm_logout: RwSignal::new(None),
            notice: RwSignal::new(None),
            base,
            generation: RwSignal::new(0),
            refresh_ticket: RwSignal::new(0),
            retired: RwSignal::new(Vec::new()),
        }
    }

    /// Open the panel and read the list fresh. The only door in; the header
    /// overflow row and the palette command both call it.
    pub fn show(self) {
        self.notice.set(None);
        self.confirm_logout.set(None);
        self.open.set(true);
        self.refresh();
    }

    /// Close the panel and retire any poll loop. A pending attempt keeps
    /// running on the daemon; reopening the panel adopts it from the list.
    pub fn close(self) {
        self.bump();
        self.open.set(false);
        self.attempt.set(None);
        self.busy.set(None);
        self.confirm_logout.set(None);
    }

    fn bump(self) -> u64 {
        let next = self.generation.get_untracked().wrapping_add(1);
        self.generation.set(next);
        next
    }

    fn current(self, claimed: u64) -> bool {
        claimed == self.generation.get_untracked()
    }

    pub fn refresh(self) {
        let claimed = self.generation.get_untracked();
        let ticket = self.refresh_ticket.get_untracked().wrapping_add(1);
        self.refresh_ticket.set(ticket);
        let base = self.base.get_untracked();
        if self.providers.get_untracked().is_empty() {
            self.phase.set(LoadPhase::Loading);
        }
        spawn_local(async move {
            let result = fetch_providers(&base).await;
            if !self.current(claimed)
                || !self.open.get_untracked()
                || self.refresh_ticket.get_untracked() != ticket
            {
                return;
            }
            match result {
                Ok(rows) => {
                    let adopt = if self.attempt.get_untracked().is_none() {
                        rows.iter().find_map(|row| {
                            let login = row.login.as_ref()?;
                            (AttemptState::parse(&login.state) == AttemptState::Pending
                                && !login.attempt_id.is_empty()
                                && !self
                                    .retired
                                    .with_untracked(|ids| ids.contains(&login.attempt_id)))
                            .then(|| ActiveAttempt {
                                provider: row.provider.clone(),
                                label: row.display_label(),
                                attempt_id: login.attempt_id.clone(),
                                authorize_url: None,
                            })
                        })
                    } else {
                        None
                    };
                    self.providers.set(rows);
                    self.phase.set(LoadPhase::Ready);
                    if let Some(attempt) = adopt {
                        self.watch(attempt);
                    }
                }
                Err(error) => {
                    self.phase.set(LoadPhase::Failed(error_message(&error)));
                }
            }
        });
    }

    /// Start a sign-in. `popup` is a blank tab opened synchronously inside the
    /// click (so the browser does not block it); it is pointed at the
    /// provider once the daemon answers, or closed if it refuses.
    pub fn sign_in(self, provider: String, label: String, popup: Option<web_sys::Window>) {
        if self.busy.get_untracked().is_some() || self.attempt.get_untracked().is_some() {
            if let Some(popup) = popup {
                let _ = popup.close();
            }
            return;
        }
        self.notice.set(None);
        self.confirm_logout.set(None);
        self.busy.set(Some(provider.clone()));
        let base = self.base.get_untracked();
        let claimed = self.generation.get_untracked();
        spawn_local(async move {
            let result = post_login(&base, &provider).await;
            if !self.current(claimed) {
                if let Some(popup) = popup {
                    let _ = popup.close();
                }
                return;
            }
            self.busy.set(None);
            match result {
                Ok(started) => {
                    let url = started.authorize_url.clone();
                    let opened = match popup {
                        Some(popup) => popup.location().set_href(&url).is_ok(),
                        None => crate::host::open_external_url(&url).await,
                    };
                    if !opened {
                        log::debug!("sign-in tab not opened; the panel offers a link instead");
                    }
                    // The native open above awaits an IPC round trip; the panel
                    // may have closed meanwhile, and a watch installed now
                    // would strand a waiting state no poll ever clears.
                    if !self.current(claimed) || !self.open.get_untracked() {
                        return;
                    }
                    self.watch(ActiveAttempt {
                        provider,
                        label,
                        attempt_id: started.attempt_id,
                        authorize_url: (!url.is_empty()).then_some(url),
                    });
                }
                Err(error) => {
                    if let Some(popup) = popup {
                        let _ = popup.close();
                    }
                    self.notice.set(Some(Notice::err(error_message(&error))));
                }
            }
        });
    }

    /// Poll `attempt` until it ends, the panel closes, or another intent
    /// supersedes it.
    fn watch(self, attempt: ActiveAttempt) {
        let claimed = self.bump();
        self.attempt.set(Some(attempt.clone()));
        let base = self.base.get_untracked();
        spawn_local(async move {
            let route = PlanRoute::LoginPoll {
                provider: attempt.provider.clone(),
                attempt: attempt.attempt_id.clone(),
            };
            let mut failures = 0;
            for _ in 0..POLL_LIMIT {
                gloo_timers::future::TimeoutFuture::new(POLL_INTERVAL_MS).await;
                if !self.current(claimed) || !self.open.get_untracked() {
                    return;
                }
                let result = get_json::<LoginAttempt>(&base, &route).await;
                if !self.current(claimed) {
                    return;
                }
                match result {
                    Ok(status) => {
                        failures = 0;
                        let state = AttemptState::parse(&status.state);
                        if state.is_terminal() {
                            self.finish(attempt_outcome(
                                &attempt.label,
                                state,
                                status.error.as_deref(),
                            ));
                            return;
                        }
                    }
                    Err(error) if error.error == "unknown_attempt" || error.status == 404 => {
                        self.finish(Notice::err(error_message(&ApiError {
                            error: "unknown_attempt".into(),
                            ..error
                        })));
                        return;
                    }
                    Err(error) => {
                        failures += 1;
                        if failures >= POLL_FAILURE_LIMIT {
                            self.retire(&attempt.attempt_id);
                            self.finish(Notice::err(error_message(&error)));
                            return;
                        }
                    }
                }
            }
            if self.current(claimed) {
                self.retire(&attempt.attempt_id);
                self.finish(Notice::err(format!(
                    "Stopped waiting for {} sign-in. If you finished it, reopen this panel.",
                    attempt.label
                )));
            }
        });
    }

    fn retire(self, attempt_id: &str) {
        self.retired.update(|ids| {
            if !ids.iter().any(|id| id == attempt_id) {
                ids.push(attempt_id.to_string());
            }
        });
    }

    fn finish(self, notice: Notice) {
        self.bump();
        self.attempt.set(None);
        if !notice.text.is_empty() {
            self.notice.set(Some(notice));
        }
        self.refresh();
    }

    /// Cancel the attempt being watched. Best effort on the wire: the local
    /// wait ends either way.
    pub fn cancel(self) {
        let Some(attempt) = self.attempt.get_untracked() else {
            return;
        };
        self.bump();
        self.attempt.set(None);
        let base = self.base.get_untracked();
        let claimed = self.generation.get_untracked();
        spawn_local(async move {
            let route = PlanRoute::LoginCancel {
                provider: attempt.provider.clone(),
                attempt: attempt.attempt_id.clone(),
            };
            let result = send(&base, &route).await;
            if !self.current(claimed) {
                return;
            }
            let notice = match result {
                Ok(()) => attempt_outcome(&attempt.label, AttemptState::Cancelled, None),
                Err(error) => {
                    // The daemon still holds the attempt. Never claim a
                    // cancellation that did not happen, and never re-adopt the
                    // attempt this click asked to stop watching.
                    self.retire(&attempt.attempt_id);
                    Notice::err(format!(
                        "Couldn't cancel {} sign-in. {}",
                        attempt.label,
                        error_message(&error)
                    ))
                }
            };
            self.notice.set(Some(notice));
            self.refresh();
        });
    }

    pub fn sign_out(self, provider: String, label: String) {
        if self.busy.get_untracked().is_some() {
            return;
        }
        self.confirm_logout.set(None);
        self.notice.set(None);
        self.busy.set(Some(provider.clone()));
        let base = self.base.get_untracked();
        let claimed = self.generation.get_untracked();
        spawn_local(async move {
            let route = PlanRoute::Logout { provider };
            let result = get_json::<LogoutResult>(&base, &route).await;
            if !self.current(claimed) {
                return;
            }
            self.busy.set(None);
            let notice = match result {
                Ok(done) => logout_outcome(&label, done.removed),
                Err(error) => Notice::err(error_message(&error)),
            };
            self.notice.set(Some(notice));
            self.refresh();
        });
    }
}

// ── transport ─────────────────────────────────────────────────────

/// The ONE seam every coding-plan request takes, like the Room owner
/// mutations': in the Tauri shell it goes through `host::coding_plan_request`,
/// which attaches the operator key natively against its own fixed routes; in
/// the browser it is a same-origin fetch the Surface proxy attaches the key
/// to. The extension has neither and never shows the panel (`supported`).
async fn call(base: &str, route: &PlanRoute) -> Result<(u16, String), ApiError> {
    let (provider, attempt) = route.ids();
    if let Some(reply) = crate::host::coding_plan_request(route.kind(), provider, attempt).await {
        return reply
            .map(|reply| (reply.status, reply.body))
            .map_err(|_| ApiError::transport());
    }
    let url = format!("{}{}", base.trim_end_matches('/'), route.path());
    let request = match route.method() {
        "POST" => gloo_net::http::Request::post(&url),
        "DELETE" => gloo_net::http::Request::delete(&url),
        _ => gloo_net::http::Request::get(&url),
    };
    let response = request.send().await.map_err(|_| ApiError::transport())?;
    let status = response.status();
    let body = response.text().await.map_err(|_| ApiError::transport())?;
    Ok((status, body))
}

/// Decode a reply: a non-2xx is the daemon's `{error, …}` body with its status.
fn decode<T: serde::de::DeserializeOwned>(status: u16, body: &str) -> Result<T, ApiError> {
    if !(200..300).contains(&status) {
        let mut error = serde_json::from_str::<ApiError>(body).unwrap_or_default();
        error.status = status;
        return Err(error);
    }
    serde_json::from_str::<T>(body).map_err(|_| ApiError {
        status,
        error: "unreadable".into(),
        ..ApiError::default()
    })
}

async fn get_json<T: serde::de::DeserializeOwned>(
    base: &str,
    route: &PlanRoute,
) -> Result<T, ApiError> {
    let (status, body) = call(base, route).await?;
    decode(status, &body)
}

async fn send(base: &str, route: &PlanRoute) -> Result<(), ApiError> {
    get_json::<serde_json::Value>(base, route).await.map(|_| ())
}

async fn fetch_providers(base: &str) -> Result<Vec<ProviderRow>, ApiError> {
    get_json::<ProvidersResponse>(base, &PlanRoute::List)
        .await
        .map(|body| body.providers)
}

async fn post_login(base: &str, provider: &str) -> Result<LoginStarted, ApiError> {
    let route = PlanRoute::LoginStart {
        provider: provider.to_string(),
    };
    get_json::<LoginStarted>(base, &route).await
}

/// Whether this host can reach the coding-plan routes at all: the browser PWA
/// (the proxy attaches the key) and the Tauri shell (it does, natively). The
/// Chrome extension has no privileged transport, so it never offers the panel.
pub fn supported() -> bool {
    !crate::daemon::running_as_extension()
}

/// Open a blank tab synchronously, inside the click, so a popup blocker lets
/// it through; the sign-in URL is assigned once the daemon answers. Returns
/// `None` in the native shell (the OS browser opens instead) and when the
/// browser refuses anyway (the panel then offers a link).
fn open_blank_tab() -> Option<web_sys::Window> {
    if crate::host::running_in_tauri() {
        return None;
    }
    let popup = web_sys::window()?
        .open_with_url_and_target("", "_blank")
        .ok()
        .flatten()?;
    // Sever the back-reference before the provider's page loads in it.
    let _ = popup.set_opener(&JsValue::NULL);
    Some(popup)
}

// ── view ──────────────────────────────────────────────────────────

/// The panel. A self-contained dialog that consumes its own Escape rather
/// than joining the reveal rail's close-exactly-one chain.
#[component]
pub fn CodingPlansPanel(state: CodingPlansState) -> impl IntoView {
    let panel: NodeRef<leptos::html::Div> = NodeRef::new();
    Effect::new(move |_| {
        if state.open.get() {
            if let Some(element) = panel.get() {
                let _ = element.focus();
            }
        }
    });
    view! {
        <div
            class="coding-plans-overlay"
            hidden=move || !state.open.get()
            on:click=move |_| state.close()
        >
            <div
                class="coding-plans"
                node_ref=panel
                role="dialog"
                aria-modal="true"
                aria-labelledby="coding-plans-title"
                tabindex="-1"
                on:click=|event| event.stop_propagation()
                on:keydown=move |event| {
                    if event.key() == "Escape" {
                        event.stop_propagation();
                        state.close();
                    }
                }
            >
                <header class="coding-plans__head">
                    <h2 class="coding-plans__title" id="coding-plans-title">"Coding plans"</h2>
                    <button
                        class="sessions-panel__close"
                        type="button"
                        aria-label="Close coding plans"
                        on:click=move |_| state.close()
                    >
                        "✕"
                    </button>
                </header>
                <p class="coding-plans__note">
                    {format!(
                        "Claude and Codex subscriptions signed in on {MACHINE}. Agents on that machine use them."
                    )}
                </p>
                <Show when=move || matches!(state.phase.get(), LoadPhase::Loading)>
                    <p class="coding-plans__note" role="status">"Checking sign-ins\u{2026}"</p>
                </Show>
                <Show when=move || matches!(state.phase.get(), LoadPhase::Failed(_))>
                    <p class="coding-plans__error" role="alert">
                        {move || match state.phase.get() {
                            LoadPhase::Failed(message) => message,
                            _ => String::new(),
                        }}
                    </p>
                </Show>
                <Show when=move || state.attempt.get().is_some()>
                    <WaitingForBrowser state=state />
                </Show>
                <ul class="coding-plans__list">
                    <For
                        each=move || state.providers.get()
                        key=|row| format!("{row:?}")
                        let:row
                    >
                        <PlanRow state=state row=row />
                    </For>
                </ul>
                <Show when=move || state.notice.get().is_some()>
                    {move || {
                        let notice = state.notice.get()?;
                        Some(if notice.is_error {
                            view! {
                                <p class="coding-plans__error" role="alert">{notice.text}</p>
                            }
                                .into_any()
                        } else {
                            view! {
                                <p class="coding-plans__note coding-plans__notice" role="status">
                                    {notice.text}
                                </p>
                            }
                                .into_any()
                        })
                    }}
                </Show>
            </div>
        </div>
    }
}

/// The wait while a sign-in is open in another tab.
#[component]
fn WaitingForBrowser(state: CodingPlansState) -> impl IntoView {
    let attempt = move || state.attempt.get();
    view! {
        <div class="coding-plans__waiting" role="status" aria-live="polite">
            <p class="coding-plans__note coding-plans__waiting-line">
                <strong>
                    {move || {
                        attempt()
                            .map(|a| format!("{}: waiting for you to finish in the browser\u{2026}", a.label))
                            .unwrap_or_default()
                    }}
                </strong>
            </p>
            <p class="coding-plans__note coding-plans__waiting-line">{same_machine_caveat()}</p>
            <div class="coding-plans__actions">
                <Show when=move || attempt().and_then(|a| a.authorize_url).is_some()>
                    <a
                        class="swipedeck-btn coding-plans__link"
                        href=move || attempt().and_then(|a| a.authorize_url).unwrap_or_default()
                        target="_blank"
                        rel="noopener noreferrer"
                        on:click=crate::host::open_external_link_click
                    >
                        "Open sign-in page"
                    </a>
                </Show>
                <button
                    class="swipedeck-btn swipedeck-btn--danger"
                    type="button"
                    on:click=move |_| state.cancel()
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
}

/// One provider.
#[component]
fn PlanRow(state: CodingPlansState, row: ProviderRow) -> impl IntoView {
    let status = PlanStatus::parse(&row.status);
    let label = row.display_label();
    let provider = row.provider.clone();
    let source = row.source.clone();
    let hint = source_hint(source.as_deref());
    let expiry = expiry_note(status, row.expires_at_ms, js_sys::Date::now());
    let detail = [hint.map(str::to_string), expiry]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{00B7} ");
    let sign_in_text = status.sign_in_label();
    let sign_out_allowed = can_sign_out(status, source.as_deref());

    let busy = {
        let provider = provider.clone();
        move || state.busy.get().as_deref() == Some(provider.as_str())
    };
    let blocked = move || state.busy.get().is_some() || state.attempt.get().is_some();
    let armed = {
        let provider = provider.clone();
        move || state.confirm_logout.get().as_deref() == Some(provider.as_str())
    };

    let sign_in = {
        let provider = provider.clone();
        let label = label.clone();
        move |_| state.sign_in(provider.clone(), label.clone(), open_blank_tab())
    };
    let arm_logout = {
        let provider = provider.clone();
        move |_| state.confirm_logout.set(Some(provider.clone()))
    };
    let confirm_logout = {
        let provider = provider.clone();
        let label = label.clone();
        move |_| state.sign_out(provider.clone(), label.clone())
    };
    let confirm_text =
        format!("Sign out of {label}? This removes the saved sign-in from {MACHINE}.");
    let busy_for_in = busy.clone();
    let label_for_actions = label.clone();

    view! {
        <li class="coding-plans__row" data-provider=provider.clone()>
            <div class="coding-plans__summary" data-status=status.tone()>
                <span class="coding-plans__dot" aria-hidden="true"></span>
                <span class="coding-plans__name">{label.clone()}</span>
                <span class="coding-plans__detail">{detail}</span>
                <span class="coding-plans__status" data-status=status.tone()>
                    {status.label()}
                </span>
            </div>
            <Show
                when=armed.clone()
                fallback=move || {
                    view! {
                        <div class="coding-plans__actions">
                            {sign_in_text
                                .map(|text| {
                                    let busy = busy_for_in.clone();
                                    view! {
                                        <button
                                            class="swipedeck-btn coding-plans__sign-in"
                                            type="button"
                                            disabled=blocked
                                            aria-label=format!("{text} to {}", label_for_actions.clone())
                                            on:click=sign_in.clone()
                                        >
                                            {move || if busy() { "Starting\u{2026}" } else { text }}
                                        </button>
                                    }
                                })}
                            {sign_out_allowed
                                .then(|| {
                                    view! {
                                        <button
                                            class="swipedeck-btn coding-plans__sign-out"
                                            type="button"
                                            disabled=blocked
                                            on:click=arm_logout.clone()
                                        >
                                            "Sign out"
                                        </button>
                                    }
                                })}
                        </div>
                    }
                }
            >
                <div class="coding-plans__confirm" role="group" aria-label="Confirm sign out">
                    <p class="coding-plans__note coding-plans__waiting-line">{confirm_text.clone()}</p>
                    <div class="coding-plans__actions">
                        <button
                            class="swipedeck-btn"
                            type="button"
                            on:click=move |_| state.confirm_logout.set(None)
                        >
                            "Keep"
                        </button>
                        <button
                            class="swipedeck-btn swipedeck-btn--danger coding-plans__confirm-sign-out"
                            type="button"
                            disabled=busy.clone()
                            on:click=confirm_logout.clone()
                        >
                            {
                                let busy = busy.clone();
                                move || if busy() { "Signing out\u{2026}" } else { "Sign out" }
                            }
                        </button>
                    </div>
                </div>
            </Show>
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_as_words_and_an_unknown_one_is_neutral() {
        assert_eq!(PlanStatus::parse("signed_in").label(), "Signed in");
        assert_eq!(PlanStatus::parse("expired").label(), "Expired");
        assert_eq!(PlanStatus::parse("signed_out").label(), "Signed out");
        assert_eq!(PlanStatus::parse("unknown").label(), "Unknown");
        assert_eq!(PlanStatus::parse("rotating").label(), "Unknown");
        assert_eq!(PlanStatus::parse("signed_in").tone(), "ok");
        assert_eq!(PlanStatus::parse("expired").tone(), "degraded");
        // Signed out is a fact, not a failure: it must not borrow red.
        assert_eq!(PlanStatus::parse("signed_out").tone(), "unknown");
    }

    #[test]
    fn the_sign_in_button_says_again_only_when_expired() {
        assert_eq!(PlanStatus::SignedIn.sign_in_label(), None);
        assert_eq!(PlanStatus::Expired.sign_in_label(), Some("Sign in again"));
        assert_eq!(PlanStatus::SignedOut.sign_in_label(), Some("Sign in"));
        assert_eq!(PlanStatus::Unknown.sign_in_label(), Some("Sign in"));
    }

    #[test]
    fn source_is_hinted_only_when_it_is_not_the_saved_sign_in() {
        assert_eq!(source_hint(Some("env")), Some("via environment"));
        assert_eq!(source_hint(Some("codex_cli")), Some("via Codex CLI login"));
        assert_eq!(
            source_hint(Some("claude_cli")),
            Some("via Claude Code login")
        );
        assert_eq!(
            source_hint(Some("claude_keychain")),
            Some("via Claude Code keychain")
        );
        assert_eq!(source_hint(Some("auth_file")), None);
        assert_eq!(source_hint(None), None);
        assert_eq!(source_hint(Some("keychain")), None);
    }

    #[test]
    fn sign_out_is_offered_only_for_a_removable_credential() {
        assert!(can_sign_out(PlanStatus::SignedIn, Some("auth_file")));
        assert!(can_sign_out(PlanStatus::Expired, None));
        assert!(can_sign_out(PlanStatus::SignedIn, Some("codex_cli")));
        // An environment variable cannot be removed by deleting a file.
        assert!(!can_sign_out(PlanStatus::SignedIn, Some("env")));
        assert!(!can_sign_out(PlanStatus::SignedOut, None));
        assert!(!can_sign_out(PlanStatus::Unknown, None));
    }

    #[test]
    fn expiry_is_mentioned_only_once_it_has_passed() {
        let now = 10_000_000_000.0;
        assert_eq!(
            expiry_note(PlanStatus::SignedIn, Some(now + 60_000.0), now),
            None
        );
        assert_eq!(expiry_note(PlanStatus::Expired, None, now), None);
        assert_eq!(
            expiry_note(PlanStatus::Expired, Some(now - 10_000.0), now).as_deref(),
            Some("expired just now")
        );
        assert_eq!(
            expiry_note(PlanStatus::Expired, Some(now - 60_000.0), now).as_deref(),
            Some("expired 1 minute ago")
        );
        assert_eq!(
            expiry_note(PlanStatus::Expired, Some(now - 3.0 * 3_600_000.0), now).as_deref(),
            Some("expired 3 hours ago")
        );
        assert_eq!(
            expiry_note(PlanStatus::Expired, Some(now - 2.0 * 86_400_000.0), now).as_deref(),
            Some("expired 2 days ago")
        );
        // A clock skewed into the future reads as just now, not negative.
        assert_eq!(
            expiry_note(PlanStatus::Expired, Some(now + 5_000.0), now).as_deref(),
            Some("expired just now")
        );
    }

    fn api(status: u16, error: &str) -> ApiError {
        ApiError {
            status,
            error: error.into(),
            ..ApiError::default()
        }
    }

    #[test]
    fn every_error_code_is_a_sentence_and_never_the_code() {
        let codes = [
            (409, "login_unavailable"),
            (503, "operator_credential_unavailable"),
            (503, "operator_credential_missing"),
            (404, "unknown_attempt"),
            (404, "unknown_provider"),
            (409, "login_in_progress"),
            (500, "logout_failed"),
            (403, "ambient_credential_rejected"),
            (403, "foreign_origin_rejected"),
            (0, "network"),
            (500, "internal"),
            (401, ""),
            (403, "forbidden"),
            (404, ""),
            (502, ""),
            (418, "teapot_mode"),
        ];
        for (status, code) in codes {
            let text = error_message(&api(status, code));
            assert!(text.ends_with('.'), "{code}: {text}");
            assert!(!text.contains('{'), "{code}: {text}");
            if !code.is_empty() {
                assert!(!text.contains(code), "{code} leaked: {text}");
            }
        }
        // The two "no key" codes (proxy-local and daemon-side) read the same.
        assert_eq!(
            error_message(&api(503, "operator_credential_unavailable")),
            error_message(&api(503, "operator_credential_missing"))
        );
        assert!(error_message(&api(409, "login_unavailable")).contains(MACHINE));
        assert!(error_message(&api(502, "")).contains("isn't answering"));
    }

    #[test]
    fn the_caveat_names_the_machine() {
        assert_eq!(
            same_machine_caveat(),
            "Finish sign-in in a browser on the machine running this Ocean \u{2014} the \
             provider sends you back to that machine."
        );
    }

    #[test]
    fn attempt_states_end_where_the_daemon_says_they_end() {
        assert!(!AttemptState::parse("pending").is_terminal());
        assert!(!AttemptState::parse("").is_terminal());
        assert!(AttemptState::parse("succeeded").is_terminal());
        assert!(AttemptState::parse("failed").is_terminal());
        assert!(AttemptState::parse("cancelled").is_terminal());
        let ok = attempt_outcome("Claude", AttemptState::Succeeded, None);
        assert!(!ok.is_error);
        assert_eq!(ok.text, "Claude is signed in.");
        let failed = attempt_outcome("Codex", AttemptState::Failed, Some("weird_internal_code"));
        assert!(failed.is_error);
        assert!(!failed.text.contains("weird_internal_code"));
        // The daemon's fixed failure classifications each get their own why.
        for code in [
            "timeout",
            "denied",
            "state_mismatch",
            "exchange_failed",
            "write_failed",
            "login_failed",
        ] {
            let notice = attempt_outcome("Claude", AttemptState::Failed, Some(code));
            assert!(notice.is_error);
            assert!(notice.text.starts_with("Claude sign-in didn't finish."));
            assert!(
                !notice.text.contains(code),
                "{code} leaked: {}",
                notice.text
            );
        }
    }

    #[test]
    fn sign_out_result_is_honest_about_nothing_to_remove() {
        let removed = logout_outcome("Claude", true);
        assert_eq!(
            removed.text,
            "Signed out of Claude on the machine running this Ocean."
        );
        let nothing = logout_outcome("Claude", false);
        assert!(nothing.text.contains("no saved Claude sign-in"));
        assert!(!nothing.is_error);
    }

    #[test]
    fn the_provider_list_decodes_the_daemons_shape() {
        let body: ProvidersResponse = serde_json::from_str(
            r#"{"ok":true,"providers":[
                {"provider":"claude","label":"Claude (Pro/Max plan)","kind":"oauth","status":"signed_in",
                 "source":"auth_file","expires_at_ms":1790000000000,
                 "stored":{"present":true,"refreshable":true,"expires_at_ms":1790000000000},
                 "login":null},
                {"provider":"codex","label":"Codex OAuth","kind":"oauth","status":"signed_out",
                 "source":null,"expires_at_ms":null,"stored":{"present":false,"refreshable":false,"expires_at_ms":null},
                 "login":{"provider":"codex","attempt_id":"a1","state":"pending"}},
                {"provider":"chatgpt","label":"ChatGPT plan","kind":"oauth","status":"unknown",
                 "source":null,"expires_at_ms":null,"stored":{"error":"auth_file_unreadable"},"login":null}
            ]}"#,
        )
        .expect("providers");
        assert_eq!(body.providers.len(), 3);
        assert_eq!(body.providers[0].source.as_deref(), Some("auth_file"));
        assert_eq!(body.providers[0].expires_at_ms, Some(1_790_000_000_000.0));
        assert!(body.providers[0].login.is_none());
        let login = body.providers[1].login.as_ref().expect("login");
        assert_eq!(login.attempt_id, "a1");
        assert_eq!(AttemptState::parse(&login.state), AttemptState::Pending);
        assert_eq!(
            PlanStatus::parse(&body.providers[2].status),
            PlanStatus::Unknown
        );

        // A sparse row still has a heading.
        let sparse: ProviderRow = serde_json::from_str(r#"{"provider":"codex"}"#).expect("row");
        assert_eq!(sparse.display_label(), "Codex");
        assert_eq!(PlanStatus::parse(&sparse.status), PlanStatus::Unknown);
        let chatgpt: ProviderRow = serde_json::from_str(r#"{"provider":"chatgpt"}"#).expect("row");
        assert_eq!(chatgpt.display_label(), "ChatGPT");
    }

    #[test]
    fn login_start_and_error_bodies_decode() {
        let started: LoginStarted = serde_json::from_str(
            r#"{"ok":true,"provider":"claude","attempt_id":"att_1","state":"pending",
                "authorize_url":"https://claude.ai/oauth/authorize?x=1","same_machine_required":true}"#,
        )
        .expect("started");
        assert_eq!(started.attempt_id, "att_1");
        assert!(started.authorize_url.starts_with("https://"));
        let error: ApiError =
            serde_json::from_str(r#"{"ok":false,"error":"login_unavailable"}"#).expect("error");
        assert_eq!(error.error, "login_unavailable");
        let logout: LogoutResult = serde_json::from_str(
            r#"{"ok":true,"provider":"codex","removed":true,"remote_revocation":"confirmed","message":"x"}"#,
        )
        .expect("logout");
        assert!(logout.removed);
        // The proxy's plain-text 502 decodes as an error carrying its status.
        let unreachable = decode::<LogoutResult>(502, "daemon unreachable").unwrap_err();
        assert_eq!(unreachable.status, 502);
        assert!(unreachable.error.is_empty());
    }

    /// Closing the panel retires everything in flight and forgets the
    /// attempt, so a reply that lands later cannot repaint a closed panel.
    #[test]
    fn closing_retires_in_flight_work() {
        let state = CodingPlansState::new(RwSignal::new(String::new()));
        state.open.set(true);
        state.busy.set(Some("claude".into()));
        state.confirm_logout.set(Some("claude".into()));
        state.attempt.set(Some(ActiveAttempt {
            provider: "claude".into(),
            label: "Claude".into(),
            attempt_id: "a1".into(),
            authorize_url: None,
        }));
        let before = state.generation.get_untracked();
        state.close();
        assert!(!state.open.get_untracked());
        assert!(state.attempt.get_untracked().is_none());
        assert!(state.busy.get_untracked().is_none());
        assert!(state.confirm_logout.get_untracked().is_none());
        assert!(!state.current(before), "in-flight replies are retired");
        // A retired attempt id is remembered and never re-adopted.
        state.retire("a1");
        state.retire("a1");
        assert_eq!(state.retired.get_untracked(), vec!["a1".to_string()]);
    }

    #[test]
    fn routes_are_the_five_contract_shapes_with_ids_encoded() {
        let list = PlanRoute::List;
        assert_eq!((list.kind(), list.method()), ("list", "GET"));
        assert_eq!(list.path(), "/v1/auth/providers");
        assert_eq!(list.ids(), (None, None));
        let start = PlanRoute::LoginStart {
            provider: "claude".into(),
        };
        assert_eq!((start.kind(), start.method()), ("login_start", "POST"));
        assert_eq!(start.path(), "/v1/auth/providers/claude/login");
        assert_eq!(start.ids(), (Some("claude"), None));
        let poll = PlanRoute::LoginPoll {
            provider: "co dex".into(),
            attempt: "a/b".into(),
        };
        assert_eq!((poll.kind(), poll.method()), ("login_poll", "GET"));
        assert_eq!(poll.path(), "/v1/auth/providers/co%20dex/login/a%2Fb");
        assert_eq!(poll.ids(), (Some("co dex"), Some("a/b")));
        let cancel = PlanRoute::LoginCancel {
            provider: "claude".into(),
            attempt: "a1".into(),
        };
        assert_eq!((cancel.kind(), cancel.method()), ("login_cancel", "DELETE"));
        assert_eq!(cancel.path(), "/v1/auth/providers/claude/login/a1");
        let logout = PlanRoute::Logout {
            provider: "codex".into(),
        };
        assert_eq!((logout.kind(), logout.method()), ("logout", "POST"));
        assert_eq!(logout.path(), "/v1/auth/providers/codex/logout");
        assert_eq!(logout.ids(), (Some("codex"), None));
    }
}
