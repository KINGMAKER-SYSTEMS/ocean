//! Room agent work cards (team-platform P3).
//!
//! Every room-convened agent turn gets one [`RoomAgentRun`]: a mutable
//! projection over that turn's own agent-session events, stored by
//! ocean-store and pushed to room SSE tails as `room_agent_run` frames. It is
//! never a second transcript — transcript rows stay append-only, and tool
//! steps/diffs stay in the agent session, which only the owner's surfaces read.
//!
//! Runs are local to the owning daemon and never cross federation.
//!
//! P4 adds the agent's voice inside the run: `room_post_update` progress posts,
//! `room_ask` parking (`AwaitingReply`, resumed by a human thread reply), and
//! in-room approvals (`AwaitingPermission` + a per-run decision token that only
//! the owning daemon's `POST .../runs/{run_id}/permission` route presents).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use chrono::Utc;
use ocean_agent_sdk::AgentSessionId;
use ocean_core::{RoomAgentRun, RoomAgentRunState, RoomKey, RoomRunPermission};
use ocean_runtime::AgentEvent;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::persistent_rooms::{publish_room_access_wake, with_rooms};
use crate::AppState;

/// Most recent runs a room tail or `GET .../runs` projects.
pub(crate) const ROOM_RUNS_LIMIT: usize = 50;
/// Longest summary kept on a run (characters).
const SUMMARY_CHARS: usize = 280;
/// Longest tool label kept on a run (characters).
const LABEL_CHARS: usize = 60;

/// Per-run permission decision tokens, held only in daemon memory for the
/// life of the turn. A room turn's permission waiters are bound to its token,
/// so the generic `/v1/permissions/{id}/decision` route cannot resolve them
/// without it; the room run route looks it up here.
static RUN_DECISION_TOKENS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn tokens() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    RUN_DECISION_TOKENS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The live decision token for `run_id`, if its turn is still running.
pub(crate) fn run_decision_token(run_id: &str) -> Option<String> {
    tokens().get(run_id).cloned()
}

/// Close a parked `room_ask` run once a human has answered in its thread; the
/// answer convenes a fresh run on the same session.
pub(crate) fn close_answered(state: &AppState, mut run: RoomAgentRun) {
    if !run.state.is_parked() {
        return;
    }
    run.state = RoomAgentRunState::Done;
    run.updated_at = Utc::now();
    if let Err(e) = with_rooms(state, |store| store.put_room_agent_run(&run)) {
        tracing::warn!(room = %run.room_id, %e, "room agent run write failed");
        return;
    }
    publish_room_access_wake(state, &run.room_id);
}

/// Owns one run's projection and persists every change.
pub(crate) struct RunTracker {
    state: AppState,
    run: RoomAgentRun,
    cwd: String,
}

impl RunTracker {
    /// Create the run in `Queued` and publish it.
    pub(crate) fn start(
        state: AppState,
        room: RoomKey,
        agent_id: &str,
        session_id: AgentSessionId,
        trigger_seq: u64,
        thread_root_seq: u64,
        cwd: String,
    ) -> Self {
        let now = Utc::now();
        let mut tracker = Self {
            state,
            run: RoomAgentRun {
                run_id: Uuid::new_v4().to_string(),
                room_id: room,
                agent_id: agent_id.to_string(),
                session_id: session_id.to_string(),
                trigger_seq,
                thread_root_seq,
                state: RoomAgentRunState::Queued,
                started_at: now,
                updated_at: now,
                summary: None,
                files_changed: Vec::new(),
                tool_count: 0,
                reply_seq: None,
                pending_permission: None,
            },
            cwd,
        };
        tracker.save();
        tracker
    }

    pub(crate) fn is_parked(&self) -> bool {
        self.run.state.is_parked()
    }

    /// Mint and register this run's permission decision token.
    pub(crate) fn mint_decision_token(&self) -> String {
        let token = Uuid::new_v4().simple().to_string();
        tokens().insert(self.run.run_id.clone(), token.clone());
        token
    }

