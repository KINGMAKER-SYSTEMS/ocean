//! Room attention (team-platform P6): mentions inbox, bounded room search,
//! per-room mute, and the "agent is working" line above the composer.
//!
//! Everything here is owner-local: the inbox, search, and mute prefs are
//! served by this Ocean's own daemon and never cross federation. Coworker
//! typing presence is deliberately absent — it would change what crosses
//! Bedrock and needs an operator decision first.

use gloo_net::http::Request;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

use crate::rooms::{
    author_display_name, encode, name_initials, Room, RoomAgentRun, RoomAgentRunState, RoomMessage,
    Rooms,
};

const UNREACHABLE: &str = "Ocean could not reach the room service.";

/// Why an inbox item is there. Mirrors the daemon's `reason` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxReason {
    Mention,
    Reply,
    #[serde(other)]
    Other,
}

/// One mention or thread reply addressed to this Ocean's owner.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InboxItem {
    pub room_id: String,
    #[serde(default)]
    pub room_name: String,
    pub reason: InboxReason,
    /// Display name resolved by the daemon from the item's room roster.
    #[serde(default)]
    pub author_name: String,
    pub message: RoomMessage,
}

/// Per-room prefs. Mirrors `ocean_core::RoomPrefs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomPrefs {
    #[serde(default)]
    pub muted: bool,
}

#[derive(Deserialize)]
struct InboxEnvelope {
    #[serde(default)]
    items: Vec<InboxItem>,
}

#[derive(Deserialize)]
struct SearchEnvelope {
    #[serde(default)]
    results: Vec<RoomMessage>,
}

#[derive(Deserialize)]
struct PrefsEnvelope {
    prefs: RoomPrefs,
}

/// Shortest query the daemon accepts.
pub(crate) const MIN_QUERY_CHARS: usize = 2;

/// A response belongs to the daemon, room admission and query that requested
/// it. A newer read or a changed scope retires both successful and failed reads.
pub(crate) struct AttentionResponseFence {
    pub(crate) origin: String,
    pub(crate) room: Option<(String, u64)>,
    pub(crate) query: Option<String>,
    pub(crate) ticket: u64,
}

impl AttentionResponseFence {
    pub(crate) fn publish<T: Send + Sync + 'static>(
        &self,
        rooms: Rooms,
        latest_ticket: RwSignal<u64>,
        current_query: Option<String>,
        slot: RwSignal<Option<T>>,
        value: T,
    ) -> bool {
        if latest_ticket.try_get_untracked() != Some(self.ticket)
            || rooms.url.try_get_untracked().as_deref() != Some(self.origin.as_str())
            || current_query != self.query
            || slot.is_disposed()
        {
            return false;
        }
        if let Some((key, generation)) = &self.room {
            if !rooms.room_is_current(*generation, key) {
                return false;
            }
        }
        slot.set(Some(value));
        true
    }
}

pub async fn fetch_inbox(base: &str) -> Result<Vec<InboxItem>, String> {
    match Request::get(&format!("{base}/v1/rooms/persistent/inbox?limit=50"))
        .send()
        .await
    {
        Ok(r) if r.ok() => r
            .json::<InboxEnvelope>()
            .await
            .map(|e| e.items)
            .map_err(|_| "Ocean returned an invalid inbox.".into()),
        Ok(_) => Err("Ocean could not load the inbox.".into()),
        Err(_) => Err(UNREACHABLE.into()),
    }
}

pub async fn search_room(base: &str, key: &str, query: &str) -> Result<Vec<RoomMessage>, String> {
    let url = format!(
        "{base}/v1/rooms/persistent/{}/search?q={}&limit=50",
        encode(key),
        encode(query.trim())
    );
    match Request::get(&url).send().await {
        Ok(r) if r.ok() => r
            .json::<SearchEnvelope>()
            .await
            .map(|e| e.results)
            .map_err(|_| "Ocean returned invalid search results.".into()),
        Ok(_) => Err("Ocean could not search this room.".into()),
        Err(_) => Err(UNREACHABLE.into()),
    }
}

pub async fn put_prefs(base: &str, key: &str, prefs: RoomPrefs) -> Result<RoomPrefs, String> {
    let body = serde_json::to_string(&prefs).map_err(|_| UNREACHABLE.to_string())?;
    let request = Request::put(&format!("{base}/v1/rooms/persistent/{}/prefs", encode(key)))
        .header("content-type", "application/json")
        .body(body)
        .map_err(|_| UNREACHABLE.to_string())?;
    match request.send().await {
        Ok(r) if r.ok() => r
            .json::<PrefsEnvelope>()
            .await
            .map(|e| e.prefs)
            .map_err(|_| "Ocean returned invalid prefs.".into()),
        Ok(_) => Err("Ocean could not save this room's prefs.".into()),
        Err(_) => Err(UNREACHABLE.into()),
    }
}

