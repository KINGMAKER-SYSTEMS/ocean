//! Channel composer: send admission, draft clearing, failed-outbox matching,
//! and the room-file attach control.

use leptos::prelude::*;
use std::collections::HashMap;

use super::access::access_allows_writes;
use super::attachments::{upload_control_enabled, RoomAttachmentsState};
use super::mentions::MentionState;
use crate::rooms::{OutboxItemState, Rooms};

/// Whether the composer should clear after the normalized wire body is
/// confirmed. The current draft must still be the exact original draft so
/// typing that happened while the send was in flight is never discarded.
pub(super) fn should_clear_composer(current: &str, original_draft: &str) -> bool {
    !original_draft.is_empty() && current == original_draft
}

/// Stable identity for one room's unsent composer text. The daemon origin is
/// part of the key because different servers may use the same room key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct RoomDraftIdentity {
    origin: String,
    room_key: String,
}

pub(super) type RoomDrafts = HashMap<RoomDraftIdentity, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingRoomSend {
    pub original_draft: String,
    pub wire: String,
    pub sent_at_seq: u64,
}

pub(super) type PendingRoomSends = HashMap<RoomDraftIdentity, PendingRoomSend>;

pub(super) fn retire_pending_room_send(
    pending: &mut PendingRoomSends,
    room: &RoomDraftIdentity,
) -> bool {
    pending.remove(room).is_some()
}

pub(super) fn room_draft_identity(
    origin: String,
    room_key: Option<String>,
) -> Option<RoomDraftIdentity> {
    room_key.map(|room_key| RoomDraftIdentity { origin, room_key })
}

/// Save the visible draft under its old room before loading the new room's
/// draft. A missing room only hides the text; it never discards it.
pub(super) fn transition_room_draft(
    drafts: &mut RoomDrafts,
    current: &mut Option<RoomDraftIdentity>,
    visible: &mut String,
    next: Option<RoomDraftIdentity>,
) {
    if *current == next {
        return;
    }
    if let Some(previous) = current.as_ref() {
        drafts.insert(previous.clone(), visible.clone());
    }
    *visible = next
        .as_ref()
        .and_then(|identity| drafts.get(identity).cloned())
        .unwrap_or_default();
    *current = next;
}

/// Clear a sent draft only in the room that admitted it. The visible value is
/// used when that room is open; otherwise its saved value is checked. This
/// prevents an older completion from clearing another room's composer.
pub(super) fn clear_confirmed_room_draft(
    drafts: &mut RoomDrafts,
    active: Option<&RoomDraftIdentity>,
    visible: &mut String,
    sent_room: &RoomDraftIdentity,
    original_draft: &str,
) -> bool {
    let current = if active == Some(sent_room) {
        visible.as_str()
    } else {
        drafts
            .get(sent_room)
            .map(String::as_str)
            .unwrap_or_default()
    };
    if !should_clear_composer(current, original_draft) {
        return false;
    }
    drafts.insert(sent_room.clone(), String::new());
    if active == Some(sent_room) {
        visible.clear();
    }
    true
}

pub(super) fn normalized_message_body(draft: &str) -> String {
    draft.trim().to_string()
}

pub(super) fn message_send_admitted(
    own_in_flight: bool,
    other_in_flight: bool,
    writes_allowed: bool,
    draft: &str,
) -> bool {
    !own_in_flight
        && !other_in_flight
        && writes_allowed
        && !normalized_message_body(draft).is_empty()
}

pub(super) fn outbox_matches_failed_message(
    item: &crate::rooms::RoomOutboxItem,
    author_member_id: &str,
    wire: &str,
    thread_parent_seq: Option<u64>,
) -> bool {
    item.state == OutboxItemState::Failed
        && item.author_member_id == author_member_id
        && item.payload.get("body").and_then(|body| body.as_str()) == Some(wire)
        && item
            .payload
            .get("thread_parent_seq")
            .and_then(|value| value.as_u64())
            == thread_parent_seq
}