    /// The turn is blocked on `permission_id` for `tool`.
    pub(crate) fn awaiting_permission(&mut self, permission_id: String, tool: &str, args: &Value) {
        if self.run.state.is_terminal() {
            return;
        }
        self.run.pending_permission = Some(RoomRunPermission {
            permission_id,
            tool_label: tool_label(tool, args, &self.cwd),
        });
        self.run.state = RoomAgentRunState::AwaitingPermission;
        self.save();
    }

    /// The pending permission was decided (or cancelled).
    pub(crate) fn permission_resolved(&mut self) {
        if self.run.pending_permission.take().is_none() {
            return;
        }
        if !self.run.state.is_terminal() {
            self.run.state = RoomAgentRunState::Thinking;
        }
        self.save();
    }

    /// `room_post_update`: the agent's latest progress line becomes the
    /// card summary.
    pub(crate) fn posted_update(&mut self, text: &str) {
        if self.run.state.is_terminal() {
            return;
        }
        self.run.summary = summarize(text);
        self.save();
    }

    /// `room_ask`: park the run until a human answers in its thread.
    pub(crate) fn awaiting_reply(&mut self, question: &str, ask_seq: Option<u64>) {
        if self.run.state.is_terminal() {
            return;
        }
        self.run.summary = summarize(question);
        self.run.reply_seq = ask_seq;
        self.run.state = RoomAgentRunState::AwaitingReply;
        self.save();
    }

    fn save(&mut self) {
        self.run.updated_at = Utc::now();
        let run = self.run.clone();
        if let Err(e) = with_rooms(&self.state, |store| store.put_room_agent_run(&run)) {
            tracing::warn!(room = %run.room_id, %e, "room agent run write failed");
            return;
        }
        publish_room_access_wake(&self.state, &run.room_id);
    }

    /// Move to `next` unless the run is already terminal or unchanged.
    pub(crate) fn set_state(&mut self, next: RoomAgentRunState) {
        if self.run.state.is_terminal() || self.run.state.is_parked() || self.run.state == next {
            return;
        }
        self.run.state = next;
        self.save();
    }

    /// Record a tool call: bump the count, label the running step, and note a
    /// written path.
    pub(crate) fn tool_started(&mut self, name: &str, args: &Value) {
        if self.run.state.is_terminal() || self.run.state.is_parked() {
            return;
        }
        self.run.tool_count = self.run.tool_count.saturating_add(1);
        if let Some(path) = written_path(name, args, &self.cwd) {
            if !self.run.files_changed.contains(&path) {
                self.run.files_changed.push(path);
            }
        }
        self.run.state = RoomAgentRunState::RunningTool {
            label: tool_label(name, args, &self.cwd),
        };
        self.save();
    }

    pub(crate) fn finish_done(&mut self, reply: &str, reply_seq: Option<u64>) {
        tokens().remove(&self.run.run_id);
        if self.run.state.is_terminal() || self.run.state.is_parked() {
            return;
        }
        self.run.pending_permission = None;
        self.run.summary = summarize(reply);
        self.run.reply_seq = reply_seq;
        self.run.state = RoomAgentRunState::Done;
        self.save();
    }

    pub(crate) fn finish_failed(&mut self, reason: &str) {
        tokens().remove(&self.run.run_id);
        if self.run.state.is_terminal() {
            return;
        }
        self.run.pending_permission = None;
        self.run.state = RoomAgentRunState::Failed {
            reason: truncate(
                reason.lines().next().unwrap_or("turn failed"),
                SUMMARY_CHARS,
            ),
        };
        self.save();
    }
}

/// Fold this turn's own runtime events (its `PromptControl` event sink) into
/// the tracker. Room turns have no product SSE bridge, so the sink is the only
/// live source for the card. Ends when the turn drops its sender.
pub(crate) async fn watch_runtime_events(
    tracker: Arc<Mutex<RunTracker>>,
    mut events: mpsc::UnboundedReceiver<AgentEvent>,
) {
    while let Some(event) = events.recv().await {
        let mut tracker = match tracker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        apply_runtime_event(&mut tracker, &event);
    }
}

