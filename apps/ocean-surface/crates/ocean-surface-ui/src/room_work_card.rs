//! Room agent work cards (team-platform P3).
//!
//! One card per room-convened agent turn, driven by the daemon's
//! `room_agent_run` projection. The header is always live (state, elapsed,
//! tool count, changed files); opening the card loads that turn's real steps
//! from the owner's agent session and renders them with the session
//! transcript's own `AssistantTurn` — the same tool groups, thinking, and
//! component blocks as direct chat. Steps are collapsed by default.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::daemon::Daemon;
use crate::model::{Block, Role, Turn};
use crate::rooms::{name_initials, RoomAgentRunState, Rooms};

/// Short status label for a run state (no instructional copy).
pub(crate) fn state_label(state: &RoomAgentRunState) -> String {
    match state {
        RoomAgentRunState::Queued => "queued".into(),
        RoomAgentRunState::Thinking => "thinking".into(),
        RoomAgentRunState::RunningTool { label } => label.clone(),
        RoomAgentRunState::AwaitingPermission => "needs approval".into(),
        RoomAgentRunState::AwaitingReply => "waiting on reply".into(),
        RoomAgentRunState::Done => "done".into(),
        RoomAgentRunState::Failed { reason } => format!("failed · {reason}"),
        RoomAgentRunState::Cancelled => "cancelled".into(),
        RoomAgentRunState::Unknown => "working".into(),
    }
}

/// Stable CSS state token for styling (`data-state`).
pub(crate) fn state_token(state: &RoomAgentRunState) -> &'static str {
    match state {
        RoomAgentRunState::Queued => "queued",
        RoomAgentRunState::Thinking | RoomAgentRunState::Unknown => "thinking",
        RoomAgentRunState::RunningTool { .. } => "tool",
        RoomAgentRunState::AwaitingPermission => "approval",
        RoomAgentRunState::AwaitingReply => "reply",
        RoomAgentRunState::Done => "done",
        RoomAgentRunState::Failed { .. } => "failed",
        RoomAgentRunState::Cancelled => "cancelled",
    }
}