/// Agents with a run in progress (not finished, not parked on a question).
pub(crate) fn working_agents(runs: &[RoomAgentRun]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for run in runs {
        let working = !run.state.is_terminal()
            && !matches!(
                run.state,
                RoomAgentRunState::AwaitingReply | RoomAgentRunState::AwaitingPermission
            );
        if working && !out.contains(&run.agent_id) {
            out.push(run.agent_id.clone());
        }
    }
    out
}

/// The thread a message belongs to (its root seq).
pub(crate) fn thread_root_of(message: &RoomMessage) -> u64 {
    message.thread_parent_seq.unwrap_or(message.seq)
}

/// A bounded one-line excerpt of `body` around the first match of `query`.
pub(crate) fn excerpt(body: &str, query: &str, max_chars: usize) -> String {
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = flat.chars().count();
    if total <= max_chars {
        return flat;
    }
    let lower = flat.to_lowercase();
    let q = query.trim().to_lowercase();
    let hit_char = if q.is_empty() {
        0
    } else {
        lower
            .find(&q)
            .map(|byte| lower[..byte].chars().count())
            .unwrap_or(0)
    };
    let start = hit_char.saturating_sub(max_chars / 3);
    let start = start.min(total.saturating_sub(max_chars));
    let mut out: String = flat.chars().skip(start).take(max_chars).collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + max_chars < total {
        out.push('…');
    }
    out
}

/// An inbox author's display name from the room list's rosters; the raw id
/// when the room or participant is unknown.
pub(crate) fn inbox_author_name(list: &[Room], room_id: &str, author_id: &str) -> String {
    list.iter()
        .find(|r| r.id == room_id)
        .and_then(|r| r.participants.iter().find(|p| p.id == author_id))
        .map(|p| p.display_name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| author_id.to_string())
}

/// Compact local time for list rows (`HH:MM`), from an RFC 3339 stamp.
fn short_time(rfc3339: &str) -> String {
    rfc3339
        .split('T')
        .nth(1)
        .map(|t| t.chars().take(5).collect())
        .unwrap_or_default()
}

