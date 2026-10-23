//! Closing a room — the member-facing half of the daemon's close route
//! (Rooms DoD 4.3). Ported from Risingtides-dev/ocean-surface#229 against the
//! monorepo daemon's `persistent_rooms::room_close`.
//!
//!   POST /v1/rooms/persistent/{key}/close?actor_id=<member>
//!     → 200 `{ok:true, room, closed:true, marker_seq}`
//!
//! Five facts about that route shape everything below.
//!
//! 1. **Two authorities, and this surface uses exactly one.** The daemon takes
//!    either `?actor_id=` naming a roster member (the MEMBER lane) or an
//!    `X-Ocean-Operator` credential (the OPERATOR lane, deliberately not
//!    roster-checked), and the header's PRESENCE picks the lane. The operator
//!    key never enters a WASM bundle and the proxy injects it only on the
//!    owner-mutation routes, so every host — browser PWA, extension, Tauri —
//!    closes through the member lane with no operator header. Wiring the
//!    operator lane here would let anyone signed in to the proxy close a room
//!    they are not in, which is precisely what the daemon's member lane
//!    refuses.
//! 2. **Who the daemon lets close.** In the member lane: `actor_id` must be
//!    non-empty after trimming (`400 invalid_request`), the id must NOT name
//!    an `Agent` or `System` participant (`403 forged_closer`), the id must be
//!    ON the room's local participant roster, checked inside the closing
//!    transaction (`403`, `RoomCloserNotInRoster`, no code), and the room
//!    must be open (`404`, the store's `UnknownRoom`). `Human`, `Bot` and
//!    `Tool` rows all pass. [`close_offered`] is that rule over
//!    `Room.participants` — which the daemon serves from the same participants
//!    table its transaction reads — and the menu item does not render for
//!    anyone it answers `false` for. The daemon stays the authority; this copy
//!    only stops the surface offering an action the daemon would refuse.
//! 3. **A second close is a 404, not a silent success.** `close_with_marker`
//!    refuses a room that is not open with the same `UnknownRoom` every other
//!    write to a closed room gets, which this daemon answers as a JSON
//!    `404 {ok:false, error:"unknown room: …"}` — it does NOT wrap the close
//!    route's 404 with the `room_not_open` marker the read routes carry. A
//!    daemon or proxy that predates the route answers axum's EMPTY 404 (or a
//!    405). [`classify_close`] tells the two apart by whether the body
//!    decodes: a close that raced another member's, or a retry after a cut
//!    response whose close DID land, reads as [`CloseOutcome::AlreadyClosed`]
//!    and lands where a clean close does — the room is closed either way, and
//!    saying "failed" would be a lie.
//! 4. **Afterwards the room is frozen, not gone.** `GET .../{key}` 404s, every
//!    write refuses, the room's tail ends, and the room leaves
//!    `GET /v1/rooms/persistent`. This surface has no soft-closed audit view
//!    yet (the daemon's `/snapshot` is not consumed), so a close lands back on
//!    the rail: [`Rooms::close_room`] retires the live tail and clears room
//!    state through the one shared reset path, and [`Rooms::fetch_rooms`]
//!    drops the row.
//! 5. **Closing is local.** Nothing is sent to Bedrock: a federated room's
//!    credential, outbox and access projection are untouched. Members on other
//!    nodes keep their copy, so the confirmation says so for any room whose
//!    access state is not `Local`.
//!
//! Everything that decides what the operator sees is a free function below,
//! unit-tested natively.

use gloo_net::http::Request;
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::rooms::{
    encode, Room, RoomAccessProjection, RoomAccessState, RoomParticipantKind, Rooms,
};

// ---- Eligibility ------------------------------------------------------------

/// Should the close item render for this viewer in this room?
///
/// The daemon's member-lane rule, point for point (module doc, fact 2):
///
/// - the viewer's identity is loaded (`GET /v1/me` answered) and non-empty
///   after trimming — the same bar join and post clear, because acting under
///   an unresolved identity is how ghost members were made;
/// - that trimmed id is on the room's participant roster;
/// - and its row is not an `Agent` or `System` participant.
///
/// An open room is the only kind this surface can hold (a closed one 404s on
/// open), so there is no `closed` input to check.
pub(crate) fn close_offered(identity_id: &str, room: Option<&Room>) -> bool {
    let actor = identity_id.trim();
    if actor.is_empty() {
        return false;
    }
    let Some(room) = room else {
        return false;
    };
    room.participants
        .iter()
        .find(|participant| participant.id == actor)
        .is_some_and(|participant| {
            !matches!(
                participant.kind,
                RoomParticipantKind::Agent | RoomParticipantKind::System
            )
        })
}

