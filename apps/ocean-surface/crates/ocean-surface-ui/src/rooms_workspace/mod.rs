//! Slack-style persistent 3-column Rooms workspace.
//!
//! Left rail (room list + create), center rail (header + transcript + composer),
//! right rail (members / details). Renders against the existing CSS classes in
//! `styles/rooms-workspace.css` and the existing [`crate::rooms::Rooms`] signals
//! and API surface. Mounted as the default web+Tauri collaboration surface in
//! [`crate::app`]; the legacy RoomStage/RoomsPanel components in rooms.rs are
//! deleted.
//!
//! Submodules own the pure helpers (each with its unit tests) and the
//! extracted rail components; this module owns `RoomsWorkspace`, the shared
//! signals and Effects that must outlive rail re-renders, and the transcript.

mod access;
mod banners;
mod composer;
mod format;
mod members;
mod mentions;
mod read_cursor;
mod room_list;
#[cfg(test)]
mod test_support;
mod threads;

use leptos::prelude::*;

use crate::room_messages;
use crate::rooms::{author_display_name, local_speaker_id, RoomMessage, Rooms};

pub(crate) use access::access_allows_sharing;
use access::{access_allows_thread_writes, access_allows_writes};
use banners::{AccessStateBanner, InviteResult, OutboxPanel, PendingInviteBanner};
use composer::{
    message_send_admitted, normalized_message_body, outbox_matches_failed_message,
    should_clear_composer, ChannelComposer,
};
use format::{avatar_identity_class, canonical_wire_clock_time, today_day_key};
use members::MembersPanel;
use mentions::{mention_roster, MentionState};
use read_cursor::{
    applied_read_seq, read_advance_needs_queue, read_advance_request, ready_read_target,
    transcript_is_near_bottom, transcript_pass_action, transcript_read_hydrated,
    ReadAdvanceRequest, TranscriptPassAction,
};
use room_list::RoomListRail;
use threads::{
    is_thread_open, partition_thread_messages, reply_count_for, should_show_thread_button,
    show_transcript_empty, sync_thread_selection, thread_root_for, ThreadPanel,
};

// ── Production helpers (testable directly, called from Effects) ─

/// Toggle a boolean signal — the exact logic consumed by hamburger
/// drawer open/close clicks.
fn toggle_drawer(current: bool) -> bool {
    !current
}

/// Escape behavior owned by the Rooms workspace. Only a visible compact
/// drawer is handled here; every other Escape bubbles to the app-level
/// topmost-surface hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompactEscapeAction {
    CloseDrawer,
}

fn compact_escape_action(
    is_compact: bool,
    drawer_open: bool,
    default_prevented: bool,
) -> Option<CompactEscapeAction> {
    if is_compact && drawer_open && !default_prevented {
        Some(CompactEscapeAction::CloseDrawer)
    } else {
        None
    }
}

/// Keep this aligned with the compact media queries in
/// `styles/rooms-workspace.css` and Tauri's minimum window width.
fn rooms_layout_is_compact() -> bool {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .is_some_and(|width| width <= 650.0)
}

/// Display name for an author/member id in the open room (tracked, so a late
/// roster load or an owner rename re-renders names in place).
fn author_name(rooms: Rooms, id: &str) -> String {
    author_display_name(
        rooms.open_room.get().as_ref(),
        rooms.access.get().as_ref(),
        id,
    )
}

/// Who this surface speaks as in the open room (untracked read), or empty
/// while the owner identity or federated member projection is still loading.
fn speaker_id(rooms: Rooms) -> String {
    local_speaker_id(
        rooms.access.get_untracked().as_ref(),
        rooms.identity_id.get_untracked(),
    )
    .unwrap_or_default()
}

// ── Component ─────────────────────────────────────────────────────────