/// "Helper is working" above the composer while an agent run is active.
#[component]
pub fn WorkingIndicator(rooms: Rooms) -> impl IntoView {
    let names = Memo::new(move |_| {
        let agents = rooms.runs.with(|runs| working_agents(runs));
        agents
            .iter()
            .map(|id| {
                author_display_name(
                    rooms.open_room.get().as_ref(),
                    rooms.access.get().as_ref(),
                    id,
                )
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="room-working" role="status" aria-live="polite">
            {move || {
                let names = names.get();
                (!names.is_empty()).then(|| {
                    let who = match names.len() {
                        1 => format!("{} is working", names[0]),
                        2 => format!("{} and {} are working", names[0], names[1]),
                        n => format!("{} agents are working", n),
                    };
                    view! {
                        <span class="room-working__dots" aria-hidden="true">
                            <span></span><span></span><span></span>
                        </span>
                        <span class="room-working__label">{who}</span>
                    }
                })
            }}
        </div>
    }
}

/// Mentions and replies to the owner across rooms (left rail).
#[component]
pub fn RoomInboxPanel(rooms: Rooms, open: RwSignal<bool>) -> impl IntoView {
    let items = RwSignal::new(None::<Result<Vec<InboxItem>, String>>);
    let ticket = RwSignal::new(0u64);
    Effect::new(move |_| {
        let active = open.get();
        let base = rooms.url.get();
        let mine = ticket.get_untracked().wrapping_add(1);
        ticket.set(mine);
        items.set(None);
        if !active {
            return;
        }
        let fence = AttentionResponseFence {
            origin: base.clone(),
            room: None,
            query: None,
            ticket: mine,
        };
        spawn_local(async move {
            let found = fetch_inbox(&base).await;
            if open.try_get_untracked() == Some(true) {
                fence.publish(rooms, ticket, None, items, found);
            }
        });
    });
    view! {
        <div class="room-inbox" role="list" aria-label="Mentions">
            {move || match items.get() {
                None => ().into_any(),
                Some(Err(e)) => view! {
                    <div class="rooms-workspace__left-empty rooms-workspace__left-empty--error" role="alert">{e}</div>
                }.into_any(),
                Some(Ok(list)) if list.is_empty() => view! {
                    <div class="room-inbox__empty" aria-label="No mentions"></div>
                }.into_any(),
                Some(Ok(list)) => list
                    .into_iter()
                    .map(|item| {
                        let room_id = item.room_id.clone();
                        let root = thread_root_of(&item.message);
                        // The inbox spans rooms that are not open: resolve
                        // names from the room list's rosters.
                        let author = if item.author_name.trim().is_empty() {
                            rooms.list.with_untracked(|list| {
                                inbox_author_name(list, &item.room_id, &item.message.author_id)
                            })
                        } else {
                            item.author_name.clone()
                        };
                        let initials = name_initials(&author);
                        let room_label = if item.room_name.is_empty() {
                            item.room_id.clone()
                        } else {
                            item.room_name.clone()
                        };
                        view! {
                            <button
                                class="room-inbox__item"
                                type="button"
                                role="listitem"
                                data-reason=match item.reason {
                                    InboxReason::Mention => "mention",
                                    InboxReason::Reply => "reply",
                                    InboxReason::Other => "other",
                                }
                                on:click=move |_| {
                                    open.set(false);
                                    rooms.open_room(room_id.clone());
                                    rooms.focus_thread.set(Some((rooms.url.get_untracked(), room_id.clone(), root)));
                                }
                            >
                                <span class="room-inbox__avatar" aria-hidden="true">{initials}</span>
                                <span class="room-inbox__main">
                                    <span class="room-inbox__meta">
                                        <span class="room-inbox__author">{author}</span>
                                        <span class="room-inbox__room">{format!("#{room_label}")}</span>
                                        <span class="room-inbox__time">{short_time(&item.message.created_at)}</span>
                                    </span>
                                    <span class="room-inbox__body">{excerpt(&item.message.body, "", 120)}</span>
                                </span>
                            </button>
                        }
                    })
                    .collect_view()
                    .into_any(),
            }}
        </div>
    }
}

/// Bounded server-side search of the open room (right rail).
#[component]
pub fn RoomSearchPanel(rooms: Rooms, on_pick: Callback<u64>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let results = RwSignal::new(None::<Result<Vec<RoomMessage>, String>>);
    let ticket = RwSignal::new(0u64);
    Effect::new(move |_| {
        let _ = rooms.url.get();
        let _ = rooms.open_key.get();
        ticket.update(|t| *t = t.wrapping_add(1));
        query.set(String::new());
        results.set(None);
    });
    let run = move || {
        let q = query.get_untracked();
        if q.trim().chars().count() < MIN_QUERY_CHARS {
            results.set(None);
            return;
        }
        let Some(key) = rooms.open_key.get_untracked() else {
            return;
        };
        let base = rooms.url.get_untracked();
        let mine = ticket.get_untracked().wrapping_add(1);
        ticket.set(mine);
        let fence = AttentionResponseFence {
            origin: base.clone(),
            room: Some((key.clone(), rooms.generation_snapshot())),
            query: Some(q.clone()),
            ticket: mine,
        };
        spawn_local(async move {
            let found = search_room(&base, &key, &q).await;
            if let Some(current_query) = query.try_get_untracked() {
                fence.publish(rooms, ticket, Some(current_query), results, found);
            }
        });
    };
    let input_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let _ = el.focus();
        }
    });
    view! {
        <div class="room-search">
            <form
                class="room-search__form"
                role="search"
                on:submit=move |ev| {
                    ev.prevent_default();
                    run();
                }
            >
                <input
                    class="room-search__input"
                    type="search"
                    aria-label="Search this room"
                    placeholder="Search"
                    node_ref=input_ref
                    prop:value=move || query.get()
                    on:input=move |ev| {
                        ticket.update(|t| *t = t.wrapping_add(1));
                        results.set(None);
                        query.set(event_target_value(&ev));
                    }
                />
            </form>
            <div class="room-search__results" role="list" aria-label="Search results">
                {move || match results.get() {
                    None => ().into_any(),
                    Some(Err(e)) => view! { <div class="room-search__error" role="alert">{e}</div> }.into_any(),
                    Some(Ok(list)) if list.is_empty() => view! {
                        <div class="room-search__none" aria-label="No results"></div>
                    }.into_any(),
                    Some(Ok(list)) => {
                        let q = query.get_untracked();
                        list.into_iter()
                            .map(|m| {
                                let root = thread_root_of(&m);
                                let author = author_display_name(
                                    rooms.open_room.get_untracked().as_ref(),
                                    rooms.access.get_untracked().as_ref(),
                                    &m.author_id,
                                );
                                let text = excerpt(&m.body, &q, 140);
                                view! {
                                    <button
                                        class="room-search__hit"
                                        type="button"
                                        role="listitem"
                                        on:click=move |_| on_pick.run(root)
                                    >
                                        <span class="room-search__meta">
                                            <span class="room-search__author">{author}</span>
                                            <span class="room-search__time">{short_time(&m.created_at)}</span>
                                        </span>
                                        <span class="room-search__body">{text}</span>
                                    </button>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// Toggle the open room's mute pref, then refresh the list (which carries it).
pub(crate) fn toggle_mute(rooms: Rooms, muted_now: bool) {
    let Some(key) = rooms.open_key.get_untracked() else {
        return;
    };
    let base = rooms.url.get_untracked();
    let generation = rooms.generation_snapshot();
    spawn_local(async move {
        let result = put_prefs(&base, &key, RoomPrefs { muted: !muted_now }).await;
        if rooms.url.try_get_untracked().as_deref() != Some(base.as_str()) {
            return;
        }
        match result {
            Ok(_) => rooms.fetch_rooms_silent(),
            Err(e) if rooms.room_is_current(generation, &key) => rooms.status.set(e),
            Err(_) => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::{RoomMessageKind, RoomParticipantKind};

    fn run(agent: &str, state: RoomAgentRunState) -> RoomAgentRun {
        RoomAgentRun {
            run_id: format!("r-{agent}-{state:?}"),
            room_id: "room".into(),
            agent_id: agent.into(),
            session_id: "s".into(),
            trigger_seq: 1,
            thread_root_seq: 1,
            state,
            started_at: "2026-10-04T00:00:00Z".into(),
            updated_at: "2026-10-04T00:00:00Z".into(),
            summary: None,
            files_changed: Vec::new(),
            tool_count: 0,
            reply_seq: None,
            pending_permission: None,
        }
    }

    #[test]
    fn working_agents_excludes_finished_and_waiting_and_dedupes() {
        let runs = vec![
            run("helper", RoomAgentRunState::Thinking),
            run("helper", RoomAgentRunState::Queued),
            run("scout", RoomAgentRunState::Done),
            run("asker", RoomAgentRunState::AwaitingReply),
            run("gate", RoomAgentRunState::AwaitingPermission),
            run(
                "builder",
                RoomAgentRunState::RunningTool {
                    label: "bash".into(),
                },
            ),
        ];
        assert_eq!(working_agents(&runs), vec!["helper", "builder"]);
    }

    #[test]
    fn excerpt_centers_on_the_match_and_marks_cuts() {
        assert_eq!(excerpt("short  body\nhere", "body", 50), "short body here");
        let long = format!("{} needle {}", "a ".repeat(60), "b ".repeat(60));
        let out = excerpt(&long, "NEEDLE", 40);
        assert!(out.contains("needle"), "{out}");
        assert!(out.starts_with('…') && out.ends_with('…'), "{out}");
        assert!(out.chars().count() <= 42);
    }

    #[test]
    fn inbox_author_name_reads_the_room_roster() {
        let list: Vec<Room> = serde_json::from_str(
            r#"[{"id":"r1","name":"r1","participants":[{"id":"helper","kind":"agent","display_name":"Helper"}]}]"#,
        )
        .unwrap();
        assert_eq!(inbox_author_name(&list, "r1", "helper"), "Helper");
        assert_eq!(inbox_author_name(&list, "r1", "ghost"), "ghost");
        assert_eq!(inbox_author_name(&list, "r2", "helper"), "helper");
    }

    #[test]
    fn thread_root_of_uses_parent_when_threaded() {
        let mut m = RoomMessage {
            seq: 9,
            author_id: "ada".into(),
            author_kind: RoomParticipantKind::Human,
            kind: RoomMessageKind::Message,
            body: "hi".into(),
            created_at: "2026-10-04T05:33:00Z".into(),
            thread_parent_seq: None,
            federated: None,
        };
        assert_eq!(thread_root_of(&m), 9);
        m.thread_parent_seq = Some(4);
        assert_eq!(thread_root_of(&m), 4);
        assert_eq!(short_time(&m.created_at), "05:33");
    }

    #[test]
    fn inbox_and_prefs_decode_the_daemon_shapes() {
        let item: InboxItem = serde_json::from_str(
            r#"{"room_id":"r","room_name":"R","reason":"mention","author_name":"Ada","message":{"seq":3,"author_id":"ada","author_kind":"human","kind":"message","body":"@john hi","created_at":"2026-10-04T05:33:00Z"}}"#,
        )
        .unwrap();
        assert_eq!(item.reason, InboxReason::Mention);
        assert_eq!(item.author_name, "Ada");
        assert_eq!(item.message.seq, 3);
        let p: PrefsEnvelope =
            serde_json::from_str(r#"{"ok":true,"prefs":{"muted":true}}"#).unwrap();
        assert!(p.prefs.muted);
        assert_eq!(
            serde_json::to_string(&RoomPrefs { muted: false }).unwrap(),
            r#"{"muted":false}"#
        );
    }
}
