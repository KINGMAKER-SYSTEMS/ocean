//! Room-turn tools (team-platform P4).
//!
//! Built per room turn by `spawn_room_agent_turn` and handed to the runtime
//! through the opaque `AgentRuntime::admit_room_turn_tools` handle, so each tool is bound to its own
//! room, agent, thread root, and run — the model never names a room. Local
//! rooms only: federated threads are not writable yet.
//!
//! - `room_post_update { text }` posts a progress line into the run's thread
//!   and makes it the card summary. Bounded per run.
//! - `room_ask { question }` posts the question into the run's thread, parks
//!   the run in `AwaitingReply`, and ends the turn. The next human reply in
//!   that thread convenes the same `(room, agent)` session again; no in-memory
//!   wait is held, so a daemon restart cannot lose the question.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ocean_runtime::{AgentTool, AgentToolResult, SharedTool};
use serde_json::{json, Value};

use crate::persistent_rooms::append_authorized_room_agent_reply;
use crate::room_agent_authority::RoomAgentAdmission;
use crate::room_agent_runs::RunTracker;
use crate::AppState;
use ocean_agent_sdk::AgentSessionId;
use tokio_util::sync::CancellationToken;

/// Most progress posts one run may make.
pub(crate) const MAX_UPDATES_PER_RUN: u32 = 12;
/// Longest post or question (characters).
pub(crate) const MAX_POST_CHARS: usize = 2000;

/// Everything a room tool is bound to.
#[derive(Clone)]
pub(crate) struct RoomTurnBinding {
    pub(crate) state: AppState,
    pub(crate) admission: RoomAgentAdmission,
    pub(crate) session_id: AgentSessionId,
    pub(crate) cancel: CancellationToken,
    pub(crate) thread_root: u64,
    pub(crate) tracker: Arc<Mutex<RunTracker>>,
}

impl RoomTurnBinding {
    fn with_tracker<T>(&self, f: impl FnOnce(&mut RunTracker) -> T) -> T {
        let mut guard = match self.tracker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// Post `body` as the agent into the run's thread.
    fn post(&self, body: &str) -> Result<u64, String> {
        append_authorized_room_agent_reply(
            &self.state,
            &self.admission,
            body,
            Some(self.thread_root),
            self.session_id,
            &self.cancel,
        )
        .map(|m| m.seq)
        .map_err(|_| "could not post to the room".to_string())
    }
}

/// The two room tools for one turn.
pub(crate) fn room_turn_tools(binding: RoomTurnBinding) -> Vec<SharedTool> {
    vec![
        Arc::new(RoomPostUpdateTool {
            binding: binding.clone(),
            posted: AtomicU32::new(0),
        }),
        Arc::new(RoomAskTool { binding }),
    ]
}

/// Validate a model-supplied post body.
pub(crate) fn post_text(args: &Value, key: &str) -> Result<String, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return Err(format!("`{key}` is required"));
    }
    if text.chars().count() > MAX_POST_CHARS {
        return Err(format!(
            "`{key}` is longer than {MAX_POST_CHARS} characters"
        ));
    }
    Ok(text.to_string())
}

struct RoomPostUpdateTool {
    binding: RoomTurnBinding,
    posted: AtomicU32,
}

#[async_trait]
impl AgentTool for RoomPostUpdateTool {
    fn name(&self) -> &str {
        "room_post_update"
    }

    fn description(&self) -> &str {
        "Post a short progress update to the room thread you were called from. \
         Use it at real milestones of long work (found the cause, tests pass, \
         opened a PR) — not for every step. Your final answer is posted \
         automatically; do not repeat it here."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "One or two sentences of markdown."
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> Result<AgentToolResult, String> {
        let text = post_text(&args, "text")?;
        if self.posted.fetch_add(1, Ordering::SeqCst) >= MAX_UPDATES_PER_RUN {
            return Err(format!(
                "update limit reached ({MAX_UPDATES_PER_RUN} per turn); continue the work"
            ));
        }
        self.binding.post(&text)?;
        self.binding.with_tracker(|t| t.posted_update(&text));
        Ok(AgentToolResult::text("posted"))
    }
}

struct RoomAskTool {
    binding: RoomTurnBinding,
}

#[async_trait]
impl AgentTool for RoomAskTool {
    fn name(&self) -> &str {
        "room_ask"
    }

    fn description(&self) -> &str {
        "Ask the room a clarifying question when you cannot proceed safely \
         without an answer. The question is posted in your thread and your turn \
         ends; you are called again with the room's reply. Ask one specific \
         question, only when guessing would be costly."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question, in markdown."
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> Result<AgentToolResult, String> {
        let question = post_text(&args, "question")?;
        let seq = self.binding.post(&question)?;
        self.binding
            .with_tracker(|t| t.awaiting_reply(&question, Some(seq)));
        Ok(AgentToolResult {
            terminate: true,
            ..AgentToolResult::text(
                "Question posted. Your turn ends now; you will be called again with the reply.",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_text_trims_and_bounds() {
        assert_eq!(
            post_text(&json!({"text": "  hi \n"}), "text").unwrap(),
            "hi"
        );
        assert!(post_text(&json!({"text": "   "}), "text").is_err());
        assert!(post_text(&json!({}), "text").is_err());
        assert!(post_text(&json!({"text": "x".repeat(MAX_POST_CHARS + 1)}), "text").is_err());
        assert!(post_text(&json!({"question": "why?"}), "question").is_ok());
    }
}
