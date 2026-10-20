//! Mid-turn steering through the real agent loop (`AgentConfig::steering`).
//!
//! A scripted provider pushes a steering message into the run's inbox while a
//! chosen round is streaming, which is exactly when an operator types into a
//! busy session. The tests pin where the message lands in the transcript,
//! that a run never ends while a message is waiting, and that the inbox
//! refuses messages once the run is over.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream;
use ocean_protocol::{
    AssistantMessage, AssistantMessageEvent, AssistantMessageEventStream, Content, Context,
    Message, Model, Provider, StopReason, StreamOptions, Usage,
};
use ocean_runtime::types::{AgentConfig, AgentEvent, AgentTool, AgentToolResult};
use ocean_runtime::{SteerRefusal, SteeringInbox, SteeringMessage};
use serde_json::Value;
use tokio::sync::mpsc;

type Round = Vec<AssistantMessageEvent>;

/// Scripted provider that records each request's context and, when round
/// `steer_on_call` starts, pushes `steer` into `inbox` (the operator typing
/// while that round runs).
struct SteeringProvider {
    rounds: Mutex<VecDeque<Round>>,
    contexts: Mutex<Vec<Context>>,
    calls: AtomicUsize,
    inbox: Arc<SteeringInbox>,
    steer_on_call: Option<usize>,
    steer: Vec<SteeringMessage>,
}

#[async_trait]
impl Provider for SteeringProvider {
    async fn stream(
        &self,
        model: &Model,
        context: &Context,
        _options: &StreamOptions,
    ) -> ocean_protocol::Result<AssistantMessageEventStream> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.contexts.lock().unwrap().push(context.clone());
        if self.steer_on_call == Some(call) {
            for message in &self.steer {
                self.inbox
                    .push(message.clone())
                    .expect("inbox open while running");
            }
        }
        let round = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("loop requested more rounds than scripted");
        let events = round
            .into_iter()
            .map(|mut event| {
                if let AssistantMessageEvent::Done { message, .. } = &mut event {
                    message.api.clone_from(&model.api);
                    message.provider.clone_from(&model.provider);
                    message.model.clone_from(&model.id);
                }
                Ok(event)
            })
            .collect::<Vec<_>>();
        Ok(Box::pin(stream::iter(events)))
    }
}

fn done(content: Vec<Content>, stop: StopReason) -> AssistantMessageEvent {
    AssistantMessageEvent::Done {
        reason: stop,
        message: AssistantMessage {
            content,
            api: "mock".into(),
            provider: "mock".into(),
            model: "mock".into(),
            usage: Usage::default(),
            stop_reason: stop,
            error_message: None,
            timestamp: 0,
        },
    }
}

fn tool_round() -> Round {
    vec![done(
        vec![Content::ToolCall {
            id: "call-1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"x": 1}),
        }],
        StopReason::ToolUse,
    )]
}

fn text_round(text: &str) -> Round {
    vec![done(vec![Content::text(text)], StopReason::Stop)]
}

fn steer(id: &str, text: &str) -> SteeringMessage {
    SteeringMessage {
        id: id.into(),
        text: text.into(),
    }
}

struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes its input"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn requires_permission(&self) -> bool {
        false
    }
    async fn execute(&self, _id: &str, args: Value) -> Result<AgentToolResult, String> {
        Ok(AgentToolResult::text(format!("echo: {args}")))
    }
}

struct Run {
    provider: Arc<SteeringProvider>,
    inbox: Arc<SteeringInbox>,
    messages: Vec<Message>,
    events: Vec<AgentEvent>,
}

async fn run(
    rounds: Vec<Round>,
    steer_on_call: Option<usize>,
    steer_messages: Vec<SteeringMessage>,
    max_turns: u32,
    pre_queued: Vec<SteeringMessage>,
) -> Run {
    let inbox = Arc::new(SteeringInbox::new());
    for message in pre_queued {
        inbox.push(message).unwrap();
    }
    let provider = Arc::new(SteeringProvider {
        rounds: Mutex::new(rounds.into()),
        contexts: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
        inbox: inbox.clone(),
        steer_on_call,
        steer: steer_messages,
    });
    let cfg = AgentConfig::new(Model::anthropic_claude_sonnet_4_6(), "test system")
        .with_tools(vec![Arc::new(EchoTool)])
        .with_max_turns(max_turns)
        .with_session_id("steer")
        .with_provider(provider.clone())
        .with_steering(inbox.clone());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result =
        ocean_runtime::run_agent_with_history(&cfg, vec![Message::user_text("start")], Some(tx))
            .await
            .expect("run succeeds");
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    Run {
        provider,
        inbox,
        messages: result.messages,
        events,
    }
}

fn user_text(message: &Message) -> Option<String> {
    match message {
        Message::User { content, .. } => Some(
            content
                .iter()
                .filter_map(|c| c.as_text().map(str::to_string))
                .collect::<String>(),
        ),
        _ => None,
    }
}