/// The dispatch-time re-check: the same rule as [`close_offered`], asked again
/// at the moment `fire` would send, because identity or roster can move
/// between opening the dialog and pressing it. `Err` is the sentence shown
/// instead of sending.
pub(crate) fn dispatch_check(identity_id: &str, room: Option<&Room>) -> Result<(), String> {
    if close_offered(identity_id, room) {
        Ok(())
    } else {
        Err(
            "You can no longer close this room from here \u{2014} you are not on its roster \
             as a person, or it has already closed."
                .to_string(),
        )
    }
}

/// Where Tab goes inside the modal. `Keep` is first, `Fire` is last; anything
/// else focused inside the dialog (the dialog itself) counts as neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogFocus {
    Keep,
    Fire,
    Other,
}

/// What a Tab press inside the modal does. `None` lets the browser move focus
/// natively (it is already between the two ends); `Some` is a move this code
/// makes after preventing the default, so focus never leaves an
/// `aria-modal` dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrapMove {
    /// Focus the first control (`Keep room open`).
    First,
    /// Focus the last control (`Close room`).
    Last,
    /// Both controls are disabled while a close is in flight; hold focus on
    /// the dialog itself rather than letting it fall to `<body>`.
    Hold,
}

pub(crate) fn tab_trap(shift: bool, focused: DialogFocus, in_flight: bool) -> Option<TrapMove> {
    if in_flight {
        return Some(TrapMove::Hold);
    }
    match (shift, focused) {
        (true, DialogFocus::Keep | DialogFocus::Other) => Some(TrapMove::Last),
        (false, DialogFocus::Fire | DialogFocus::Other) => Some(TrapMove::First),
        _ => None,
    }
}

/// Whether closing leaves copies of this room elsewhere. Any access state but
/// `Local` means the room is federated, and the daemon's close is a LOCAL
/// statement that tells Bedrock nothing. `None` (no projection yet) is not
/// assumed federated: the note is a claim, and a claim needs an answer behind
/// it.
pub(crate) fn close_is_local_only(access: Option<&RoomAccessProjection>) -> bool {
    access.is_some_and(|access| access.state != RoomAccessState::Local)
}

/// The member-lane close URL. Both the key and the actor are encoded as one
/// path segment / one query value each: an id carrying `&` or `#` would
/// otherwise split the query and close as somebody else, or as nobody.
pub(crate) fn room_close_url(base: &str, key: &str, actor_id: &str) -> String {
    format!(
        "{}/v1/rooms/persistent/{}/close?actor_id={}",
        base.trim_end_matches('/'),
        encode(key),
        encode(actor_id.trim()),
    )
}

// ---- Reply ------------------------------------------------------------------

/// Every field the close route or its refusals can carry. All optional: a
/// refusal body has no `closed`, a success has no `code`, and an older
/// daemon's bare 404 has no body at all (`None` at the call site).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CloseReply {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    closed: bool,
    /// The marker the daemon's READ routes put on a not-open 404. Its close
    /// route does not carry it today; honoured if a later one does.
    #[serde(default)]
    room_not_open: bool,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// What a close request came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloseOutcome {
    /// The daemon closed the room and said so.
    Closed,
    /// The daemon says the room is not open — somebody closed it first, or a
    /// cut response hid that this screen's own close landed. Either way the
    /// room is closed and the rail is the honest destination.
    AlreadyClosed,
    /// Refused or failed; the sentence is what the operator reads.
    Refused(String),
}