/// Full-screen 3-column Slack-style rooms workspace.
///
/// Takes a [`Rooms`] handle (Clone, Copy) and drives the three rails:
///
/// - **Left:** room list with active highlight, new-room create input.
/// - **Center:** selected room header, message timeline, composer form,
///   status bar.
/// - **Right:** participant / member roster with kind, role, and presence
///   badges.
///
/// At 650px and below, a compact top nav reveals the hidden left rail so the
/// reader is never stranded with no room navigation. Tauri can reach this at
/// its matching minimum window width.
#[component]
pub fn RoomsWorkspace(
    rooms: Rooms,
    /// The owner's daemon handle; work cards render agent-session steps with
    /// the session transcript's own components.
    daemon: crate::daemon::Daemon,
    /// Called when the user wants to leave the Rooms workspace entirely
    /// (e.g. switch to Direct Messages). If `None` the close button is
    /// hidden.
    #[prop(optional)]
    on_close: Option<Callback<()>>,
) -> impl IntoView {
    let daemon_sv = StoredValue::new(daemon);
    // Toggle for narrow-screen left-rail visibility.
    let show_left_rail = RwSignal::new(false);

    // Refresh owner identity and room list when bootstrap resolves the origin.
    Effect::new(move |_| {
        let _origin = rooms.url.get();
        rooms.fetch_me();
        rooms.fetch_rooms();
    });

    // ── Center-rail: composer signal + focus/scroll refs ────────────────
    let composer = RwSignal::new(String::new());
    let thread_composer = RwSignal::new(String::new());
    let selected_thread_root_seq = RwSignal::new(None::<u64>);
    // Team-platform P6: the right rail shows search when no thread is open.
    let search_open = RwSignal::new(false);
    // An inbox pick asks for a thread; open it once its root has loaded.
    Effect::new(move |_| {
        if let Some(root) = rooms.take_pending_thread_focus() {
            selected_thread_root_seq.set(Some(root));
        }
    });
    let thread_send_in_flight = RwSignal::new(false);
    let copied_invite_link = RwSignal::new(None::<String>);
    let invite_copy_error = RwSignal::new(None::<(String, String)>);
    let thread_last_sent_draft = RwSignal::new(String::new());
    let thread_last_sent_wire = RwSignal::new(String::new());
    let thread_last_sent_seq = RwSignal::new(0u64);
    let list_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Mention truth source: the open room's daemon-provided roster ids.
    // room_markdown highlights @id ONLY when it resolves here.
    let member_ids = Memo::new(move |_| {
        let local_participants = rooms
            .open_room
            .get()
            .map(|room| room.participants)
            .unwrap_or_default();
        mention_roster(&local_participants, rooms.access.get().as_ref())
            .into_iter()
            .map(|participant| participant.id)
            .collect::<std::collections::HashSet<_>>()
    });
    let mobile_toggle_ref: NodeRef<leptos::html::Button> = NodeRef::new();

    // ── Mention autosuggest state (channel + thread composers) ──
    let mention = MentionState::new(rooms);
    let thread_mention = MentionState::new(rooms);

    // Keep transcript pinned to newest message. When the reader has
    // scrolled up, never yank them — surface the missed append as a
    // "New messages" jump affordance instead (client scroll state only;
    // this is not read-cursor "unread" state).
    let transcript = rooms.transcript;
    let new_below = RwSignal::new(false);
    let pending_read_advance = RwSignal::new(None::<ReadAdvanceRequest>);
    let refresh_handle = RwSignal::new(None::<IntervalHandle>);
    Effect::new(move |prev: Option<usize>| {
        let len = transcript.with(|t| t.len());
        let open_key = rooms.open_key.get();
        // Track the access projection. For a `Live` room the durable candidate
        // is `last_confirmed_global_sequence`, which routinely lands *after*
        // the fill that pinned the transcript to the bottom; reading it
        // reactively lets that arrival re-enter this pass and queue the read
        // advance the fill itself could not compute. A re-run cannot mark a
        // scrolled-up reader read or fake hydration: it is not a first fill
        // (`prev_len > 0`), so it only queues through the `PinAndQueue` arm,
        // which requires a measured, genuinely at-bottom transcript.
        let access = rooms.access.get();
        let prev_len = prev.unwrap_or(0);
        let first_fill = prev_len == 0;
        let el = list_ref.get();
        let metrics = el
            .as_ref()
            .map(|el| (el.scroll_height(), el.scroll_top(), el.client_height()));
        let near_bottom = metrics.is_some_and(|(scroll_height, scroll_top, client_height)| {
            transcript_is_near_bottom(scroll_height, scroll_top, client_height, 120)
        });
        match transcript_pass_action(len, prev_len, el.is_some(), near_bottom) {
            TranscriptPassAction::Reset => {
                // Generation reset / room switch: nothing below.
                new_below.set(false);
                pending_read_advance.set(None);
            }
            TranscriptPassAction::PinAndQueue => {
                let (scroll_height, _, client_height) = metrics.unwrap_or_default();
                if let Some(el) = el.clone() {
                    request_animation_frame(move || el.set_scroll_top(el.scroll_height()));
                }
                new_below.set(false);
                pending_read_advance.set(read_advance_request(
                    open_key.as_deref(),
                    rooms.generation_snapshot(),
                    // A scrollable first fill defers to the `scroll` event the
                    // pin above produces; a transcript that fits its measured
                    // viewport never fires one and marks read here instead.
                    transcript_read_hydrated(first_fill, scroll_height, client_height),
                    near_bottom,
                    rooms.open_room.get().is_some(),
                    &rooms.transcript.get_untracked(),
                    access.as_ref(),
                ));
            }
            TranscriptPassAction::RaiseJump => {
                new_below.set(true);
                pending_read_advance.set(None);
            }
            TranscriptPassAction::Hold => {}
        }
        // Single open-none clear: this Effect already tracks `open_key`, so it
        // re-runs on close and drops any queued request in the same pass.
        if open_key.is_none() {
            pending_read_advance.set(None);
        }
        // Only a pass that measured a real viewport may consume the first-fill
        // state. An element reporting zero height was not laid out, and
        // spending the first fill on it would hand the *next* pass unearned
        // hydration (see `transcript_read_hydrated`).
        let viewport_measured = metrics.is_some_and(|(_, _, client_height)| client_height > 0);
        if len == 0 {
            0
        } else if viewport_measured {
            len
        } else {
            prev_len
        }
    });

    Effect::new(move |_| {
        let Some(request) = pending_read_advance.get() else {
            return;
        };
        if !rooms.room_is_current(request.generation, &request.open_room_key) {
            pending_read_advance.set(None);
            return;
        }
        let open = rooms.open_key.get();
        let ready = rooms.open_room.get().is_some();
        if ready && open.is_some() {
            rooms.mark_open_read_if_current(request.candidate_read_seq);
            pending_read_advance.set(None);
        }
    });

    // The one confirmed-at-bottom queue path, shared by the transcript
    // `scroll` handler and the jump-to-latest button. Both are already-proven
    // at-bottom intents (hydration and near-bottom are settled by the caller),
    // so the only question left is whether this target is worth publishing —
    // see `read_advance_needs_queue`. Returning early instead of writing
    // `None` also stops a candidate-less frame from clobbering a still-queued
    // request that was valid when it was built.
    let queue_bottom_read_advance = move || {
        let Some(open_room_key) = rooms.open_key.get_untracked() else {
            return;
        };
        let generation = rooms.generation_snapshot();
        let Some(candidate_read_seq) = ready_read_target(
            true,
            true,
            rooms.open_room.get_untracked().is_some(),
            &rooms.transcript.get_untracked(),
            rooms.access.get_untracked().as_ref(),
        ) else {
            return;
        };
        let applied = applied_read_seq(
            rooms.read_summaries.with_untracked(|summaries| {
                summaries
                    .get(&open_room_key)
                    .and_then(|summary| summary.read_seq)
            }),
            rooms.open_read_cursor.get_untracked().as_ref(),
        );
        let needs_queue = pending_read_advance.with_untracked(|pending| {
            read_advance_needs_queue(
                pending.as_ref(),
                &open_room_key,
                generation,
                candidate_read_seq,
                applied,
            )
        });
        if !needs_queue {
            return;
        }
        pending_read_advance.set(Some(ReadAdvanceRequest {
            open_room_key,
            generation,
            candidate_read_seq,
        }));
    };

    Effect::new(move |_| {
        if refresh_handle.get().is_none() {
            let rooms = rooms;
            let handle = leptos::prelude::set_interval_with_handle(
                move || rooms.fetch_rooms_silent(),
                std::time::Duration::from_secs(8),
            )
            .expect("rooms refresh interval");
            refresh_handle.set(Some(handle));
        }
    });

    Owner::on_cleanup(move || {
        if let Some(handle) = refresh_handle.get_untracked() {
            handle.clear();
        }
        refresh_handle.set(None);
    });

    Effect::new(move |_| {
        let next = sync_thread_selection(
            selected_thread_root_seq.get(),
            rooms.open_key.get().as_deref(),
            &rooms.transcript.get(),
        );
        if next != selected_thread_root_seq.get_untracked() {
            selected_thread_root_seq.set(next);
        }
        if next.is_none() {
            thread_composer.set(String::new());
            thread_send_in_flight.set(false);
            thread_last_sent_draft.set(String::new());
            thread_last_sent_wire.set(String::new());
            thread_last_sent_seq.set(0);
        }
    });

    // ── Composer: one admitted send at a time. Keep both the exact draft and
    // the normalized wire body: daemon messages are trimmed, while clearing is
    // allowed only if the operator has not edited the original draft.
    let last_sent_draft = RwSignal::new(String::new());
    let last_sent_wire = RwSignal::new(String::new());
    let last_sent_seq = RwSignal::new(0u64);
    let send_in_flight = RwSignal::new(false);

    // A completion from a prior room/thread is intentionally discarded by the
    // Rooms generation guard. Reset the matching local latch on context changes
    // so the newly selected conversation can never inherit a disabled composer.
    Effect::new(move |previous: Option<(Option<String>, Option<u64>)>| {
        let current = (rooms.open_key.get(), selected_thread_root_seq.get());
        if let Some((previous_room, previous_root)) = previous {
            if previous_room != current.0 {
                send_in_flight.set(false);
                last_sent_draft.set(String::new());
                last_sent_wire.set(String::new());
                last_sent_seq.set(0);
                thread_send_in_flight.set(false);
                thread_last_sent_draft.set(String::new());
                thread_last_sent_wire.set(String::new());
                thread_last_sent_seq.set(0);
            } else if previous_root != current.1 {
                thread_send_in_flight.set(false);
                thread_last_sent_draft.set(String::new());
                thread_last_sent_wire.set(String::new());
                thread_last_sent_seq.set(0);
            }
        }
        current
    });

    let do_send = move || {
        let draft = composer.get_untracked();
        if !message_send_admitted(
            send_in_flight.get_untracked(),
            thread_send_in_flight.get_untracked(),
            access_allows_writes(rooms.access.get_untracked().as_ref()),
            &draft,
        ) {
            return;
        }
        let wire = normalized_message_body(&draft);
        let max_seq = rooms
            .transcript
            .get_untracked()
            .iter()
            .map(|m| m.seq)
            .max()
            .unwrap_or(0);
        rooms.status.set(String::new());
        last_sent_draft.set(draft);
        last_sent_wire.set(wire.clone());
        last_sent_seq.set(max_seq);
        send_in_flight.set(true);
        rooms.post_message(wire, None);
    };

    let do_send_thread_reply = move || {
        let Some(root_seq) = selected_thread_root_seq.get_untracked() else {
            return;
        };
        let draft = thread_composer.get_untracked();
        if !message_send_admitted(
            thread_send_in_flight.get_untracked(),
            send_in_flight.get_untracked(),
            access_allows_thread_writes(rooms.access.get_untracked().as_ref()),
            &draft,
        ) {
            return;
        }
        let wire = normalized_message_body(&draft);
        let max_seq = rooms
            .transcript
            .get_untracked()
            .iter()
            .map(|m| m.seq)
            .max()
            .unwrap_or(0);
        rooms.status.set(String::new());
        thread_last_sent_draft.set(draft);
        thread_last_sent_wire.set(wire.clone());
        thread_last_sent_seq.set(max_seq);
        thread_send_in_flight.set(true);
        rooms.post_message(wire, Some(root_seq));
    };

    Effect::new(move |_: Option<()>| {
        if !send_in_flight.get() {
            return;
        }
        let wire = last_sent_wire.get();
        let sent_at_seq = last_sent_seq.get();
        let me = speaker_id(rooms);
        let me = me.as_str();
        let confirmed = rooms.transcript.get().iter().any(|m| {
            m.seq > sent_at_seq
                && m.body == wire
                && m.author_id == me
                && m.thread_parent_seq.is_none()
        });
        if confirmed {
            let original = last_sent_draft.get_untracked();
            if should_clear_composer(&composer.get_untracked(), &original) {
                composer.set(String::new());
            }
            last_sent_draft.set(String::new());
            last_sent_wire.set(String::new());
            last_sent_seq.set(0);
            send_in_flight.set(false);
            return;
        }

        let failed_outbox = rooms.access.get().is_some_and(|access| {
            access
                .outbox
                .iter()
                .any(|item| outbox_matches_failed_message(item, me, &wire, None))
        });
        let request_failed = rooms.status.get().starts_with("message ");
        if failed_outbox || request_failed {
            last_sent_draft.set(String::new());
            last_sent_wire.set(String::new());
            last_sent_seq.set(0);
            send_in_flight.set(false);
        }
    });

    Effect::new(move |_: Option<()>| {
        if !thread_send_in_flight.get() {
            return;
        }
        let Some(root_seq) = selected_thread_root_seq.get() else {
            return;
        };
        let wire = thread_last_sent_wire.get();
        let sent_at_seq = thread_last_sent_seq.get();
        let me = speaker_id(rooms);
        let me = me.as_str();
        let confirmed = rooms.transcript.get().iter().any(|m| {
            m.seq > sent_at_seq
                && m.body == wire
                && m.author_id == me
                && m.thread_parent_seq == Some(root_seq)
        });
        if confirmed {
            let original = thread_last_sent_draft.get_untracked();
            if should_clear_composer(&thread_composer.get_untracked(), &original) {
                thread_composer.set(String::new());
            }
            thread_last_sent_draft.set(String::new());
            thread_last_sent_wire.set(String::new());
            thread_last_sent_seq.set(0);
            thread_send_in_flight.set(false);
            return;
        }

        let failed_outbox = rooms.access.get().is_some_and(|access| {
            access
                .outbox
                .iter()
                .any(|item| outbox_matches_failed_message(item, me, &wire, Some(root_seq)))
        });
        let request_failed = rooms.status.get().starts_with("message ");
        if failed_outbox || request_failed {
            thread_last_sent_draft.set(String::new());
            thread_last_sent_wire.set(String::new());
            thread_last_sent_seq.set(0);
            thread_send_in_flight.set(false);
        }
    });

    let on_send = Callback::new(move |()| do_send());
    let on_send_thread_reply = Callback::new(move |()| do_send_thread_reply());

    view! {
        <div
            class="rooms-workspace"
            role="region"
            aria-label="Rooms workspace"
            on:keydown=move |ev| {
                if ev.key() != "Escape" {
                    return;
                }
                let action = compact_escape_action(
                    rooms_layout_is_compact(),
                    show_left_rail.get_untracked(),
                    ev.default_prevented(),
                );
                if action == Some(CompactEscapeAction::CloseDrawer) {
                    ev.prevent_default();
                    show_left_rail.set(false);
                    if let Some(toggle) = mobile_toggle_ref.get() {
                        let _ = toggle.focus();
                    }
                }
            }
        >

            // ═══ COMPACT NAV — narrow-screen room selector ════════════
            // Always present but CSS hides it above the shared 650px
            // breakpoint; renders a compact bar with room toggle + active
            // room name so the reader is never stranded.
            <div class="rooms-workspace__mobile-nav">
                <button
                    class="rooms-workspace__mobile-nav-toggle"
                    type="button"
                    node_ref=mobile_toggle_ref
                    aria-label="Toggle room list"
                    aria-controls="rooms-workspace-room-list"
                    aria-expanded=move || show_left_rail.get().to_string()
                    on:click=move |_| show_left_rail.update(|v| *v = toggle_drawer(*v))
                >
                    <svg viewBox="0 0 16 16" width="16" height="16"
                        fill="none" stroke="currentColor" stroke-width="1.5"
                        stroke-linecap="round">
                        <path d="M2 4h12M2 8h12M2 12h12"/>
                    </svg>
                </button>
                <span class="rooms-workspace__mobile-nav-title">
                    {move || rooms.open_key.get()
                        .and_then(|_k| rooms.open_room.get().map(|r| r.name))
                        .unwrap_or_else(|| "Rooms".into())
                    }
                </span>
            </div>

            // ═══ LEFT RAIL — room list ═══════════════════════════════════
            // Backdrop closes the drawer on narrow screens (tapping outside).
            {move || {
                if show_left_rail.get() {
                    view! {
                        <div
                            class="rooms-workspace__left-backdrop"
                            aria-hidden="true"
                            on:click=move |_| {
                                show_left_rail.set(false);
                                if let Some(toggle) = mobile_toggle_ref.get() {
                                    let _ = toggle.focus();
                                }
                            }
                        ></div>
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}
            <RoomListRail rooms=rooms show_left_rail=show_left_rail on_close=on_close />

            // ═══ CENTER RAIL — header + transcript + composer ═══════════
            <div class="rooms-workspace__center">
                <PendingInviteBanner rooms=rooms />
                {move || {
                    let open = rooms.open_room.get();
                    match open {
                        None => {
                            // No room open: the empty canvas carries no copy.
                            view! { <div class="rooms-workspace__join"></div> }.into_any()
                        }
                        Some(ref room) => {
                            let joined = rooms.joined_open();
                            let room_name = room.name.clone();

                            view! {
                                // Header
                                <div class="rooms-workspace__center-head">
                                    <span class="rooms-workspace__center-hash">"#"</span>
                                    <h1 class="rooms-workspace__center-title">
                                        {room_name.clone()}
                                    </h1>
                                    <div class="rooms-workspace__center-actions">
                                        {(!joined).then(|| view! {
                                            <button
                                                class="rooms-workspace__join-btn"
                                                type="button"
                                                on:click=move |_| rooms.join_open()
                                            >
                                                "Join room"
                                            </button>
                                        })}
                                        <button
                                            class="rooms-workspace__center-search"
                                            class:is-on=move || search_open.get()
                                            type="button"
                                            aria-label="Search"
                                            aria-pressed=move || search_open.get().to_string()
                                            on:click=move |_| search_open.update(|o| *o = !*o)
                                        >
                                            <crate::icons::Search />
                                        </button>
                                        <crate::room_overflow::RoomOverflow rooms=rooms joined=joined />
                                        <button
                                            class="rooms-workspace__center-back"
                                            type="button"
                                            title="Back to room list"
                                            aria-label="Close current room"
                                            on:click=move |_| rooms.close_room()
                                        >
                                            <svg viewBox="0 0 16 16" width="14" height="14"
                                                fill="none" stroke="currentColor" stroke-width="1.6"
                                                stroke-linecap="round">
                                                <path d="M10 3L5 8l5 5"/>
                                            </svg>
                                        </button>
                                    </div>
                                </div>

                                // Access status banner (Connecting, Recovering, Revoked)
                                <AccessStateBanner rooms=rooms />

                                // Explicit invite result. Codes appear only
                                // after an owner asks to share and are copied
                                // as an ocean:// link for the installed app.
                                <InviteResult
                                    rooms=rooms
                                    copied_invite_link=copied_invite_link
                                    invite_copy_error=invite_copy_error
                                />

                                // Transcript + empty state
                                // role=log: AT treats the timeline as a
                                // polite live region — new messages are
                                // announced without stealing focus.
                                <div
                                    class="rooms-workspace__transcript"
                                    role="log"
                                    aria-label="Messages"
                                    node_ref=list_ref
                                    on:scroll=move |_| {
                                        if let Some(el) = list_ref.get() {
                                            if transcript_is_near_bottom(
                                                el.scroll_height(),
                                                el.scroll_top(),
                                                el.client_height(),
                                                120,
                                            ) {
                                                new_below.set(false);
                                                queue_bottom_read_advance();
                                            }
                                        }
                                    }
                                >
                                    <For
                                        // Pair each root with its predecessor so
                                        // density decisions (grouping, gap headers,
                                        // day separators) are derived per row. The
                                        // transcript is append-only under one
                                        // generation, so a cached keyed child never
                                        // sees its predecessor change; generation
                                        // reset rebuilds the whole list.
                                        each=move || {
                                            let roots: Vec<RoomMessage> = partition_thread_messages(&rooms.transcript.get(), 0)
                                                .roots
                                                .into_iter()
                                                .filter(|m| !room_messages::is_convene_audit(m))
                                                .collect();
                                            std::iter::once(None)
                                                .chain(roots.iter().cloned().map(Some))
                                                .zip(roots.clone())
                                                .collect::<Vec<_>>()
                                        }
                                        key=|(_, m): &(Option<RoomMessage>, RoomMessage)| m.seq
                                        children=move |(prev, m): (Option<RoomMessage>, RoomMessage)| {
                                            let is_system = room_messages::is_compact_system_row(&m);
                                            let full_ts = m.created_at.clone();
                                            let root_seq = m.seq;
                                            let day_label = room_messages::day_separator_label(prev.as_ref(), &m)
                                                .map(|d| room_messages::humanize_day_label(&d, &today_day_key()));
                                            // A long silence gets a time header —
                                            // unless a day separator already marks
                                            // this row (no double dividers).
                                            let gap_label = (day_label.is_none()
                                                && prev
                                                    .as_ref()
                                                    .map(|p| room_messages::needs_gap_header(p, &m))
                                                    .unwrap_or(false))
                                            .then(|| canonical_wire_clock_time(&full_ts));
                                            let grouped = prev
                                                .as_ref()
                                                .map(|p| room_messages::is_grouped(p, &m))
                                                .unwrap_or(false);
                                            view! {
                                                {day_label.map(|d| view! {
                                                    <div class="rooms-workspace__day-separator" data-day="true">{d}</div>
                                                })}
                                                {gap_label.map(|g| view! {
                                                    <div class="rooms-workspace__day-separator" data-gap="true">{g}</div>
                                                })}
                                                <div
                                                    class="rooms-workspace__msg"
                                                    class:rooms-workspace__msg--system=is_system
                                                    class:rooms-workspace__msg--grouped=grouped
                                                >
                                                    <div class=if is_system {
                                                        "rooms-workspace__msg-avatar".to_string()
                                                    } else {
                                                        format!(
                                                            "rooms-workspace__msg-avatar {}",
                                                            avatar_identity_class(&m.author_id)
                                                        )
                                                    }>
                                                        {if is_system {
                                                            view! { <crate::icons::Waves /> }.into_any()
                                                        } else {
                                                            { let id = m.author_id.clone(); move || crate::rooms::name_initials(&author_name(rooms, &id)) }.into_any()
                                                        }}
                                                    </div>
                                                    <div class="rooms-workspace__msg-body">
                                                        <div class="rooms-workspace__msg-author">
                                                            <span class="rooms-workspace__msg-name">
                                                                { let id = m.author_id.clone(); move || author_name(rooms, &id) }
                                                            </span>
                                                            <time
                                                                class="rooms-workspace__msg-time"
                                                                datetime=full_ts.clone()
                                                                aria-label=full_ts.clone()
                                                                title=full_ts.clone()
                                                            >
                                                                {canonical_wire_clock_time(&full_ts)}
                                                            </time>
                                                        </div>
                                                        <div class="rooms-workspace__msg-text">
                                                            {crate::room_markdown::body_view(m.body.clone(), member_ids)}
                                                        </div>
                                                        {move || {
                                                            if should_show_thread_button(&m) {
                                                                let reply_count = move || reply_count_for(&rooms.transcript.get(), root_seq);
                                                                let thread_label = move || {
                                                                    let count = reply_count();
                                                                    if count > 0 {
                                                                        format!("Open thread ({count})")
                                                                    } else {
                                                                        "Open thread".to_string()
                                                                    }
                                                                };
                                                                // Hover/focus action rail. ONLY real actions live
                                                                // here (reply-in-thread today); persistent when the
                                                                // thread has replies or is open, so truthful state
                                                                // never hides. Reveal is CSS (hover/focus-within,
                                                                // always-on for no-hover pointers).
                                                                view! {
                                                                    <div
                                                                        class="rooms-workspace__action-rail"
                                                                        class:rooms-workspace__action-rail--persistent=move || {
                                                                            reply_count() > 0
                                                                                || selected_thread_root_seq.get() == Some(root_seq)
                                                                        }
                                                                    >
                                                                    <button
                                                                        class="rooms-workspace__thread-toggle"
                                                                        class:rooms-workspace__thread-toggle--active=move || {
                                                                            selected_thread_root_seq.get() == Some(root_seq)
                                                                        }
                                                                        type="button"
                                                                        aria-label=move || {
                                                                            if is_thread_open(selected_thread_root_seq.get(), root_seq) {
                                                                                format!("Close thread for message {}", root_seq)
                                                                            } else {
                                                                                format!("Open thread for message {}", root_seq)
                                                                            }
                                                                        }
                                                                        aria-pressed=move || {
                                                                            is_thread_open(selected_thread_root_seq.get(), root_seq)
                                                                                .to_string()
                                                                        }
                                                                        on:click=move |_| {
                                                                            selected_thread_root_seq.update(|selected| {
                                                                                *selected = if *selected == Some(root_seq) {
                                                                                    None
                                                                                } else {
                                                                                    Some(root_seq)
                                                                                };
                                                                            });
                                                                        }
                                                                    >
                                                                        {move || thread_label()}
                                                                    </button>
                                                                    </div>
                                                                }.into_any()
                                                            } else {
                                                                ().into_any()
                                                            }
                                                        }}
                                                        // Team-platform P3: live agent work cards for turns
                                                        // this message convened.
                                                        <For
                                                            each=move || rooms.runs.with(|runs| crate::rooms::run_ids_for_root(runs, root_seq))
                                                            key=|id: &String| id.clone()
                                                            children=move |id: String| view! {
                                                                <crate::room_work_card::RoomWorkCard
                                                                    run_id=id
                                                                    rooms=rooms
                                                                    daemon=daemon_sv
                                                                />
                                                            }
                                                        />
                                                    </div>
                                                </div>
                                            }
                                        }
                                    />
                                    {move || {
                                        let roots = partition_thread_messages(&rooms.transcript.get(), 0).roots;
                                        let roots_empty = roots.is_empty();
                                        if show_transcript_empty(rooms.transcript_tail_is_live(), roots_empty) {
                                            view! {
                                                <div class="rooms-workspace__empty">
                                                    {move || rooms.open_room.get().map(|r| format!("#{}", r.name))}
                                                </div>
                                            }.into_any()
                                        } else {
                                            ().into_any()
                                        }
                                    }}
                                </div>

                                // Jump affordance for appends missed while
                                // scrolled up. Scroll-state UX only — never
                                // read-cursor "unread" semantics.
                                {move || new_below.get().then(|| view! {
                                    <button
                                        type="button"
                                        class="rooms-workspace__jump-new"
                                        on:click=move |_| {
                                            if let Some(el) = list_ref.get() {
                                                el.set_scroll_top(el.scroll_height());
                                            }
                                            new_below.set(false);
                                            queue_bottom_read_advance();
                                        }
                                    >
                                        "\u{2193} New messages"
                                    </button>
                                })}

                                // Federation outbox is explicitly outside the
                                // confirmed transcript. Pending items are
                                // informational; only failed items can retry.
                                <OutboxPanel rooms=rooms />

                                <crate::room_attention::WorkingIndicator rooms=rooms />
                                // Composer + status line
                                <ChannelComposer
                                    rooms=rooms
                                    composer=composer
                                    send_in_flight=send_in_flight
                                    mention=mention
                                    on_send=on_send
                                />
                            }.into_any()
                        }
                    }
                }}
            </div>

            // ═══ RIGHT RAIL — members / details ═════════════════════════
            <div
                class="rooms-workspace__right"
                class:rooms-workspace__right--thread=move || selected_thread_root_seq.get().is_some()
            >
                <div class="rooms-workspace__right-head">
                    <h3 class="rooms-workspace__right-title">
                        {move || if selected_thread_root_seq.get().is_some() {
                            "Thread"
                        } else if search_open.get() {
                            "Search"
                        } else {
                            "Members"
                        }}
                    </h3>
                    {move || {
                        if selected_thread_root_seq.get().is_none() && search_open.get() {
                            view! {
                                <button
                                    class="rooms-workspace__right-close"
                                    type="button"
                                    aria-label="Close search"
                                    on:click=move |_| search_open.set(false)
                                >
                                    <svg viewBox="0 0 16 16" width="14" height="14"
                                        fill="none" stroke="currentColor" stroke-width="1.6"
                                        stroke-linecap="round">
                                        <path d="M3 3l10 10M13 3L3 13"/>
                                    </svg>
                                </button>
                            }.into_any()
                        } else if selected_thread_root_seq.get().is_some() {
                            view! {
                                <button
                                    class="rooms-workspace__right-close"
                                    type="button"
                                    aria-label="Close thread"
                                    on:click=move |_| selected_thread_root_seq.set(None)
                                >
                                    <svg viewBox="0 0 16 16" width="14" height="14"
                                        fill="none" stroke="currentColor" stroke-width="1.6"
                                        stroke-linecap="round">
                                        <path d="M3 3l10 10M13 3L3 13"/>
                                    </svg>
                                </button>
                            }.into_any()
                        } else {
                            ().into_any()
                        }
                    }}
                </div>

                <div class="rooms-workspace__right-list">
                    {move || {
                        if let Some(root) = thread_root_for(&rooms.transcript.get(), selected_thread_root_seq.get()) {
                            view! {
                                <ThreadPanel
                                    rooms=rooms
                                    root=root
                                    member_ids=member_ids
                                    thread_composer=thread_composer
                                    thread_send_in_flight=thread_send_in_flight
                                    mention=thread_mention
                                    on_send=on_send_thread_reply
                                />
                            }.into_any()
                        } else if search_open.get() {
                            view! {
                                <crate::room_attention::RoomSearchPanel
                                    rooms=rooms
                                    on_pick=Callback::new(move |root: u64| selected_thread_root_seq.set(Some(root)))
                                />
                            }.into_any()
                        } else {
                            view! { <MembersPanel rooms=rooms /> }.into_any()
                        }
                    }}
                </div>

            </div>
        </div>
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_opens_when_closed() {
        assert!(toggle_drawer(false));
    }

    #[test]
    fn drawer_closes_when_open() {
        assert!(!toggle_drawer(true));
    }

    #[test]
    fn drawer_toggle_is_idempotent() {
        // Two toggles = identity — the drawer returns to its previous
        // state, which is the contract for a compact-nav hamburger.
        assert!(!toggle_drawer(toggle_drawer(false)));
        assert!(toggle_drawer(toggle_drawer(true)));
    }

    #[test]
    fn compact_escape_closes_an_open_unhandled_drawer() {
        assert_eq!(
            compact_escape_action(true, true, false),
            Some(CompactEscapeAction::CloseDrawer)
        );
    }

    #[test]
    fn compact_escape_with_closed_drawer_bubbles_to_app() {
        assert_eq!(compact_escape_action(true, false, false), None);
    }

    #[test]
    fn desktop_escape_is_a_rooms_workspace_no_op() {
        assert_eq!(compact_escape_action(false, true, false), None);
    }

    #[test]
    fn already_handled_escape_does_not_also_close_drawer() {
        assert_eq!(compact_escape_action(true, true, true), None);
    }

    fn strip_css_comments(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(open) = rest.find("/*") {
            out.push_str(&rest[..open]);
            if let Some(close) = rest[open + 2..].find("*/") {
                rest = &rest[open + 2 + close + 2..];
            } else {
                break;
            }
        }
        out.push_str(rest);
        out
    }

    /// Brace-matched bodies for media rules with the requested prelude.
    fn css_media_blocks(css: &str, needle: &str) -> Vec<String> {
        let css = strip_css_comments(css);
        let bytes = css.as_bytes();
        let mut blocks = Vec::new();
        let mut from = 0usize;
        while let Some(relative) = css[from..].find(needle) {
            let at = from + relative;
            let Some(open_relative) = css[at..].find('{') else {
                break;
            };
            let open = at + open_relative;
            let mut depth = 0usize;
            let mut end = None;
            for (index, byte) in bytes.iter().enumerate().skip(open) {
                match byte {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                break;
            };
            blocks.push(css[open + 1..end].to_string());
            from = end + 1;
        }
        blocks
    }

    fn css_without_whitespace(css: &str) -> String {
        css.chars().filter(|char| !char.is_whitespace()).collect()
    }

    #[test]
    fn compact_nav_visibility_override_follows_its_hidden_base_rule() {
        let css = include_str!("../../../../styles/rooms-workspace.css");
        let stripped = strip_css_comments(css);
        let compact_blocks = css_media_blocks(&stripped, "@media (max-width: 650px)");
        let has_compact_override = compact_blocks.iter().any(|body| {
            css_without_whitespace(body).contains(".rooms-workspace__mobile-nav{display:flex;")
        });
        assert!(
            has_compact_override,
            "compact media body must make the Rooms mobile nav visible"
        );

        let normalized = css_without_whitespace(&stripped);
        let hidden = normalized
            .find(".rooms-workspace__mobile-nav{display:none;")
            .expect("compact nav needs a hidden desktop base rule");
        let visible = normalized
            .rfind(".rooms-workspace__mobile-nav{display:flex;")
            .expect("compact nav needs a compact visibility override");
        assert!(
            hidden < visible,
            "compact override must follow the base rule"
        );
    }
}