fn delivered(events: &[AgentEvent]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::SteeringDelivered { id, text, .. } => Some((id.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_message_steered_during_a_tool_round_lands_after_its_results() {
    let run = run(
        vec![tool_round(), text_round("done")],
        Some(0),
        vec![steer("s1", "also check the tests")],
        8,
        vec![],
    )
    .await;

    let contexts = run.provider.contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 2, "one tool round, one answer round");
    let second = &contexts[1].messages;
    let n = second.len();
    assert!(
        matches!(second[n - 2], Message::ToolResult(_)),
        "the round's tool result comes first: {second:?}"
    );
    assert_eq!(
        user_text(&second[n - 1]).as_deref(),
        Some("also check the tests"),
        "the steering message follows it, before the next model call"
    );
    assert_eq!(
        delivered(&run.events),
        vec![("s1".to_string(), "also check the tests".to_string())]
    );
    // Persisted right away, as its own checkpoint.
    assert!(run.events.iter().any(|event| matches!(
        event,
        AgentEvent::TurnCheckpoint { messages, .. }
            if messages.len() == 1 && user_text(&messages[0]).as_deref() == Some("also check the tests")
    )));
    assert!(run.inbox.is_closed());
    assert_eq!(
        run.inbox.push(steer("late", "too late")),
        Err(SteerRefusal::Closed),
        "a finished run refuses steering"
    );
}

#[tokio::test]
async fn a_message_steered_while_the_answer_streams_gets_one_more_round() {
    let run = run(
        vec![text_round("first answer"), text_round("second answer")],
        Some(0),
        vec![steer("s1", "actually, use tabs")],
        8,
        vec![],
    )
    .await;

    let contexts = run.provider.contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 2, "the run continued instead of ending");
    let second = &contexts[1].messages;
    let n = second.len();
    assert!(matches!(second[n - 2], Message::Assistant(_)));
    assert_eq!(
        user_text(&second[n - 1]).as_deref(),
        Some("actually, use tabs")
    );
    assert!(matches!(
        run.messages.last(),
        Some(Message::Assistant(a)) if a.content.iter().any(|c| c.as_text() == Some("second answer"))
    ));
    assert!(run.inbox.is_closed());
}

#[tokio::test]
async fn several_messages_land_in_order_in_one_boundary() {
    let run = run(
        vec![tool_round(), text_round("done")],
        Some(0),
        vec![steer("a", "first"), steer("b", "second")],
        8,
        vec![],
    )
    .await;
    let ids: Vec<String> = delivered(&run.events)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
    let second = run.provider.contexts.lock().unwrap()[1].messages.clone();
    let n = second.len();
    assert_eq!(user_text(&second[n - 2]).as_deref(), Some("first"));
    assert_eq!(user_text(&second[n - 1]).as_deref(), Some("second"));
}

#[tokio::test]
async fn nothing_is_delivered_before_the_first_round_completes() {
    // Queued before the loop starts (the steer raced the turn's start). It
    // must not ride the first request, which a pre-stream failover could
    // discard; it lands after the first round instead.
    let run = run(
        vec![text_round("a"), text_round("b")],
        None,
        vec![],
        8,
        vec![steer("early", "early note")],
    )
    .await;
    let contexts = run.provider.contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 2);
    assert!(
        contexts[0]
            .messages
            .iter()
            .all(|m| user_text(m).as_deref() != Some("early note")),
        "first request carries only the original prompt"
    );
    let second = &contexts[1].messages;
    assert_eq!(
        user_text(second.last().unwrap()).as_deref(),
        Some("early note")
    );
}

#[tokio::test]
async fn the_turn_limit_closes_the_inbox_with_the_message_undelivered() {
    let run = run(
        vec![tool_round()],
        Some(0),
        vec![steer("s1", "one more thing")],
        1,
        vec![],
    )
    .await;
    assert_eq!(run.provider.contexts.lock().unwrap().len(), 1);
    assert!(delivered(&run.events).is_empty());
    assert!(run.inbox.is_closed());
    assert_eq!(run.inbox.push(steer("x", "x")), Err(SteerRefusal::Closed));
}

#[test]
fn the_inbox_bounds_its_backlog_and_closes_only_when_idle() {
    let inbox = SteeringInbox::new();
    for i in 0..SteeringInbox::MAX_PENDING {
        inbox.push(steer(&i.to_string(), "m")).unwrap();
    }
    assert_eq!(inbox.push(steer("over", "m")), Err(SteerRefusal::Full));
    assert!(!inbox.close_if_idle(), "messages waiting: stays open");
    assert!(!inbox.is_closed());
    assert_eq!(inbox.take_pending().len(), SteeringInbox::MAX_PENDING);
    assert!(inbox.close_if_idle(), "nothing waiting: closes");
    assert!(inbox.close_if_idle(), "idempotent once closed");
    assert_eq!(inbox.push(steer("late", "m")), Err(SteerRefusal::Closed));

    let other = SteeringInbox::new();
    other.push(steer("kept", "m")).unwrap();
    let handed_back = other.close();
    assert_eq!(
        handed_back,
        vec![steer("kept", "m")],
        "close hands back the undelivered"
    );
    assert!(other.is_closed());
}