/// Map a close reply onto what the dialog shows. Success is settled first, on
/// the status AND the body's own `closed: true`, so a 200 from something that
/// is not the close route cannot read as a closed room.
pub(crate) fn classify_close(status: u16, body: Option<CloseReply>) -> CloseOutcome {
    let Some(body) = body else {
        // No JSON at all. A bare 404/405 is axum's own answer for a route
        // the daemon (or the proxy in front of it) does not have; anything
        // else is unreadable.
        return match status {
            404 | 405 => CloseOutcome::Refused(route_absent_sentence()),
            502 | 504 => CloseOutcome::Refused(uncertain_sentence(&format!("HTTP {status}"))),
            _ => CloseOutcome::Refused(format!(
                "The close reply could not be read (HTTP {status}). The room may or may not \
                 have closed \u{2014} try again: a room that already closed says so."
            )),
        };
    };
    if (200..300).contains(&status) && body.ok && body.closed {
        return CloseOutcome::Closed;
    }
    // A hop between here and the daemon lost the answer. The proxy turns ANY
    // upstream reqwest error into a plain-text `502 daemon unreachable` —
    // including its forward timeout and a reset after the daemon had already
    // committed the close — and a gateway in front of it answers 502/504 for
    // the same class. None of those says the close did not happen.
    if status == 502 || status == 504 {
        return CloseOutcome::Refused(uncertain_sentence(&format!("HTTP {status}")));
    }
    // The daemon's not-open answer: its close route maps the store's
    // `UnknownRoom` to a JSON 404 with `ok:false` and no marker (module doc,
    // fact 3). Status AND a refusal body: `room_not_open` on anything but a
    // 404 is not the not-open answer, whatever else carried it.
    if status == 404 && (body.room_not_open || !body.ok) {
        return CloseOutcome::AlreadyClosed;
    }
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    let detail = body
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty());
    match (status, code) {
        (403, Some("forged_closer")) => CloseOutcome::Refused(
            "The daemon refused: this identity belongs to an agent or the system in this \
             room, and only a person on the roster can close it."
                .to_string(),
        ),
        // The store's roster proof inside the closing transaction. The item
        // only renders for a roster member, so reaching this means the roster
        // moved under the open dialog.
        (403, _) => CloseOutcome::Refused(with_detail(
            "The daemon refused: you are not on this room's roster, so you cannot close it.",
            detail,
        )),
        (400, _) => CloseOutcome::Refused(with_detail(
            "The daemon refused the request: it did not name a roster member.",
            detail,
        )),
        (404 | 405, _) => CloseOutcome::Refused(route_absent_sentence()),
        (200..=299, _) => CloseOutcome::Refused(
            "The daemon answered without confirming the room closed. Reopen the room to see \
             its current state."
                .to_string(),
        ),
        _ => CloseOutcome::Refused(with_detail(
            &format!("Closing the room failed (HTTP {status})."),
            detail,
        )),
    }
}

fn with_detail(sentence: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{sentence} ({detail})"),
        None => sentence.to_string(),
    }
}

fn route_absent_sentence() -> String {
    "This Ocean cannot close rooms yet \u{2014} its daemon or proxy predates the close route. \
     Update ocean-os on the machine running it."
        .to_string()
}

/// The answer was lost on the way back — the browser's transport failed, or a
/// hop in front of the daemon (proxy, gateway) gave up on it. The daemon
/// commits the close before it answers, so a lost answer can hide a close
/// that happened.
fn uncertain_sentence(what: &str) -> String {
    format!(
        "The request was cut ({what}). The room may have closed without this screen hearing \
         back \u{2014} try again: a room that already closed says so."
    )
}

// ---- State ------------------------------------------------------------------

/// Reactive handle for the open room's close confirmation.
///
/// Constructed at `RoomsWorkspace` scope, never inside the header closure:
/// that closure re-runs on every `open_room` write (roster SSE, a leave, an
/// agent add), and an in-flight flag rebuilt by one would re-enable the
/// button during its own close. The dialog is mounted at that scope too, so a
/// header rebuild cannot unmount a request mid-flight.
#[derive(Clone, Copy)]
pub struct RoomCloseState {
    /// The confirmation dialog is open.
    confirm: RwSignal<bool>,
    /// A close is in flight — blocks a second POST and drives the label.
    in_flight: RwSignal<bool>,
    /// The last refusal, shown inside the dialog.
    error: RwSignal<Option<String>>,
    /// The header's overflow trigger, where focus returns on dismiss.
    trigger_ref: NodeRef<leptos::html::Button>,
    /// The dialog's safe choice, focused when it opens.
    keep_ref: NodeRef<leptos::html::Button>,
    /// The dialog's firing control, the last stop of the focus trap.
    fire_ref: NodeRef<leptos::html::Button>,
    /// The dialog element, which holds focus while both buttons are disabled.
    dialog_ref: NodeRef<leptos::html::Div>,
}

