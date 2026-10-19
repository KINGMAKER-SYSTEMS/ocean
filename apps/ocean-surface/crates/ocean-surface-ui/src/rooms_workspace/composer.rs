//! Channel composer: send admission, draft clearing, and failed-outbox matching.

use leptos::prelude::*;

use super::access::access_allows_writes;
use super::mentions::MentionState;
use crate::rooms::{OutboxItemState, Rooms};

/// Whether the composer should clear after the normalized wire body is
/// confirmed. The current draft must still be the exact original draft so
/// typing that happened while the send was in flight is never discarded.
pub(super) fn should_clear_composer(current: &str, original_draft: &str) -> bool {
    !original_draft.is_empty() && current == original_draft
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

/// Channel composer row (input, mention popup, send) plus the room status
/// line. Draft, in-flight latch, and mention state are owned by
/// `RoomsWorkspace` so they survive the center rail re-rendering.
#[component]
pub(super) fn ChannelComposer(
    rooms: Rooms,
    composer: RwSignal<String>,
    send_in_flight: RwSignal<bool>,
    mention: MentionState,
    on_send: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="rooms-workspace__composer">
            <form
                class="rooms-workspace__composer-row"
                on:submit=move |ev| {
                    ev.prevent_default();
                    on_send.run(());
                }
            >
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
}