fn apply_runtime_event(tracker: &mut RunTracker, event: &AgentEvent) {
    match event {
        AgentEvent::ToolExecutionStart {
            tool_name, args, ..
        } => tracker.tool_started(tool_name, args),
        AgentEvent::TurnStart { .. }
        | AgentEvent::TextDelta { .. }
        | AgentEvent::ThinkingDelta { .. }
        | AgentEvent::ToolExecutionEnd { .. } => {
            tracker.set_state(RoomAgentRunState::Thinking);
        }
        _ => {}
    }
}

/// A short, human label for a running tool step.
pub(crate) fn tool_label(name: &str, args: &Value, cwd: &str) -> String {
    let detail = if let Some(path) = arg_path(args) {
        Some(relative(path, cwd))
    } else {
        ["command", "cmd", "pattern", "query", "url"]
            .iter()
            .find_map(|key| args.get(*key).and_then(Value::as_str))
            .map(|s| s.lines().next().unwrap_or("").trim().to_string())
    };
    let label = match detail.filter(|d| !d.is_empty()) {
        Some(d) => format!("{name} {d}"),
        None => name.to_string(),
    };
    truncate(&label, LABEL_CHARS)
}

/// The workspace-relative path a writing tool touches, if any.
pub(crate) fn written_path(name: &str, args: &Value, cwd: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let writes = ["edit", "write", "patch", "create", "apply"]
        .iter()
        .any(|verb| lower.contains(verb));
    if !writes {
        return None;
    }
    arg_path(args).map(|p| relative(p, cwd))
}

fn arg_path(args: &Value) -> Option<&str> {
    ["path", "file_path", "filePath", "file"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .filter(|p| !p.trim().is_empty())
}

fn relative(path: &str, cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    match path.strip_prefix(cwd) {
        Some(rest) if !cwd.is_empty() && rest.starts_with('/') => rest[1..].to_string(),
        _ => path.to_string(),
    }
}

/// First paragraph of a reply, bounded.
pub(crate) fn summarize(reply: &str) -> Option<String> {
    let first = reply.split("\n\n").map(str::trim).find(|p| !p.is_empty())?;
    Some(truncate(first, SUMMARY_CHARS))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_label_prefers_relative_path_then_command() {
        assert_eq!(
            tool_label("edit", &json!({"path": "/repo/src/lib.rs"}), "/repo"),
            "edit src/lib.rs"
        );
        assert_eq!(
            tool_label("bash", &json!({"command": "cargo test\n--x"}), "/repo"),
            "bash cargo test"
        );
        assert_eq!(tool_label("todo", &json!({}), "/repo"), "todo");
        assert!(
            tool_label("bash", &json!({"command": "x".repeat(200)}), "/")
                .chars()
                .count()
                <= 60
        );
    }

    #[test]
    fn written_path_only_for_writing_tools() {
        assert_eq!(
            written_path(
                "hashline_edit",
                &json!({"file_path": "/repo/a.rs"}),
                "/repo"
            ),
            Some("a.rs".into())
        );
        assert_eq!(
            written_path("read", &json!({"path": "/repo/a.rs"}), "/repo"),
            None
        );
        assert_eq!(
            written_path("write", &json!({"path": "/elsewhere/b.rs"}), "/repo"),
            Some("/elsewhere/b.rs".into())
        );
        assert_eq!(written_path("write", &json!({}), "/repo"), None);
    }

    #[test]
    fn summarize_takes_first_paragraph() {
        assert_eq!(
            summarize("\n\nFixed the bug.\nDetails.\n\nMore."),
            Some("Fixed the bug.\nDetails.".into())
        );
        assert_eq!(summarize("   "), None);
        assert!(summarize(&"y".repeat(400)).unwrap().chars().count() <= 280);
    }
}