impl RoomCloseState {
    /// `trigger_ref` is the overflow trigger the `Close room…` item lives
    /// under; dismissing the dialog hands focus back to it.
    pub fn new(trigger_ref: NodeRef<leptos::html::Button>) -> Self {
        Self {
            confirm: RwSignal::new(false),
            in_flight: RwSignal::new(false),
            error: RwSignal::new(None),
            trigger_ref,
            keep_ref: NodeRef::new(),
            fire_ref: NodeRef::new(),
            dialog_ref: NodeRef::new(),
        }
    }

    /// Open the confirmation. The overflow item is the only caller.
    pub(crate) fn arm(&self) {
        self.error.set(None);
        self.confirm.set(true);
    }

    /// Retire everything: a confirmation belongs to the room it was opened
    /// over. A reply still in flight is dropped by its own generation check.
    fn reset(&self) {
        self.confirm.set(false);
        self.in_flight.set(false);
        self.error.set(None);
    }

    /// Dismiss the dialog without closing anything. Refused while a close is
    /// in flight: the request is already out, and pretending otherwise would
    /// hide its answer.
    fn dismiss(&self) {
        if self.in_flight.get_untracked() {
            return;
        }
        self.confirm.set(false);
        self.error.set(None);
        if let Some(trigger) = self.trigger_ref.get_untracked() {
            let _ = trigger.focus();
        }
    }

    /// Keep Tab inside the modal ([`tab_trap`]).
    fn trap_tab(&self, ev: &web_sys::KeyboardEvent) {
        let active = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element());
        let is_active = |el: Option<web_sys::HtmlButtonElement>| {
            el.zip(active.as_ref())
                .is_some_and(|(el, active)| AsRef::<web_sys::Element>::as_ref(&el) == active)
        };
        let focused = if is_active(self.keep_ref.get_untracked()) {
            DialogFocus::Keep
        } else if is_active(self.fire_ref.get_untracked()) {
            DialogFocus::Fire
        } else {
            DialogFocus::Other
        };
        let Some(step) = tab_trap(ev.shift_key(), focused, self.in_flight.get_untracked()) else {
            return;
        };
        ev.prevent_default();
        match step {
            TrapMove::First => {
                if let Some(keep) = self.keep_ref.get_untracked() {
                    let _ = keep.focus();
                }
            }
            TrapMove::Last => {
                if let Some(fire) = self.fire_ref.get_untracked() {
                    let _ = fire.focus();
                }
            }
            TrapMove::Hold => {
                if let Some(dialog) = self.dialog_ref.get_untracked() {
                    let _ = dialog.focus();
                }
            }
        }
    }

    /// Fire the close for the open room.
    fn fire(&self, rooms: Rooms) {
        if self.in_flight.get_untracked() {
            return;
        }
        let Some(key) = rooms.open_key.get_untracked().filter(|key| !key.is_empty()) else {
            return;
        };
        let actor = rooms.identity_id.get_untracked();
        if let Err(sentence) = dispatch_check(actor, rooms.open_room.get_untracked().as_ref()) {
            self.error.set(Some(sentence));
            return;
        }
        let url = room_close_url(&rooms.url.get_untracked(), &key, actor);
        let generation = rooms.generation_snapshot();
        let me = *self;
        self.in_flight.set(true);
        self.error.set(None);
        // Both buttons disable while in flight, which would drop focus to
        // `<body>`; the dialog itself holds it instead.
        if let Some(dialog) = self.dialog_ref.get_untracked() {
            let _ = dialog.focus();
        }
        spawn_local(async move {
            let outcome = post_close(&url).await;
            let closed = matches!(outcome, CloseOutcome::Closed | CloseOutcome::AlreadyClosed);
            if !rooms.room_is_current(generation, &key) {
                // The reader moved on, but the room DID close: the rail
                // still lists it until it is told.
                if closed {
                    rooms.fetch_rooms_silent();
                }
                return;
            }
            me.in_flight.set(false);
            match outcome {
                CloseOutcome::Closed | CloseOutcome::AlreadyClosed => {
                    me.confirm.set(false);
                    // Leave the frozen room through the one shared reset path:
                    // the generation bump retires the live tail (which the
                    // daemon has ended anyway) and clears every room signal.
                    // There is no audit view to re-hydrate into on this
                    // surface, so the rail is where a close lands.
                    rooms.close_room();
                    // The rail lists open rooms; the closed one leaves it.
                    rooms.fetch_rooms();
                    focus_room_list();
                }
                CloseOutcome::Refused(sentence) => {
                    me.error.set(Some(sentence));
                    if let Some(keep) = me.keep_ref.get_untracked() {
                        let _ = keep.focus();
                    }
                }
            }
        });
    }
}