/// Channel composer row (attach, input, mention popup, send) plus the room
/// status line. Draft, in-flight latch, mention state and the attachments
/// state are owned by `RoomsWorkspace` so they survive the center rail
/// re-rendering.
///
/// The attach control takes `attach_allowed` — the workspace's
/// `local_store_write_gate` ruling — rather than the composer's own
/// `access_allows_writes`: an upload lands in this daemon's store and is never
/// enqueued for a peer, so a link that is down or coming back does not hold
/// it the way it holds a message. The paperclip only forwards the gesture to
/// the input `AttachmentInput` mounts at workspace scope.
#[component]
pub(super) fn ChannelComposer(
    rooms: Rooms,
    composer: RwSignal<String>,
    send_in_flight: RwSignal<bool>,
    mention: MentionState,
    on_send: Callback<()>,
    attachments: RoomAttachmentsState,
    attach_allowed: Signal<bool>,
) -> impl IntoView {
    let can_attach = move || {
        upload_control_enabled(
            attach_allowed.get(),
            attachments.uploading.get(),
            rooms.open_key.get().as_deref(),
        )
    };
    view! {
        <div class="rooms-workspace__composer">
            <form
                class="rooms-workspace__composer-row"
                on:submit=move |ev| {
                    ev.prevent_default();
                    on_send.run(());
                }
            >
                <button
                    class="rooms-workspace__composer-attach"
                    type="button"
                    aria-label=move || {
                        if attachments.uploading.get() {
                            "Uploading a room file"
                        } else {
                            "Attach a file to this room"
                        }
                    }
                    title=move || {
                        if attachments.uploading.get() {
                            "Uploading\u{2026}"
                        } else {
                            "Attach a context file to this room (up to 8 MB)"
                        }
                    }
                    aria-busy=move || attachments.uploading.get().to_string()
                    disabled=move || !can_attach()
                    on:click=move |_| {
                        attachments.error.set(None);
                        attachments.open_picker();
                    }
                >
                    <crate::icons::Paperclip />
                </button>
                <input
                    class="rooms-workspace__composer-input"
                    type="text"
                    aria-label="Message"
                    placeholder=move || rooms.open_room.get().map(|r| format!("Message #{}", r.name)).unwrap_or_default()
                    node_ref=mention.input_ref
                    role="combobox"
                    aria-autocomplete="list"
                    aria-controls="rooms-mention-listbox"
                    aria-expanded=move || (!mention.items.get().is_empty()).to_string()
                    aria-activedescendant=move || mention.active_descendant("rooms-mention-opt-")
                    prop:value=move || composer.get()
                    on:input=move |ev| mention.on_input(composer, &ev)
                    on:keydown=move |ev| mention.on_keydown(rooms, composer, &ev)
                    on:blur=move |_| mention.ctx.set(None)
                    disabled=move || !access_allows_writes(rooms.access.get().as_ref())
                />
                {mention.popup(rooms, composer, "rooms-mention-listbox", "rooms-mention-opt-")}
                <button
                    class="rooms-workspace__composer-send"
                    type="submit"
                    disabled=move || {
                        send_in_flight.get()
                            || composer.get().trim().is_empty()
                            || !access_allows_writes(rooms.access.get().as_ref())
                    }
                >
                    {move || if send_in_flight.get() { "Sending…" } else { "Send" }}
                </button>
            </form>

            // A file refusal (too large, no usable name, the daemon's typed
            // code) lands here, beside the control that caused it, whether
            // or not the Files rail is open.
            {move || {
                attachments.error.get().map(|error| view! {
                    <div class="rooms-workspace__files-error" role="alert">{error}</div>
                })
            }}

            {move || {
                let s = rooms.status.get();
                if s.is_empty()
                    || s.starts_with("rooms ")
                    || s.starts_with("create ")
                {
                    ().into_any()
                } else {
                    view! {
                        <div
                            class="rooms-workspace__status"
                            role="status"
                            aria-live="polite"
                        >
                            {s}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_outbox_matching_discriminates_thread_parent_seq() {
        let top_level = crate::rooms::RoomOutboxItem {
            client_event_id: "client-1".into(),
            source_id: "surface-web".into(),
            source_sequence: 1,
            author_member_id: "user".into(),
            event_type: "room_message".into(),
            payload: serde_json::json!({"body": "hello"}),
            mention_member_ids: vec![],
            state: OutboxItemState::Failed,
        };
        assert!(outbox_matches_failed_message(
            &top_level, "user", "hello", None
        ));
        assert!(!outbox_matches_failed_message(
            &top_level,
            "user",
            "hello",
            Some(7)
        ));

        let threaded = crate::rooms::RoomOutboxItem {
            payload: serde_json::json!({"body": "hello", "thread_parent_seq": 7}),
            ..top_level.clone()
        };
        assert!(outbox_matches_failed_message(
            &threaded,
            "user",
            "hello",
            Some(7)
        ));
        assert!(!outbox_matches_failed_message(
            &threaded, "user", "hello", None
        ));
    }

    #[test]
    fn composer_clears_when_unedited() {
        assert!(should_clear_composer("hello", "hello"));
    }

    #[test]
    fn composer_preserves_edited_draft() {
        assert!(!should_clear_composer("hello world", "hello"));
        assert!(!should_clear_composer("something else", "hello"));
    }

    #[test]
    fn composer_preserves_whitespace_edit() {
        // " hi " -> "hi" is still an edit; exact equality only.
        assert!(!should_clear_composer("hi", " hi "));
    }

    #[test]
    fn composer_ignores_empty_sent_body() {
        assert!(!should_clear_composer("hello", ""));
    }

    #[test]
    fn composer_normalizes_wire_body_once() {
        assert_eq!(normalized_message_body("  hello world \n"), "hello world");
    }

    #[test]
    fn composer_admission_rejects_empty_blocked_and_concurrent_sends() {
        assert!(!message_send_admitted(false, false, true, " \n\t "));
        assert!(!message_send_admitted(false, false, false, "hello"));
        assert!(!message_send_admitted(true, false, true, "hello"));
        assert!(!message_send_admitted(false, true, true, "hello"));
        assert!(message_send_admitted(false, false, true, " hello "));
    }

    #[test]
    fn room_drafts_restore_only_for_their_origin_and_key() {
        let room_a = room_draft_identity("https://one.example".into(), Some("a".into()));
        let room_b = room_draft_identity("https://one.example".into(), Some("b".into()));
        let other_server = room_draft_identity("https://two.example".into(), Some("a".into()));
        let mut drafts = RoomDrafts::new();
        let mut current = room_a.clone();
        let mut visible = "draft A".to_string();

        transition_room_draft(&mut drafts, &mut current, &mut visible, room_b.clone());
        assert_eq!(visible, "");
        visible = "draft B".into();
        transition_room_draft(&mut drafts, &mut current, &mut visible, room_a.clone());
        assert_eq!(visible, "draft A");
        transition_room_draft(&mut drafts, &mut current, &mut visible, room_b);
        assert_eq!(visible, "draft B");
        transition_room_draft(&mut drafts, &mut current, &mut visible, other_server);
        assert_eq!(visible, "");
    }

    #[test]
    fn room_draft_survives_identity_loss_and_loading() {
        let room = room_draft_identity("https://one.example".into(), Some("a".into()));
        let mut drafts = RoomDrafts::new();
        let mut current = room.clone();
        let mut visible = "keep me".to_string();

        transition_room_draft(&mut drafts, &mut current, &mut visible, None);
        assert_eq!(visible, "");
        transition_room_draft(&mut drafts, &mut current, &mut visible, room);
        assert_eq!(visible, "keep me");
    }

    #[test]
    fn pending_send_completion_clears_only_its_room_draft() {
        let room_a = room_draft_identity("https://one.example".into(), Some("a".into())).unwrap();
        let room_b = room_draft_identity("https://one.example".into(), Some("b".into())).unwrap();
        let mut drafts = RoomDrafts::from([
            (room_a.clone(), "same text".into()),
            (room_b.clone(), "same text".into()),
        ]);
        let mut pending_sends = PendingRoomSends::from([(
            room_a.clone(),
            PendingRoomSend {
                original_draft: "same text".into(),
                wire: "same text".into(),
                sent_at_seq: 0,
            },
        )]);
        let mut visible = "same text".into();
        let mut current = Some(room_b.clone());

        assert!(pending_sends.contains_key(&room_a));
        assert!(!pending_sends.contains_key(&room_b));

        assert!(clear_confirmed_room_draft(
            &mut drafts,
            Some(&room_b),
            &mut visible,
            &room_a,
            "same text",
        ));
        assert_eq!(visible, "same text");
        assert_eq!(drafts.get(&room_b).map(String::as_str), Some("same text"));
        assert_eq!(drafts.get(&room_a).map(String::as_str), Some(""));
        transition_room_draft(
            &mut drafts,
            &mut current,
            &mut visible,
            Some(room_a.clone()),
        );
        assert_eq!(visible, "");
        pending_sends.remove(&room_a);
        assert!(!pending_sends.contains_key(&room_a));
        assert!(!clear_confirmed_room_draft(
            &mut drafts,
            current.as_ref(),
            &mut visible,
            &room_a,
            "same text",
        ));
    }

    #[test]
    fn successful_send_clears_an_unchanged_same_room_draft_once() {
        let room = room_draft_identity("https://one.example".into(), Some("a".into())).unwrap();
        let mut drafts = RoomDrafts::from([(room.clone(), "sent text".into())]);
        let mut visible = "sent text".into();

        assert!(clear_confirmed_room_draft(
            &mut drafts,
            Some(&room),
            &mut visible,
            &room,
            "sent text",
        ));
        assert_eq!(visible, "");
        assert!(!clear_confirmed_room_draft(
            &mut drafts,
            Some(&room),
            &mut visible,
            &room,
            "sent text",
        ));
    }

    #[test]
    fn pending_http_failure_after_navigation_restores_retryable_draft() {
        let room_a = room_draft_identity("https://one.example".into(), Some("a".into())).unwrap();
        let room_b = room_draft_identity("https://one.example".into(), Some("b".into())).unwrap();
        let mut drafts = RoomDrafts::from([
            (room_a.clone(), "possibly sent".into()),
            (room_b.clone(), "draft B".into()),
        ]);
        let mut pending = PendingRoomSends::from([(
            room_a.clone(),
            PendingRoomSend {
                original_draft: "possibly sent".into(),
                wire: "possibly sent".into(),
                sent_at_seq: 0,
            },
        )]);
        let mut active = Some(room_a.clone());
        let mut visible = "possibly sent".into();
        let sent_generation = 7;

        // A's request is fenced when navigation advances its generation. Its
        // failure may arrive while B is open, so keep the draft but retire the
        // latch that could otherwise leave A disabled on return.
        assert!(retire_pending_room_send(&mut pending, &room_a));
        assert!(!crate::rooms::room_request_is_current(
            sent_generation,
            sent_generation + 1,
            "a",
            Some("b"),
        ));
        let late_http_failure = crate::rooms::room_request_is_current(
            sent_generation,
            sent_generation + 1,
            "a",
            Some("b"),
        )
        .then_some("message failed: HTTP 500");
        assert_eq!(late_http_failure, None);
        transition_room_draft(&mut drafts, &mut active, &mut visible, Some(room_b.clone()));
        assert_eq!(visible, "draft B");
        assert!(!pending.contains_key(&room_a));

        // The late A failure is discarded by Rooms' generation guard. A new
        // generation for the same identity restores text with no stale latch.
        transition_room_draft(&mut drafts, &mut active, &mut visible, Some(room_a.clone()));
        assert!(!crate::rooms::room_request_is_current(
            sent_generation,
            sent_generation + 2,
            "a",
            Some("a"),
        ));
        assert_eq!(visible, "possibly sent");
        assert!(!pending.contains_key(&room_a));
        assert!(message_send_admitted(false, false, true, &visible));
    }
}
