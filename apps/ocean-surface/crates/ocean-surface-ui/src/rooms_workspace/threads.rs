//! One-level thread helpers and the right-rail thread panel.

use std::collections::HashSet;

use leptos::prelude::*;

use super::access::access_allows_thread_writes;
use super::author_name;
use super::format::{avatar_identity_class, canonical_wire_clock_time};
use super::mentions::MentionState;
use crate::room_messages;
use crate::rooms::{RoomAccessState, RoomMessage, RoomMessageKind, Rooms};

/// Whether to show the "No messages yet" empty state in the transcript.
#[allow(dead_code)]
pub(super) fn show_transcript_empty(tail_is_live: bool, roots_empty: bool) -> bool {
    roots_empty && tail_is_live
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ThreadPartition {
    pub(super) roots: Vec<RoomMessage>,
    pub(super) replies: Vec<RoomMessage>,
}

pub(super) fn partition_thread_messages(
    transcript: &[RoomMessage],
    root_seq: u64,
) -> ThreadPartition {
    let mut roots = Vec::new();
    let mut replies = Vec::new();
    for message in transcript {
        match message.thread_parent_seq {
            None => roots.push(message.clone()),
            Some(parent) if parent == root_seq => replies.push(message.clone()),
            Some(_) => {}
        }
    }
    ThreadPartition { roots, replies }
}

pub(super) fn reply_count_for(transcript: &[RoomMessage], root_seq: u64) -> usize {
    transcript
        .iter()
        .filter(|message| message.thread_parent_seq == Some(root_seq))
        .count()
}

pub(super) fn thread_root_for(
    transcript: &[RoomMessage],
    root_seq: Option<u64>,
) -> Option<RoomMessage> {
    let root_seq = root_seq?;
    transcript
        .iter()
        .find(|message| message.seq == root_seq && message.thread_parent_seq.is_none())
        .cloned()
}

pub(super) fn sync_thread_selection(
    current_root_seq: Option<u64>,
    open_room_key: Option<&str>,
    transcript: &[RoomMessage],
) -> Option<u64> {
    let root_seq = current_root_seq?;
    open_room_key?;
    thread_root_for(transcript, Some(root_seq)).map(|_| root_seq)
}

pub(super) fn should_show_thread_button(message: &RoomMessage) -> bool {
    message.thread_parent_seq.is_none() && matches!(message.kind, RoomMessageKind::Message)
}

pub(super) fn is_thread_open(selected_thread_root_seq: Option<u64>, root_seq: u64) -> bool {
    selected_thread_root_seq == Some(root_seq)
}

/// Right-rail thread view: the root message, its direct replies, and the
/// thread reply composer. Draft, in-flight latch, and mention state are owned
/// by `RoomsWorkspace`.
#[component]
pub(super) fn ThreadPanel(
    rooms: Rooms,
    root: RoomMessage,
    member_ids: Memo<HashSet<String>>,
    thread_composer: RwSignal<String>,
    thread_send_in_flight: RwSignal<bool>,
    mention: MentionState,
    on_send: Callback<()>,
) -> impl IntoView {
    let root_seq = root.seq;
    let full_ts = root.created_at.clone();
    let root_is_system = matches!(
        root.kind,
        RoomMessageKind::System
            | RoomMessageKind::ParticipantJoined
            | RoomMessageKind::ParticipantLeft
    );
    view! {
        <div class="rooms-workspace__right-thread">
            <div class="rooms-workspace__right-thread-head">
                <p class="rooms-workspace__right-thread-title">"Thread"</p>
                <div class="rooms-workspace__right-thread-subtitle">
                    {format!("Replying to {}", root.author_id)}
                </div>
            </div>
            <div
                class="rooms-workspace__right-thread-transcript"
                role="log"
                aria-label="Thread replies"
            >
                <div
                    class="rooms-workspace__msg rooms-workspace__msg--thread-root"
                    class:rooms-workspace__msg--system=root_is_system
                >
                    <div class=if root_is_system {
                        "rooms-workspace__msg-avatar".to_string()
                    } else {
                        format!(
                            "rooms-workspace__msg-avatar {}",
                            avatar_identity_class(&root.author_id)
                        )
                    }>
                        {if root_is_system {
                            view! { <crate::icons::Waves /> }.into_any()
                        } else {
                            { let id = root.author_id.clone(); move || crate::rooms::name_initials(&author_name(rooms, &id)) }.into_any()
                        }}
                    </div>
                    <div class="rooms-workspace__msg-body">
                        <div class="rooms-workspace__msg-author">
                            <span class="rooms-workspace__msg-name">{ let id = root.author_id.clone(); move || author_name(rooms, &id) }</span>
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
                            {crate::room_markdown::body_view(root.body.clone(), member_ids)}
                        </div>
                    </div>
                </div>
                <For
                    each=move || partition_thread_messages(&rooms.transcript.get(), root_seq).replies
                    key=|m: &RoomMessage| m.seq
                    children=move |reply: RoomMessage| {
                        let full_ts = reply.created_at.clone();
                        let is_system = room_messages::is_compact_system_row(&reply);
                        view! {
                            <div
                                class="rooms-workspace__msg rooms-workspace__msg--thread-reply"
                                class:rooms-workspace__msg--system=is_system
                            >
                                <div class=if is_system {
                                    "rooms-workspace__msg-avatar".to_string()
                                } else {
                                    format!(
                                        "rooms-workspace__msg-avatar {}",
                                        avatar_identity_class(&reply.author_id)
                                    )
                                }>
                                    {if is_system {
                                        view! { <crate::icons::Waves /> }.into_any()
                                    } else {
                                        { let id = reply.author_id.clone(); move || crate::rooms::name_initials(&author_name(rooms, &id)) }.into_any()
                                    }}
                                </div>
                                <div class="rooms-workspace__msg-body">
                                    <div class="rooms-workspace__msg-author">
                                        <span class="rooms-workspace__msg-name">{ let id = reply.author_id.clone(); move || author_name(rooms, &id) }</span>
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
                                        {crate::room_markdown::body_view(reply.body.clone(), member_ids)}
                                    </div>
                                </div>
                            </div>
                        }
                    }
                />
            </div>
            <div class="rooms-workspace__composer rooms-workspace__composer--thread">
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
                        aria-label="Thread reply"
                        placeholder=move || {
                            if matches!(
                                rooms.access.get().map(|access| access.state),
                                Some(RoomAccessState::Live)
                            ) {
                                "Federated thread replies unavailable"
                            } else {
                                "Reply in thread…"
                            }
                        }
                        node_ref=mention.input_ref
                        role="combobox"
                        aria-autocomplete="list"
                        aria-controls="rooms-mention-listbox-thread"
                        aria-expanded=move || (!mention.items.get().is_empty()).to_string()
                        aria-activedescendant=move || {
                            mention.active_descendant("rooms-mention-thread-opt-")
                        }
                        prop:value=move || thread_composer.get()
                        on:input=move |ev| mention.on_input(thread_composer, &ev)
                        on:keydown=move |ev| mention.on_keydown(rooms, thread_composer, &ev)
                        on:blur=move |_| mention.ctx.set(None)
                        disabled=move || !access_allows_thread_writes(rooms.access.get().as_ref())
                    />
                    {mention.popup(
                        rooms,
                        thread_composer,
                        "rooms-mention-listbox-thread",
                        "rooms-mention-thread-opt-",
                    )}
                    <button
                        class="rooms-workspace__composer-send"
                        type="submit"
                        disabled=move || {
                            thread_send_in_flight.get()
                                || thread_composer.get().trim().is_empty()
                                || !access_allows_thread_writes(rooms.access.get().as_ref())
                        }
                    >
                        {move || if thread_send_in_flight.get() { "Sending…" } else { "Reply" }}
                    </button>
                </form>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_msg;
    use super::*;

    #[test]
    fn transcript_with_root_is_not_empty_when_live() {
        let msgs = vec![test_msg(1, "hello", None)];
        assert!(!show_transcript_empty(
            true,
            partition_thread_messages(&msgs, 0).roots.is_empty()
        ));
    }

    #[test]
    fn transcript_empty_is_hidden_until_tail_is_live() {
        assert!(!show_transcript_empty(false, true));
        assert!(show_transcript_empty(true, true));
        assert!(!show_transcript_empty(true, false));
    }

    #[test]
    fn thread_open_helper_is_exact() {
        assert!(is_thread_open(Some(7), 7));
        assert!(!is_thread_open(Some(8), 7));
        assert!(!is_thread_open(None, 7));
    }

    #[test]
    fn partition_thread_messages_separates_roots_and_direct_replies() {
        let transcript = vec![
            test_msg(1, "root 1", None),
            test_msg(2, "reply to 1", Some(1)),
            test_msg(3, "root 3", None),
            test_msg(4, "reply to 3", Some(3)),
            test_msg(5, "orphan nested", Some(2)),
        ];

        let partition = partition_thread_messages(&transcript, 1);
        assert_eq!(
            partition.roots.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(
            partition.replies.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(reply_count_for(&transcript, 3), 1);
        assert_eq!(reply_count_for(&transcript, 2), 1);
    }

    #[test]
    fn reply_only_append_keeps_root_keys_and_bumps_count() {
        // The timeline For is keyed by root seq: a reply-only append must
        // not churn root keys (children stay cached), which is exactly why
        // the thread-toggle label must read the count reactively at the
        // leaf — this locks both halves of that contract.
        let mut transcript = vec![test_msg(0, "root", None), test_msg(1, "other root", None)];
        let roots_before: Vec<u64> = partition_thread_messages(&transcript, 0)
            .roots
            .iter()
            .map(|m| m.seq)
            .collect();
        assert_eq!(reply_count_for(&transcript, 0), 0);

        transcript.push(test_msg(2, "reply", Some(0)));

        let roots_after: Vec<u64> = partition_thread_messages(&transcript, 0)
            .roots
            .iter()
            .map(|m| m.seq)
            .collect();
        assert_eq!(
            roots_before, roots_after,
            "reply-only appends must preserve root For keys"
        );
        assert_eq!(reply_count_for(&transcript, 0), 1);
        assert_eq!(reply_count_for(&transcript, 1), 0);
    }

    #[test]
    fn sync_thread_selection_clears_on_room_close_or_missing_root() {
        let transcript = vec![test_msg(1, "root 1", None), test_msg(2, "reply", Some(1))];
        assert_eq!(
            sync_thread_selection(Some(1), Some("room-1"), &transcript),
            Some(1)
        );
        assert_eq!(sync_thread_selection(Some(1), None, &transcript), None);
        assert_eq!(
            sync_thread_selection(Some(9), Some("room-1"), &transcript),
            None
        );
    }
}