/// Compact elapsed time: `4s`, `1m 07s`, `1h 02m`.
pub(crate) fn elapsed_label(ms: f64) -> String {
    let secs = (ms.max(0.0) / 1000.0).floor() as u64;
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// The assistant turns that answered the convene marked by `trigger_seq`.
///
/// Room agents keep one session per (room, agent), so a session holds many
/// convenes. The daemon's room prompt marks the convening line as
/// `[#<seq>] … «— mention`; the run's turns are the assistant turns after the
/// last user turn carrying that marker, up to the next user turn.
pub(crate) fn run_turns(turns: &[Turn], trigger_seq: u64) -> Vec<Turn> {
    let needle = format!("[#{trigger_seq}] ");
    let marks = |turn: &Turn| {
        turn.role == Role::User
            && turn.blocks.iter().any(|block| match block {
                Block::Text(text) => text
                    .lines()
                    .any(|line| line.starts_with(&needle) && line.ends_with("«— mention")),
                _ => false,
            })
    };
    let Some(start) = turns.iter().rposition(marks) else {
        return Vec::new();
    };
    turns[start + 1..]
        .iter()
        .take_while(|turn| turn.role == Role::Assistant)
        .cloned()
        .collect()
}

fn parse_ms(rfc3339: &str) -> f64 {
    js_sys::Date::parse(rfc3339)
}

/// One live work card for `run_id` in the open room.
#[component]
pub fn RoomWorkCard(run_id: String, rooms: Rooms, daemon: StoredValue<Daemon>) -> impl IntoView {
    let run_id = StoredValue::new(run_id);
    let run = Memo::new(move |_| {
        let id = run_id.get_value();
        rooms
            .runs
            .with(|runs| runs.iter().find(|r| r.run_id == id).cloned())
    });

    // Live elapsed clock while the run is active.
    let now = RwSignal::new(js_sys::Date::now());
    let ticker = set_interval_with_handle(
        move || now.set(js_sys::Date::now()),
        std::time::Duration::from_secs(1),
    )
    .ok();
    on_cleanup(move || {
        if let Some(handle) = ticker {
            handle.clear();
        }
    });
    let elapsed = move || {
        run.get().map(|r| {
            let start = parse_ms(&r.started_at);
            let end = if r.state.is_terminal() {
                parse_ms(&r.updated_at)
            } else {
                now.get()
            };
            elapsed_label(end - start)
        })
    };

    // Steps load on open and refresh as the run advances while open.
    let open = RwSignal::new(false);
    let steps: RwSignal<Vec<Turn>> = RwSignal::new(Vec::new());
    // Hoisted: a turbofish inside a `view!` attribute parses as a tag opener.
    let step_indices = move || -> Vec<usize> { (0..steps.with(Vec::len)).collect() };
    let steps_error = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(r) = run.get() else { return };
        let base = rooms.url.get_untracked();
        spawn_local(async move {
            match crate::daemon::fetch_session_turns(&base, &r.session_id).await {
                Ok(turns) => {
                    steps.set(run_turns(&turns, r.trigger_seq));
                    steps_error.set(None);
                }
                Err(e) => steps_error.set(Some(e)),
            }
        });
    });

    // Team-platform P4: the owner decides a pending tool permission here.
    // Nothing is applied optimistically; the next run frame clears the row.
    let deciding = RwSignal::new(false);
    let decide_error = RwSignal::new(None::<String>);
    let decide = move |permission: crate::rooms::RoomRunPermission, allow: bool| {
        if deciding.get_untracked() {
            return;
        }
        let Some(key) = rooms.open_key.get_untracked() else {
            return;
        };
        let base = rooms.url.get_untracked();
        let id = run_id.get_value();
        deciding.set(true);
        decide_error.set(None);
        spawn_local(async move {
            let result =
                crate::rooms::decide_run_permission(&base, &key, &id, &permission, allow).await;
            deciding.set(false);
            if let Err(e) = result {
                decide_error.set(Some(e));
            }
        });
    };

    view! {
        {move || run.get().map(|r| {
            let agent = crate::rooms::author_display_name(
                rooms.open_room.get().as_ref(),
                rooms.access.get().as_ref(),
                &r.agent_id,
            );
            let initials = name_initials(&agent);
            let token = state_token(&r.state);
            let label = state_label(&r.state);
            let active = !r.state.is_terminal();
            let files = r.files_changed.clone();
            let file_count = files.len();
            let tool_count = r.tool_count;
            let summary = r.summary.clone();
            let pending = matches!(r.state, RoomAgentRunState::AwaitingPermission)
                .then(|| r.pending_permission.clone())
                .flatten();
            view! {
                <section
                    class="room-card"
                    data-state=token
                    aria-label=format!("{agent} work")
                >
                    <button
                        class="room-card__head"
                        type="button"
                        aria-expanded=move || open.get().to_string()
                        on:click=move |_| open.update(|o| *o = !*o)
                    >
                        <span class="room-card__avatar" aria-hidden="true">{initials}</span>
                        <span class="room-card__agent">{agent.clone()}</span>
                        <span class="room-card__state">
                            <span class="room-card__dot" class:is-live=active aria-hidden="true"></span>
                            <span class="room-card__label">{label}</span>
                        </span>
                        <span class="room-card__meta">
                            {(tool_count > 0).then(|| view! {
                                <span class="room-card__stat" title="tool calls">{tool_count}</span>
                            })}
                            {(file_count > 0).then(|| view! {
                                <span class="room-card__stat room-card__stat--files" title="files changed">
                                    {format!("{file_count} file{}", if file_count == 1 { "" } else { "s" })}
                                </span>
                            })}
                            <span class="room-card__elapsed">{elapsed}</span>
                        </span>
                        <span class="room-card__chevron" class:is-open=move || open.get() aria-hidden="true">
                            <crate::icons::ChevronDown />
                        </span>
                    </button>
                    {pending.map(|p| {
                        let allow_p = p.clone();
                        let deny_p = p.clone();
                        view! {
                        <div class="room-card__approval" role="group" aria-label="Permission request">
                            <span class="room-card__approval-tool">{p.tool_label}</span>
                            <button
                                class="room-card__decide room-card__decide--allow"
                                type="button"
                                aria-label="Approve"
                                title="Approve"
                                disabled=move || deciding.get()
                                on:click=move |_| decide(allow_p.clone(), true)
                            >
                                <crate::icons::Check />
                            </button>
                            <button
                                class="room-card__decide room-card__decide--deny"
                                type="button"
                                aria-label="Deny"
                                title="Deny"
                                disabled=move || deciding.get()
                                on:click=move |_| decide(deny_p.clone(), false)
                            >
                                <crate::icons::Close />
                            </button>
                        </div>
                        }
                    })}
                    {move || decide_error.get().map(|e| view! {
                        <div class="room-card__error room-card__error--decide" role="alert">{e}</div>
                    })}
                    // Chat-grade markdown; `markdown::render` textifies raw HTML.
                    {summary.map(|s| view! {
                        <div class="room-card__summary md" inner_html=crate::markdown::render(&s)></div>
                    })}
                    <Show when=move || open.get()>
                        {
                            let files = files.clone();
                            view! {
                                <div class="room-card__body">
                                    {(!files.is_empty()).then(|| view! {
                                        <ul class="room-card__files" aria-label="files changed">
                                            {files.iter().map(|f| view! {
                                                <li class="room-card__file">{f.clone()}</li>
                                            }).collect_view()}
                                        </ul>
                                    })}
                                    {move || steps_error.get().map(|e| view! {
                                        <div class="room-card__error">{e}</div>
                                    })}
                                    <div class="room-card__steps">
                                        <For
                                            each=step_indices
                                            key=|i| *i
                                            children=move |i| view! {
                                                <crate::transcript::AssistantTurn
                                                    idx=i
                                                    turns=steps
                                                    daemon=daemon.get_value()
                                                    detached=true
                                                />
                                            }
                                        />
                                    </div>
                                </div>
                            }
                        }
                    </Show>
                </section>
            }
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Turn {
        Turn::user(text)
    }

    fn assistant(text: &str) -> Turn {
        Turn {
            turn_id: None,
            role: Role::Assistant,
            blocks: vec![Block::Text(text.into())],
        }
    }

    fn prompt(seq: u64) -> String {
        format!("header\n--- recent room transcript ---\n[#{}] ada: earlier\n[#{seq}] ada: @helper fix it  «— mention\n--- end transcript ---", seq - 1)
    }

    #[test]
    fn run_turns_selects_the_convene_marked_by_trigger_seq() {
        let turns = vec![
            user(&prompt(3)),
            assistant("first answer"),
            user(&prompt(9)),
            assistant("second step"),
            assistant("second answer"),
            user(&prompt(12)),
            assistant("third"),
        ];
        let got: Vec<_> = run_turns(&turns, 9)
            .into_iter()
            .map(|t| match &t.blocks[0] {
                Block::Text(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(got, vec!["second step", "second answer"]);
        assert_eq!(run_turns(&turns, 3).len(), 1);
        assert!(run_turns(&turns, 99).is_empty());
        // `[#8] … ` context lines that are not the mention never match.
        assert!(run_turns(&turns, 8).is_empty());
    }

    #[test]
    fn labels_and_tokens_cover_every_state() {
        assert_eq!(state_label(&RoomAgentRunState::Queued), "queued");
        assert_eq!(
            state_label(&RoomAgentRunState::RunningTool {
                label: "bash cargo test".into()
            }),
            "bash cargo test"
        );
        assert_eq!(
            state_label(&RoomAgentRunState::Failed {
                reason: "boom".into()
            }),
            "failed · boom"
        );
        assert_eq!(
            state_token(&RoomAgentRunState::AwaitingPermission),
            "approval"
        );
        assert_eq!(state_token(&RoomAgentRunState::Unknown), "thinking");
    }

    #[test]
    fn elapsed_label_is_compact() {
        assert_eq!(elapsed_label(4_200.0), "4s");
        assert_eq!(elapsed_label(67_000.0), "1m 07s");
        assert_eq!(elapsed_label(3_720_000.0), "1h 02m");
        assert_eq!(elapsed_label(-5.0), "0s");
    }
}