/// After a close the dialog and the room header are both gone and focus has
/// fallen to `<body>`. Hand it to the rail's listbox tab stop, which is the
/// next thing a keyboard reader would reach for. Deferred one frame because
/// the reset that empties the center rail renders on the next tick, and best
/// effort: no list, no move.
fn focus_room_list() {
    request_animation_frame(move || {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let lost = match document.active_element() {
            None => true,
            Some(active) => document
                .body()
                .is_some_and(|body| AsRef::<web_sys::Element>::as_ref(&body) == &active),
        };
        if !lost {
            return;
        }
        if let Ok(Some(stop)) = document
            .query_selector("#rooms-workspace-room-list [role=\"listbox\"] [tabindex=\"0\"]")
        {
            if let Ok(stop) = stop.dyn_into::<web_sys::HtmlElement>() {
                let _ = stop.focus();
            }
        }
    });
}

/// One close POST: transport, decode, classify. No body — the route reads
/// only its path and query.
async fn post_close(url: &str) -> CloseOutcome {
    match Request::post(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.json::<CloseReply>().await.ok();
            classify_close(status, body)
        }
        Err(err) => CloseOutcome::Refused(uncertain_sentence(&err.to_string())),
    }
}

// ---- Component --------------------------------------------------------------

/// The close confirmation the overflow's `Close room…` item arms. Mounted once
/// at `RoomsWorkspace` scope beside the center rail (it is `position: fixed`,
/// so where it sits in the tree does not matter), and empty until armed.
#[component]
pub fn RoomCloseDialog(rooms: Rooms, state: RoomCloseState) -> impl IntoView {
    // One room admission, one confirmation: a dialog armed over one room
    // must not survive into the next, and the post-close `close_room` that
    // clears `open_key` retires it too.
    Effect::new(move |_| {
        let _ = rooms.open_key.get();
        state.reset();
    });

    let local_only = Memo::new(move |_| close_is_local_only(rooms.access.get().as_ref()));

    // Focus the safe choice when the dialog mounts.
    Effect::new(move |_| {
        if let Some(keep) = state.keep_ref.get() {
            let _ = keep.focus();
        }
    });

    let room_name = move || {
        rooms
            .open_room
            .get()
            .map(|room| room.name)
            .unwrap_or_default()
    };

    view! {
        {move || {
            if !state.confirm.get() {
                return ().into_any();
            }
            view! {
                <div
                    class="rooms-workspace__close-scrim"
                    on:click=move |_| state.dismiss()
                ></div>
                <div
                    class="rooms-workspace__close-dialog"
                    role="alertdialog"
                    aria-modal="true"
                    aria-labelledby="rooms-close-title"
                    aria-describedby="rooms-close-desc"
                    tabindex="-1"
                    node_ref=state.dialog_ref
                    on:keydown=move |ev| {
                        match ev.key().as_str() {
                            // `prevent_default` is the signal the workspace
                            // and window Escape handlers stand down on.
                            "Escape" => {
                                ev.prevent_default();
                                state.dismiss();
                            }
                            "Tab" => state.trap_tab(&ev),
                            _ => {}
                        }
                    }
                >
                    <h2 class="rooms-workspace__close-title" id="rooms-close-title">
                        {move || format!("Close #{}?", room_name())}
                    </h2>
                    <p class="rooms-workspace__close-warn" id="rooms-close-desc">
                        "Closing is permanent \u{2014} a closed room cannot be reopened. It \
                         freezes for everyone here: nobody can post, agent turns running in \
                         it are cancelled, and no new messages arrive. The transcript and \
                         roster stay as a closed record, and the transcript will say you \
                         closed it. If this Ocean's operator has turned on room retention, \
                         that record is deleted once the retention window passes."
                    </p>
                    {move || {
                        local_only.get().then(|| view! {
                            <p class="rooms-workspace__close-note">
                                "This room is federated. Closing it here does not close it \
                                 on Bedrock \u{2014} members on other Ocean nodes keep their \
                                 copy."
                            </p>
                        })
                    }}
                    {move || {
                        state.error.get().map(|error| view! {
                            <div class="rooms-workspace__close-error" role="alert">
                                {error}
                            </div>
                        })
                    }}
                    <div class="rooms-workspace__close-actions">
                        <button
                            class="rooms-workspace__close-keep"
                            type="button"
                            node_ref=state.keep_ref
                            disabled=move || state.in_flight.get()
                            on:click=move |_| state.dismiss()
                        >
                            "Keep room open"
                        </button>
                        <button
                            class="rooms-workspace__close-fire"
                            type="button"
                            node_ref=state.fire_ref
                            disabled=move || state.in_flight.get()
                            on:click=move |_| state.fire(rooms)
                        >
                            {move || {
                                if state.in_flight.get() {
                                    "Closing\u{2026}"
                                } else {
                                    "Close room"
                                }
                            }}
                        </button>
                    </div>
                </div>
            }
            .into_any()
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::RoomParticipant;

    fn participant(id: &str, kind: RoomParticipantKind) -> RoomParticipant {
        RoomParticipant {
            id: id.to_string(),
            kind,
            display_name: id.to_string(),
        }
    }

    fn room(participants: Vec<RoomParticipant>) -> Room {
        serde_json::from_value::<Room>(serde_json::json!({ "id": "team", "name": "Team" }))
            .map(|mut room| {
                room.participants = participants;
                room
            })
            .expect("minimal room decodes")
    }

    fn reply(value: serde_json::Value) -> Option<CloseReply> {
        Some(serde_json::from_value(value).expect("close reply decodes"))
    }

    // ── eligibility: the daemon's member lane, point for point ─────────────

    #[test]
    fn a_person_on_the_roster_is_offered_the_close() {
        let room = room(vec![participant("alice", RoomParticipantKind::Human)]);
        assert!(close_offered("alice", Some(&room)));
    }

    #[test]
    fn bot_and_tool_rows_pass_the_daemon_forged_kind_gate_too() {
        // `room_close` refuses only Agent and System; the store proves roster
        // membership for everyone else.
        for kind in [RoomParticipantKind::Bot, RoomParticipantKind::Tool] {
            let room = room(vec![participant("me", kind)]);
            assert!(close_offered("me", Some(&room)), "{kind:?}");
        }
    }

    #[test]
    fn agent_and_system_identities_are_never_offered_the_close() {
        // The daemon answers 403 forged_closer for both.
        for kind in [RoomParticipantKind::Agent, RoomParticipantKind::System] {
            let room = room(vec![participant("me", kind)]);
            assert!(!close_offered("me", Some(&room)), "{kind:?}");
        }
    }

    #[test]
    fn a_viewer_off_the_roster_is_not_offered_the_close() {
        // The store's `RoomCloserNotInRoster` 403.
        let room = room(vec![participant("alice", RoomParticipantKind::Human)]);
        assert!(!close_offered("bob", Some(&room)));
    }

    #[test]
    fn an_unloaded_or_blank_identity_is_not_offered_the_close() {
        let room = room(vec![
            participant("alice", RoomParticipantKind::Human),
            participant("", RoomParticipantKind::Human),
        ]);
        assert!(
            !close_offered("", Some(&room)),
            "`/v1/me` has not answered: an empty owner must not act"
        );
        assert!(
            !close_offered("   ", Some(&room)),
            "the daemon trims actor_id and answers 400 for a blank one"
        );
        assert!(!close_offered("alice", None), "no room loaded");
    }

    #[test]
    fn the_identity_is_compared_trimmed_as_the_daemon_trims_it() {
        let room = room(vec![participant("alice", RoomParticipantKind::Human)]);
        assert!(close_offered(" alice ", Some(&room)));
    }

    // ── the URL ───────────────────────────────────────────────────────────

    #[test]
    fn the_close_url_is_the_member_lane_with_every_part_encoded() {
        assert_eq!(
            room_close_url("http://127.0.0.1:4780/", "team room", " a&b#c "),
            "http://127.0.0.1:4780/v1/rooms/persistent/team%20room/close?actor_id=a%26b%23c",
        );
        assert_eq!(
            room_close_url("", "team", "alice"),
            "/v1/rooms/persistent/team/close?actor_id=alice",
            "the browser PWA reaches the proxy on its own origin"
        );
    }

    // ── the reply ─────────────────────────────────────────────────────────

    #[test]
    fn a_confirmed_close_is_closed() {
        let body = reply(serde_json::json!({
            "ok": true, "closed": true, "marker_seq": 7, "room": { "id": "team", "name": "Team" }
        }));
        assert_eq!(classify_close(200, body), CloseOutcome::Closed);
    }

    #[test]
    fn a_2xx_without_the_closed_flag_is_not_read_as_closed() {
        assert!(matches!(
            classify_close(200, reply(serde_json::json!({ "ok": true }))),
            CloseOutcome::Refused(_)
        ));
        assert!(matches!(
            classify_close(
                200,
                reply(serde_json::json!({ "ok": false, "closed": true }))
            ),
            CloseOutcome::Refused(_)
        ));
    }

    #[test]
    fn the_daemon_unknown_room_404_is_already_closed_not_a_failure() {
        // Exactly what `room_store_error_response(UnknownRoom)` sends from the
        // monorepo close route: a JSON refusal with no marker and no code.
        let body = reply(serde_json::json!({ "ok": false, "error": "unknown room: team" }));
        assert_eq!(classify_close(404, body), CloseOutcome::AlreadyClosed);
        // And the read routes' marked shape, should the close route ever
        // adopt it.
        let marked = reply(serde_json::json!({
            "ok": false,
            "error": "unknown room: team",
            "room_not_open": true,
            "code": "room_not_found",
        }));
        assert_eq!(classify_close(404, marked), CloseOutcome::AlreadyClosed);
    }

    #[test]
    fn an_empty_404_or_405_is_a_daemon_without_the_route() {
        let CloseOutcome::Refused(sentence) = classify_close(404, None) else {
            panic!("axum's empty 404 is not a closed room");
        };
        assert!(sentence.contains("predates the close route"), "{sentence}");
        let CloseOutcome::Refused(sentence) = classify_close(405, None) else {
            panic!("405 is not a closed room");
        };
        assert!(sentence.contains("predates the close route"), "{sentence}");
        let CloseOutcome::Refused(sentence) = classify_close(
            405,
            reply(serde_json::json!({ "ok": false, "error": "nope" })),
        ) else {
            panic!("a 405 with a body is still not a closed room");
        };
        assert!(sentence.contains("predates the close route"), "{sentence}");
    }

    #[test]
    fn forged_closer_says_why_in_words() {
        let body = reply(serde_json::json!({
            "ok": false,
            "code": "forged_closer",
            "error": "an agent does not close a room; a client may not close one while claiming its identity",
        }));
        let CloseOutcome::Refused(sentence) = classify_close(403, body) else {
            panic!("forged_closer is a refusal");
        };
        assert!(sentence.contains("agent or the system"), "{sentence}");
    }

    #[test]
    fn a_roster_refusal_says_so_and_carries_the_daemon_detail() {
        // `RoomCloserNotInRoster` through `room_store_error_response`: a 403
        // with the store's sentence and no code.
        let body = reply(serde_json::json!({
            "ok": false,
            "error": "room closer bob is not in room team's roster",
        }));
        let CloseOutcome::Refused(sentence) = classify_close(403, body) else {
            panic!("a roster refusal is a refusal");
        };
        assert!(sentence.contains("not on this room's roster"), "{sentence}");
        assert!(sentence.contains("room closer bob"), "{sentence}");
    }

    #[test]
    fn a_bad_request_and_a_server_fault_are_refusals_with_their_status() {
        let CloseOutcome::Refused(sentence) = classify_close(
            400,
            reply(
                serde_json::json!({ "ok": false, "code": "invalid_request", "error": "closing a room needs ?actor_id=" }),
            ),
        ) else {
            panic!("400 is a refusal");
        };
        assert!(sentence.contains("closing a room needs"), "{sentence}");

        let CloseOutcome::Refused(sentence) = classify_close(
            500,
            reply(serde_json::json!({ "ok": false, "error": "disk I/O" })),
        ) else {
            panic!("500 is a refusal");
        };
        assert!(
            sentence.contains("HTTP 500") && sentence.contains("disk I/O"),
            "{sentence}"
        );

        let CloseOutcome::Refused(sentence) = classify_close(500, None) else {
            panic!("an unreadable 500 is a refusal");
        };
        assert!(sentence.contains("may or may not"), "{sentence}");
    }

    #[test]
    fn a_cut_response_does_not_claim_nothing_happened() {
        let sentence = uncertain_sentence("NetworkError");
        assert!(sentence.contains("may have closed"), "{sentence}");
    }

    fn uncertain(outcome: CloseOutcome) -> bool {
        matches!(outcome, CloseOutcome::Refused(sentence) if sentence.contains("may have closed"))
    }

    #[test]
    fn a_hop_losing_the_daemon_answer_is_uncertain_not_failed() {
        // The proxy's `502 daemon unreachable` is plain text (no JSON), and a
        // gateway in front of it answers 502/504 with or without a body.
        for status in [502, 504] {
            assert!(uncertain(classify_close(status, None)), "{status} bare");
            assert!(
                uncertain(classify_close(
                    status,
                    reply(serde_json::json!({ "ok": false, "error": "bad gateway" }))
                )),
                "{status} with body"
            );
        }
    }

    #[test]
    fn a_503_is_a_plain_failure_not_an_uncertain_close() {
        // The proxy answers 503 only before addressing the daemon (an
        // owner-mutation key it cannot read), so nothing can have closed.
        let outcome = classify_close(
            503,
            reply(serde_json::json!({ "ok": false, "error": "operator credential unavailable" })),
        );
        let CloseOutcome::Refused(sentence) = outcome else {
            panic!("503 is a refusal");
        };
        assert!(!sentence.contains("may have closed"), "{sentence}");
        assert!(sentence.contains("HTTP 503"), "{sentence}");
    }

    #[test]
    fn not_open_counts_only_on_the_daemon_404() {
        for status in [403, 500, 200] {
            let body = reply(serde_json::json!({
                "ok": false, "room_not_open": true, "error": "x",
            }));
            assert_ne!(
                classify_close(status, body),
                CloseOutcome::AlreadyClosed,
                "{status} carrying a refusal body is not the not-open answer"
            );
        }
    }

    // ── dispatch re-check ─────────────────────────────────────────────────

    #[test]
    fn the_dispatch_check_refuses_what_the_render_gate_would_refuse() {
        let human = room(vec![participant("alice", RoomParticipantKind::Human)]);
        let agent = room(vec![participant("alice", RoomParticipantKind::Agent)]);
        assert_eq!(dispatch_check("alice", Some(&human)), Ok(()));
        for (id, room) in [
            ("", Some(&human)),
            ("bob", Some(&human)),
            ("alice", Some(&agent)),
            ("alice", None),
        ] {
            let refused = dispatch_check(id, room)
                .expect_err("a close the daemon would refuse must not be sent");
            assert!(refused.contains("no longer close"), "{refused}");
        }
    }

    // ── focus trap ────────────────────────────────────────────────────────

    #[test]
    fn tab_never_leaves_the_modal() {
        use DialogFocus::*;
        assert_eq!(tab_trap(false, Fire, false), Some(TrapMove::First));
        assert_eq!(tab_trap(true, Keep, false), Some(TrapMove::Last));
        assert_eq!(tab_trap(false, Other, false), Some(TrapMove::First));
        assert_eq!(tab_trap(true, Other, false), Some(TrapMove::Last));
        // Between the two ends the browser's own move is already inside.
        assert_eq!(tab_trap(false, Keep, false), None);
        assert_eq!(tab_trap(true, Fire, false), None);
        // In flight both are disabled; focus stays on the dialog.
        for focused in [Keep, Fire, Other] {
            for shift in [false, true] {
                assert_eq!(tab_trap(shift, focused, true), Some(TrapMove::Hold));
            }
        }
    }

    // ── the federation note ───────────────────────────────────────────────

    #[test]
    fn only_a_room_known_to_be_federated_gets_the_local_only_note() {
        let projection = |state: &str| {
            serde_json::from_value::<RoomAccessProjection>(serde_json::json!({ "state": state }))
                .expect("projection decodes")
        };
        assert!(!close_is_local_only(Some(&projection("local"))));
        assert!(!close_is_local_only(None));
        for state in ["connecting", "live", "recovering", "revoked"] {
            assert!(close_is_local_only(Some(&projection(state))), "{state}");
        }
    }
}
