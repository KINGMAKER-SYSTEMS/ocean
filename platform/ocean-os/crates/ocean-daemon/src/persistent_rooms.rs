use std::{
    collections::HashSet,
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use chrono::Utc;
use ocean_agent_sdk::{AgentSessionId, AgentTurnEvent};
use ocean_core::{
    evaluate_trigger_policy, PermissionMode, PromptRequest, PublicAgentDescriptor, RequestState,
    RoomAccessProjection, RoomAccessState, RoomAgentRun, RoomAgentSettings, RoomArtifactKind,
    RoomArtifactState, RoomKey, RoomMessage, RoomMessageKind, RoomParticipant, RoomParticipantKind,
    RoomReadCursorProjection, RoomReadCursorUpdateRequest, RoomTriggerEvent, RoomTriggerPolicy,
};
#[cfg(test)]
use ocean_core::{OutboxItemState, RoomOutboxItem};
use ocean_store::{ContextPolicy, RoomStore, ThreadAppendError};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::room_retirement;
use super::{
    build_room_prompt_control, core_sid, emit_session_changed, record_prompt_result,
    resolve_permission_waiter, sdk_sid, sse_until_shutdown, AppState, PermissionDecisionBody,
    PermissionId, PermissionWaitHook, SSE_KEEPALIVE_INTERVAL,
};
use crate::request_control::{
    attach_request_handle, cancel_permission_waiter, register_room_agent_request_checked,
    RoomAgentRequestAuthority,
};
use crate::room_agent_authority::{
    self, AdmissionTrigger, ApiError, RoomAgentAdmission, RoomOperationAuthority,
};
use crate::room_federation::{
    AgentRegistrationInput, FederatedTriggerDispatch, FederatedTriggerKind, IntentError,
};
use crate::yolo_settings::effective_permission_mode;

fn canonical_submitted_workspace_root(
    workspace_root: Option<String>,
) -> Result<Option<String>, ()> {
    let Some(workspace_root) = workspace_root
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let path = std::path::Path::new(&workspace_root);
    if !path.is_absolute() {
        return Err(());
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| ())?;
    if !canonical.is_dir() {
        return Err(());
    }
    canonical
        .to_str()
        .map(|value| Some(value.to_string()))
        .ok_or(())
}

fn invalid_workspace_root_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok": false, "error": "invalid_workspace_root"})),
    )
}

pub(super) fn persisted_room_workspace(workspace_root: &str) -> Option<String> {
    let stored = std::path::Path::new(workspace_root);
    if !stored.is_absolute() {
        return None;
    }
    let canonical = std::fs::canonicalize(stored).ok()?;
    if canonical != stored || !canonical.is_dir() {
        return None;
    }
    canonical.to_str().map(str::to_string)
}

/// Shared handle to the daemon's single durable room store. Every closure is
/// synchronous, and both adapters recover a poisoned mutex without holding the
/// guard across an await.
pub(super) type RoomStoreHandle = Arc<Mutex<ocean_store::SqliteRoomStore>>;

/// A room-scoped wake hint. It deliberately carries no transcript payload:
/// SQLite remains the durable authority and every subscriber pages the store
/// after a hint, closing lag and replay/live seam gaps without trusting the
/// bounded channel for delivery.
#[derive(Debug, Clone)]
pub(super) struct RoomWakeHint {
    room: RoomKey,
    seq: u64,
}

/// Daemon-wide bounded wake channel for durable room transcript tails.
#[derive(Clone)]
pub(super) struct RoomWakeBus {
    tx: broadcast::Sender<RoomWakeHint>,
}

impl Default for RoomWakeBus {
    fn default() -> Self {
        Self::new(256)
    }
}

impl RoomWakeBus {
    pub(super) fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    fn subscribe(&self) -> broadcast::Receiver<RoomWakeHint> {
        self.tx.subscribe()
    }

    #[cfg(test)]
    pub(super) fn test_subscribe(&self) -> broadcast::Receiver<RoomWakeHint> {
        self.subscribe()
    }

    fn publish(&self, room: &RoomKey, message: &RoomMessage) {
        let _ = self.tx.send(RoomWakeHint {
            room: room.clone(),
            seq: message.seq,
        });
    }

    #[cfg(test)]
    fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// Publish only after the store adapter has returned, which means the allocating
/// SQLite transaction has committed. A missing subscriber is harmless: hints
/// are advisory and reconnect/recovery always pages the durable log.
pub(super) fn publish_room_wake(state: &AppState, room: &RoomKey, message: &RoomMessage) {
    publish_room_wake_on(&state.room_wakes, room, message);
}

/// Post-commit transcript wake seam for sibling background producers.
/// SQLite remains authoritative; callers invoke this only after the store
/// transaction has returned successfully.
pub(super) fn publish_room_wake_on(wakes: &RoomWakeBus, room: &RoomKey, message: &RoomMessage) {
    wakes.publish(room, message);
}

// ── RoomAccessWakeBus: separate bounded channel for access projection hints ──

/// A room access-projection wake hint. Carries no payload: SQLite remains the
/// durable authority and every subscriber re-reads the access projection after
/// a hint. Separate from transcript `RoomWakeBus` so a heavy transcript SSE tail
/// does not back-pressure access-projection subscribers.
#[derive(Debug, Clone)]
pub(super) struct RoomAccessWakeHint {
    room: RoomKey,
}

#[derive(Debug, Clone)]
pub(super) struct RoomReadCursorWakeHint {
    room: RoomKey,
}

/// Daemon-wide bounded wake channel for room access projection changes.
#[derive(Clone)]
pub(super) struct RoomAccessWakeBus {
    tx: broadcast::Sender<RoomAccessWakeHint>,
}

impl Default for RoomAccessWakeBus {
    fn default() -> Self {
        Self::new(64)
    }
}

impl RoomAccessWakeBus {
    pub(super) fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<RoomAccessWakeHint> {
        self.tx.subscribe()
    }

    #[cfg(test)]
    pub(super) fn test_subscribe(&self) -> broadcast::Receiver<RoomAccessWakeHint> {
        self.subscribe()
    }

    #[cfg(test)]
    fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    fn publish(&self, room: &RoomKey) {
        let _ = self.tx.send(RoomAccessWakeHint { room: room.clone() });
    }
}

/// Daemon-wide bounded wake channel for room read-cursor projection changes.
#[derive(Clone)]
pub(super) struct RoomReadCursorWakeBus {
    tx: broadcast::Sender<RoomReadCursorWakeHint>,
}

impl Default for RoomReadCursorWakeBus {
    fn default() -> Self {
        Self::new(64)
    }
}

impl RoomReadCursorWakeBus {
    pub(super) fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<RoomReadCursorWakeHint> {
        self.tx.subscribe()
    }

    fn publish(&self, room: &RoomKey) {
        let _ = self.tx.send(RoomReadCursorWakeHint { room: room.clone() });
    }
}

pub(super) fn publish_room_read_cursor_wake(state: &AppState, room: &RoomKey) {
    publish_room_read_cursor_wake_on(&state.room_read_cursor_wakes, room);
}

pub(super) fn publish_room_read_cursor_wake_on(wakes: &RoomReadCursorWakeBus, room: &RoomKey) {
    wakes.publish(room);
}

/// Publish an access-projection wake hint only after the store adapter has
/// returned (the allocating SQLite transaction has committed).
pub(super) fn publish_room_access_wake(state: &AppState, room: &RoomKey) {
    publish_room_access_wake_on(&state.room_access_wakes, room);
}

/// Post-commit access-projection wake seam for sibling background producers.
/// The hint carries no payload; subscribers reread SQLite.
pub(super) fn publish_room_access_wake_on(wakes: &RoomAccessWakeBus, room: &RoomKey) {
    wakes.publish(room);
}

/// Append one durable transcript row and issue its post-commit wake hint.
fn append_room_message(
    state: &AppState,
    room: &RoomKey,
    author_id: &str,
    author_kind: RoomParticipantKind,
    kind: RoomMessageKind,
    body: &str,
) -> Result<RoomMessage, ocean_store::RoomStoreError> {
    let message = with_rooms(state, |store| {
        store.append_message(room, author_id, author_kind, kind, body, Utc::now())
    })?;
    publish_room_wake(state, room, &message);
    Ok(message)
}

/// Post a convened agent's answer back into a room (G3).
///
/// Two invariants live here, and neither is client- or caller-controllable:
///
/// 1. **Session attribution is daemon-derived, structurally.** The persisted
///    `session_id` is minted HERE via [`room_agent_session_id`] from the
///    (room, agent) pair — it is not a parameter, so no caller (and certainly
///    no request body) can attribute a row to a session it does not own.
/// 2. **Threading degrades, it never drops.** The agent answers *after* its
///    turn ran, so the parent row it should hang under may have been closed,
///    re-parented, or otherwise invalidated in the meantime. A typed
///    [`ThreadAppendError::InvalidThreadParent`] therefore does not fail the
///    reply: it is re-appended top-level (the pre-thread behaviour) and the
///    stale parent is logged. Only a real store error propagates.
#[cfg(test)]
pub(super) fn append_room_agent_reply(
    state: &AppState,
    room: &RoomKey,
    agent_id: &str,
    body: &str,
    thread_parent_seq: Option<u64>,
) -> Result<RoomMessage, ocean_store::RoomStoreError> {
    let session_id = room_agent_session_id(room, agent_id).to_string();
    let session_id = Some(session_id.as_str());
    let message = with_rooms(state, |store| {
        let first = store.append_message_threaded(
            room,
            agent_id,
            RoomParticipantKind::Agent,
            RoomMessageKind::Message,
            body,
            Utc::now(),
            thread_parent_seq,
            session_id,
        );
        match first {
            Ok(message) => Ok(message),
            Err(ThreadAppendError::Store(e)) => Err(e),
            Err(ThreadAppendError::InvalidThreadParent {
                parent_seq, reason, ..
            }) => {
                tracing::warn!(
                    room = %room,
                    agent = %agent_id,
                    parent_seq,
                    reason = %reason,
                    "stale thread parent for agent reply; posting top-level"
                );
                store
                    .append_message_threaded(
                        room,
                        agent_id,
                        RoomParticipantKind::Agent,
                        RoomMessageKind::Message,
                        body,
                        Utc::now(),
                        None,
                        session_id,
                    )
                    .map_err(ocean_store::RoomStoreError::from)
            }
        }
    })?;
    debug_assert_eq!(
        message.session_id.as_deref(),
        session_id,
        "agent reply must persist the daemon-derived session id"
    );
    publish_room_wake(state, room, &message);
    Ok(message)
}

fn require_current_output_on(
    store: &mut ocean_store::SqliteRoomStore,
    admission: &RoomAgentAdmission,
    cancel: &CancellationToken,
) -> Result<(), ocean_store::RoomStoreError> {
    if !cancel.is_cancelled()
        && room_agent_authority::current_binding_on(
            store,
            &admission.room,
            &admission.agent_member_id,
            admission.generation,
        )?
        .is_some_and(|binding| {
            binding.agent_definition_digest == admission.package.definition_digest
        })
        && !cancel.is_cancelled()
    {
        Ok(())
    } else {
        Err(ocean_store::RoomStoreError::UnknownAgentBinding {
            room: admission.room.clone(),
            agent: admission.agent_member_id.clone(),
        })
    }
}

pub(super) fn append_authorized_room_agent_reply(
    state: &AppState,
    admission: &RoomAgentAdmission,
    body: &str,
    thread_parent_seq: Option<u64>,
    session_id: AgentSessionId,
    cancel: &CancellationToken,
) -> Result<RoomMessage, ocean_store::RoomStoreError> {
    let session = session_id.to_string();
    let append = with_rooms(state, |store| {
        require_current_output_on(store, admission, cancel).map_err(ThreadAppendError::from)?;
        store.append_authorized_agent_reply(
            &admission.room,
            &admission.agent_member_id,
            admission.generation,
            &admission.admission_id,
            body,
            Utc::now(),
            thread_parent_seq,
            &session,
        )
    });
    let (reply, audit) = match append {
        Ok(messages) => messages,
        Err(ThreadAppendError::InvalidThreadParent { parent_seq, .. }) => {
            tracing::warn!(room = %admission.room, agent = %admission.agent_member_id,
                parent_seq, reason_code = "stale_thread_parent",
                "stale thread parent for authorized agent reply; posting top-level");
            with_rooms(state, |store| {
                require_current_output_on(store, admission, cancel)
                    .map_err(ThreadAppendError::from)?;
                store.append_authorized_agent_reply(
                    &admission.room,
                    &admission.agent_member_id,
                    admission.generation,
                    &admission.admission_id,
                    body,
                    Utc::now(),
                    None,
                    &session,
                )
            })
            .map_err(ocean_store::RoomStoreError::from)?
        }
        Err(ThreadAppendError::Store(error)) => return Err(error),
    };
    publish_room_wake(state, &admission.room, &reply);
    publish_room_wake(state, &admission.room, &audit);
    Ok(reply)
}

fn append_authorized_room_agent_failure(
    state: &AppState,
    admission: &RoomAgentAdmission,
    session_id: AgentSessionId,
    cancel: &CancellationToken,
) -> Result<(), ocean_store::RoomStoreError> {
    let (failure, audit) = with_rooms(state, |store| {
        require_current_output_on(store, admission, cancel)?;
        store.append_authorized_agent_failure(
            &admission.room,
            &admission.agent_member_id,
            admission.generation,
            &admission.admission_id,
            Utc::now(),
            &session_id.to_string(),
        )
    })?;
    publish_room_wake(state, &admission.room, &failure);
    publish_room_wake(state, &admission.room, &audit);
    Ok(())
}

pub(super) fn clamp_room_message_body(body: &str) -> std::borrow::Cow<'_, str> {
    const MARKER: &str = "\n\n[truncated: reply exceeded the room message limit]";
    if body.len() <= crate::room_federation::OUTBOUND_MESSAGE_BODY_LIMIT {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut cut = crate::room_federation::OUTBOUND_MESSAGE_BODY_LIMIT - MARKER.len();
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    std::borrow::Cow::Owned(format!("{}{MARKER}", &body[..cut]))
}

// ---- Persistent Rooms (OCEAN-65) -------------------------------------------
//
// These routes serve the *persistent* `Room` lifecycle: create, fetch, roster
// join/leave, post message, read transcript. They are intentionally additive and
// fully separate from ephemeral agent sessions and the caller-submitted
// `agent_turn` handler. Auto-convene delegates a daemon-internal prompt through
// the existing runtime/session/permission owners; this module does not replace
// their authority.
//
// Error shape mirrors `GET /v1/longhouse/topics/{topic_id}`: a typed `{ ok,
// error }` body, 400 on a bad key, 404 on an unknown room. The store maps to
// status codes in `room_store_error_response`.

/// Where the persistent-rooms SQLite DB lives. `OCEAN_DB_PATH` overrides the
/// whole path; otherwise it is `rooms.db` under the agent's config dir
/// (`ocean_agent::config_dir_from_env`), so the DB sits next to sessions and
/// projects under one config directory.
pub(super) fn room_db_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("OCEAN_DB_PATH") {
        return std::path::PathBuf::from(p);
    }
    ocean_agent::config_dir_from_env().join("rooms.db")
}

/// Run a closure with a locked room store behind a [`RoomStoreHandle`], recovering
/// a poisoned lock the same way [`with_rooms`] does. Synchronous: the guard is
/// dropped before this returns, so no `await` is ever held across the lock. Takes
/// the handle directly (rather than `&AppState`) so the call sink — which only
/// holds the `rooms` handle, not the whole state — can write through.
pub(super) fn with_rooms_handle<T>(
    rooms: &RoomStoreHandle,
    f: impl FnOnce(&mut ocean_store::SqliteRoomStore) -> T,
) -> T {
    let mut guard = match rooms.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

/// Run a closure with the locked room store, recovering a poisoned lock the same
/// way the longhouse handlers do (`into_inner`). Synchronous: the guard is
/// dropped before this returns, so no `await` is ever held across the lock.
pub(super) fn with_rooms<T>(
    state: &AppState,
    f: impl FnOnce(&mut ocean_store::SqliteRoomStore) -> T,
) -> T {
    let mut guard = match state.rooms.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

#[derive(Clone)]
struct DurableRoomHistorySource {
    authority: RoomOperationAuthority,
}

fn room_history_row(message: RoomMessage) -> ocean_agent::RoomHistoryRow {
    let author_kind = match message.author_kind {
        RoomParticipantKind::Human => ocean_agent::RoomHistoryAuthorKind::Human,
        RoomParticipantKind::Agent => ocean_agent::RoomHistoryAuthorKind::Agent,
        RoomParticipantKind::System => ocean_agent::RoomHistoryAuthorKind::System,
        RoomParticipantKind::Bot => ocean_agent::RoomHistoryAuthorKind::Bot,
        RoomParticipantKind::Tool => ocean_agent::RoomHistoryAuthorKind::Tool,
    };
    ocean_agent::RoomHistoryRow {
        seq: message.seq,
        author_id: rendered_author_id(message.author_id),
        author_kind,
        text: room_history_text(message.body, message.author_kind, message.kind),
    }
}

/// A member id that is safe to store and to render in a markdown surface: not
/// empty, bounded, and free of control characters and the square brackets a
/// link needs. `None` means refuse it on write and filter it on render.
pub(super) fn bounded_member_id(raw: &str) -> Option<&str> {
    (!raw.is_empty()
        && raw.chars().count() <= super::room_agent_authority::MEMBER_ID_MAX_CHARS
        && !raw.chars().any(|c| c.is_control() || c == '[' || c == ']'))
    .then_some(raw)
}

/// The id a response renders for a row's author. Rows written before the join
/// route bounded ids are permanent, and federated rows arrive from elsewhere,
/// so the bound is applied on the way out as well as on the way in.
pub(super) fn rendered_author_id(raw: String) -> String {
    if bounded_member_id(&raw).is_some() {
        raw
    } else {
        "[filtered]".to_string()
    }
}

/// Every audit `type` this renderer has a rule for. `ocean-store` is where the
/// writers live; `every_store_audit_writer_has_a_render_rule` scans its
/// sources and goes red when a writer mints a `room.*` type missing here.
#[cfg(test)]
const RENDERED_AUDIT_TYPES: &[&str] = &[
    "room.agent.admission",
    "room.agent.authority",
    "room.agent.bootstrap",
    "room.agent.output",
    "room.participant.retired",
    "room.profile.created",
    "room.profile.updated",
    "room.resource.granted",
    "room.resource.resumed",
    "room.resource.suspended",
    "room.resource.revoked",
];

/// A CLOSED whitelist, not a `room.` prefix. A structured system body whose
/// `type` is not listed renders as a fixed `[room audit]` line — never raw —
/// and the scan test above goes red until the new writer gets its own rule.
///
/// History, transcript/SSE projection and prompt context share this rule.
/// Summary execution remains outside this admission stage.
pub(super) fn room_history_text(
    body: String,
    author_kind: RoomParticipantKind,
    message_kind: RoomMessageKind,
) -> String {
    if author_kind != RoomParticipantKind::System || message_kind != RoomMessageKind::System {
        return body;
    }
    let audit_type = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    // The agent MEMBER id is roster-public (it is what people @mention), so
    // it may ride after the fixed label; operator principal ids, decision ids,
    // digests, and capability sets never do. A refused admission names its
    // reason code so a reader can tell "refused" from "admitted" without the
    // JSON — the surface's activity strip depends on exactly these shapes.
    let value = serde_json::from_str::<serde_json::Value>(&body).ok();
    let agent = value
        .as_ref()
        .and_then(|v| v.get("agent_member_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(bounded_member_id)
        .map(|a| format!(" {a}"))
        .unwrap_or_default();
    let refused = value
        .as_ref()
        .filter(|v| v.get("outcome").and_then(serde_json::Value::as_str) == Some("refused"))
        .and_then(|v| v.get("reason_code"))
        .and_then(serde_json::Value::as_str)
        .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
        .map(str::to_owned);
    match audit_type.as_deref() {
        Some("room.agent.admission") => match refused {
            Some(code) => format!("[room agent admission refused: {code}]{agent}"),
            None => format!("[room agent admission audit]{agent}"),
        },
        Some("room.agent.authority") => format!("[room agent authority audit]{agent}"),
        Some("room.agent.bootstrap") => format!("[room agent bootstrap audit]{agent}"),
        Some("room.agent.output") => format!("[room agent output audit]{agent}"),
        Some("room.participant.retired") => {
            let side = |field: &str| {
                value
                    .as_ref()
                    .and_then(|v| v.get(field))
                    .and_then(serde_json::Value::as_str)
                    .and_then(bounded_member_id)
                    .unwrap_or("?")
                    .to_string()
            };
            format!("Participant retired: {} -> {}", side("from"), side("to"))
        }
        Some("room.profile.created") => "Room profile created".into(),
        Some("room.profile.updated") => "Room profile updated".into(),
        Some("room.resource.granted") => "Folder shared".into(),
        Some("room.resource.resumed") => "Folder access resumed".into(),
        Some("room.resource.suspended") => "Folder access suspended".into(),
        Some("room.resource.revoked") => "Folder access revoked".into(),
        // Structured but unrecognized: a ledger row this renderer has no rule
        // for yet. Its ids and fields go to no audience raw.
        Some(_) => "[room audit]".into(),
        // Not a structured audit — a plain system notice (room closed, a
        // marker) that its writer already bounded. A JSON object that carries
        // no `type` is still structured, and is still never rendered raw.
        None if value.as_ref().is_some_and(serde_json::Value::is_object) => "[room audit]".into(),
        None => body,
    }
}

/// Collapse a known Room audit body to the same summary line the agent path
/// already gets, for a row on its way to a HUMAN client.
///
/// Two things are wrong with handing that body over raw. The dull one is that a
/// human reads a wall of serde_json where an agent reads one line. The sharp one
/// is that the audit interpolates the ids that ARRIVED, and ocean-surface
/// markdown-renders every row body, System included, so a row whose
/// `owner_member_id` reads `[click here](https://evil.co)` lands an
/// attacker-labelled link in a row the UI attributes to the room itself.
///
/// `room_agent_authority::validate_member_id` now refuses that id at both
/// mutation routes, so no NEW row can be minted carrying one. This projection
/// is not thereby redundant: rows written before that guard are permanent, the
/// store still accepts whatever an in-process caller hands it, and the body
/// interpolates the package and operator-principal ids too.
///
/// The repair belongs HERE and not in `ocean-store`: that audit row is a ledger,
/// and a store that quietly repaired `owner_member_id` would report the attempt
/// as something other than what was made (see `crates/ocean-store/AGENTS.md`).
/// It goes at the point each response is SHAPED rather than inside
/// `read_transcript_page`, which stays the one raw paging implementation all of
/// its consumers share. `build_room_prompt` calls `room_history_text` directly
/// for the admitted transcript tail from `authorized_room_transcript_context`.
/// Human reads, scoped history pages and admitted prompts share one renderer;
/// summary execution remains deferred.
///
/// The whitelist matches literal `type` values. Any new audit writer must add
/// its type and regression coverage here in the same change. Named in
/// `crates/ocean-store/AGENTS.md`.
fn projected_room_message(mut message: RoomMessage) -> RoomMessage {
    message.body = room_history_text(message.body, message.author_kind, message.kind);
    message.author_id = rendered_author_id(message.author_id);
    message
}

/// [`projected_room_message`] across a page, for the handlers that hand back a
/// whole `transcript` array.
fn projected_transcript(messages: Vec<RoomMessage>) -> Vec<RoomMessage> {
    messages.into_iter().map(projected_room_message).collect()
}

#[async_trait::async_trait]
impl ocean_agent::RoomHistorySource for DurableRoomHistorySource {
    async fn page(
        &self,
        scope: &ocean_agent::RoomHistoryScope,
        request: ocean_agent::RoomHistoryRequest,
    ) -> Result<ocean_agent::RoomHistoryPage, ocean_agent::RoomHistorySourceError> {
        if scope.room_key() != self.authority.room.as_str()
            || scope.agent_member_id() != self.authority.member
            || scope.generation() != self.authority.generation
        {
            return Err(ocean_agent::RoomHistorySourceError::AuthorityChanged);
        }
        let room = RoomKey::new(scope.room_key());
        let page = with_rooms_handle(&self.authority.rooms, |store| {
            if self.authority.cancel.is_cancelled() {
                return Err(ocean_store::RoomStoreError::UnknownAgentBinding {
                    room: room.clone(),
                    agent: scope.agent_member_id().into(),
                });
            }
            if room_agent_authority::current_binding_on(
                store,
                &room,
                scope.agent_member_id(),
                scope.generation(),
            )?
            .is_none_or(|binding| binding.agent_definition_digest != self.authority.digest)
            {
                return Err(ocean_store::RoomStoreError::UnknownAgentBinding {
                    room: room.clone(),
                    agent: scope.agent_member_id().into(),
                });
            }
            store.authorized_room_history_page(
                &room,
                scope.agent_member_id(),
                scope.generation(),
                request.before_seq(),
                request.limit(),
            )
        })
        .map_err(|error| match error {
            ocean_store::RoomStoreError::UnknownAgentBinding { .. }
            | ocean_store::RoomStoreError::AgentBindingStatusConflict { .. } => {
                ocean_agent::RoomHistorySourceError::AuthorityChanged
            }
            ocean_store::RoomStoreError::UnknownRoom(_) => {
                ocean_agent::RoomHistorySourceError::Unavailable
            }
            _ => ocean_agent::RoomHistorySourceError::Internal,
        })?;
        if self.authority.cancel.is_cancelled() {
            return Err(ocean_agent::RoomHistorySourceError::AuthorityChanged);
        }
        Ok(ocean_agent::RoomHistoryPage {
            rows: page.messages.into_iter().map(room_history_row).collect(),
            has_more: page.has_more,
        })
    }
}

/// Map a store error onto an HTTP status + typed JSON body.
pub(super) fn room_store_error_response(
    err: ocean_store::RoomStoreError,
) -> (StatusCode, Json<serde_json::Value>) {
    use ocean_store::RoomStoreError::*;
    let status = match &err {
        BadKey(_) => StatusCode::BAD_REQUEST,
        UnknownRoom(_) | UnknownParticipant { .. } => StatusCode::NOT_FOUND,
        RetiredParticipant { .. } => StatusCode::CONFLICT,
        AlreadyExists(_) | RoomNotLocal(_) | LocalRoomOwnerConflict { .. } => StatusCode::CONFLICT,
        // The room exists but is not federated: a client-side misuse of a
        // federation-only operation, not a server fault.
        RoomNotFederated(_) => StatusCode::CONFLICT,
        // The caller named an owner that is not a Human in this room's roster
        // (or gave an owner to a non-Agent). That is a malformed request, and
        // the store refused it having written nothing.
        InvalidAgentOwner { .. } => StatusCode::BAD_REQUEST,
        // A join that would re-kind an existing participant is a takeover, not
        // a reconnect. 409: the id is taken by a different kind of actor.
        ParticipantKindConflict { .. } => StatusCode::CONFLICT,
        // The caller read a stale artifact. 409 with the actual version in the
        // body is the whole contract: re-read and retry. Never a silent merge.
        ArtifactVersionConflict { .. } => StatusCode::CONFLICT,
        UnknownArtifact { .. } => StatusCode::NOT_FOUND,
        // A client naming collision is the most ordinary error this endpoint
        // sees. Before this it tripped the PK constraint and surfaced as a 500 —
        // a client mistake reported as a server fault.
        ArtifactAlreadyExists { .. } => StatusCode::CONFLICT,
        // Nothing to change is a client mistake, not a conflict to retry.
        ArtifactUnchanged { .. } => StatusCode::BAD_REQUEST,
        // A write that would leave the artifact untitled. Malformed, and no
        // version the caller could re-read would make it well formed.
        ArtifactTitleBlank { .. } => StatusCode::BAD_REQUEST,
        ParticipantRecordImmutable { .. } => StatusCode::CONFLICT,
        // An artifact attributed to someone not in the room is a lie, not a
        // server fault.
        ArtifactAuthorNotInRoster { .. } => StatusCode::FORBIDDEN,
        // A stale link, or a second delete of something already gone. The
        // caller is working from an out-of-date view of the room.
        UnknownAttachment { .. } => StatusCode::NOT_FOUND,
        // Same rule as an artifact author: a file attributed to somebody who is
        // not in the room is a lie, not a server fault.
        AttachmentUploaderNotInRoster { .. } => StatusCode::FORBIDDEN,
        // And the same rule again for the person the close marker names. The
        // room exists and the act is well formed; the caller is claiming to be
        // somebody who is not in it.
        RoomCloserNotInRoster { .. } => StatusCode::FORBIDDEN,
        // Retention was asked of a live room. Nothing was written, and no
        // re-read makes it right until somebody closes the room — a conflict
        // with the room's state, not a malformed request.
        RoomNotClosed(_) => StatusCode::CONFLICT,
        // Rooms Phase 1. Asking about a binding that was never authorized is a
        // 404, not a 403: the caller is inspecting, and there is nothing there.
        // Admission refusal on an absent binding is a separate path that never
        // reaches this mapping.
        UnknownAgentBinding { .. } => StatusCode::NOT_FOUND,
        // A decision replayed against different content, and any move out of
        // the terminal revoked state, are both "your view of authority is
        // stale" — re-read and issue a new decision.
        DecisionReplayMismatch { .. } | AgentBindingStatusConflict { .. } => StatusCode::CONFLICT,
        ResourceRootAlreadyGranted { .. } | ResourceStatusConflict { .. } => StatusCode::CONFLICT,
        UnknownResourceGrant { .. } => StatusCode::NOT_FOUND,
        // A durable backend can fail on I/O or (de)serialization, which the
        // in-memory registry never could. Surface those as 500s, not as a
        // misleading 4xx. Federation corruption is a fail-closed integrity
        // stop and never carries secrets in its message.
        Db(_) | Encode(_) | FederationCorruption(_) | Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(json!({ "ok": false, "error": err.to_string() })),
    )
}

// ---- Named-agent resolution seam (TASK-9 / OCEAN Rooms Gate-1) -------------
//
// A folder-as-agent definition resolved down to the four values a turn needs to
// actually DRIVE that agent: its instructions layer, tool allowlist, model, and
// tier-1 subprocess capabilities. This is the SINGLE named-agent resolution
// path shared by `agent_turn` (the folder-as-agent turn in `main.rs`) and the
// persistent-room convene path (`room_join` validation, the `room_post_message`
// footprint gate, and `spawn_room_agent_turn`). Binding truth flows one way:
// only an Agent participant that resolves to a real AgentDef may be convened;
// a default assistant is never silently substituted for an unresolved name.

/// A resolved folder-as-agent, reduced to the four turn-driving values. Each
/// field is independently `Option`, and a valid data-only agent (an
/// `agent.toml` that declares none of instructions/tools/model/caps)
/// legitimately resolves to all-four-`None` — that is NOT an error. Resolution
/// failure (empty name or an unresolvable folder) is signaled only by
/// [`resolve_named_agent`] returning `Err`, never by an all-`None` `Ok`.
#[derive(Debug, Clone)]
pub(super) struct ResolvedAgent {
    /// Trimmed `instructions.md` when the agent authored any. Prepended as a
    /// steering layer above the (guided) prompt, exactly as `agent_turn` does.
    pub(super) instructions_layer: Option<String>,
    /// `agent.toml` `tools` + `tools/` filename stems, non-empty only when the
    /// agent narrows its toolset. Applied via `PromptControl::with_tool_allowlist`.
    pub(super) tool_allowlist: Option<Vec<String>>,
    /// Declared per-agent model. Fail-soft to the global model when `None`/empty
    /// (the emptiness trim happens inside `PromptControl::with_agent_model`).
    pub(super) model: Option<String>,
    /// Declared tier-1 subprocess capabilities plus the agent root used to
    /// resolve relative commands. Non-empty only when the agent declares caps;
    /// applied via `PromptControl::with_agent_capabilities`.
    pub(super) subprocess_caps: Option<(
        std::path::PathBuf,
        Vec<ocean_agent::agentdir::SubprocessCapability>,
    )>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct RoomTurnCapture {
    agent_id: String,
    prompt: String,
    tool_allowlist: Option<Vec<String>>,
    model: Option<String>,
    subprocess_caps: Option<(
        std::path::PathBuf,
        Vec<ocean_agent::agentdir::SubprocessCapability>,
    )>,
    extra_tools: Vec<String>,
}

#[cfg(test)]
static ROOM_TURN_CAPTURES: Mutex<Vec<RoomTurnCapture>> = Mutex::new(Vec::new());

#[cfg(test)]
fn capture_room_turn(agent_id: &str, prompt: &str, control: &ocean_agent::PromptControl) {
    let capture = RoomTurnCapture {
        agent_id: agent_id.to_string(),
        prompt: prompt.to_string(),
        tool_allowlist: control.tool_allowlist.clone(),
        model: control.agent_model.clone(),
        subprocess_caps: control.agent_capabilities.clone(),
        extra_tools: control.room_turn_tool_names(),
    };
    match ROOM_TURN_CAPTURES.lock() {
        Ok(mut captures) => captures.push(capture),
        Err(poisoned) => poisoned.into_inner().push(capture),
    }
}

/// Resolve a named folder-as-agent to the four turn-driving values above.
///
/// Returns `Err` ONLY for an empty name or an `agentdir::resolve` failure
/// (missing folder, bad name, unparseable `agent.toml`). A resolved-but-
/// data-only agent returns `Ok` with all four fields `None` — that distinction
/// is load-bearing: all-`None` `Ok` is a real, bound agent that declares no
/// overrides, NOT a sentinel for "unresolved". Callers must branch on the
/// `Result`, never on the presence of any single field.
pub(super) fn resolve_named_agent(
    name: &str,
) -> Result<ResolvedAgent, ocean_agent::agentdir::ResolveError> {
    let def = ocean_agent::agentdir::resolve(&super::agents_root(), name)?;
    let instructions_layer = def.system_prompt().map(str::to_owned);
    let tool_allowlist = {
        let tools = def.effective_tools();
        (!tools.is_empty()).then_some(tools)
    };
    let model = def.config.model.clone();
    let subprocess_caps = {
        let caps = def.config.subprocess_capabilities.clone();
        (!caps.is_empty()).then(|| (def.root.clone(), caps))
    };
    Ok(ResolvedAgent {
        instructions_layer,
        tool_allowlist,
        model,
        subprocess_caps,
    })
}

fn resolve_agent_registration(name: &str) -> Option<(String, PublicAgentDescriptor)> {
    let def = ocean_agent::agentdir::resolve(&super::agents_root(), name).ok()?;
    let skills_count = u32::try_from(def.skills.len()).ok()?;
    let canonical = def.name.clone();
    Some((
        canonical.clone(),
        PublicAgentDescriptor {
            display_name: canonical,
            description: def.config.description.clone(),
            model_alias: def.config.model.clone(),
            skills_count,
            subagent_names: def.subagents.clone(),
        },
    ))
}

fn registration_key(instance_id: &str, room: &RoomKey, agent_name: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ocean-regkey-v1");
    for value in [instance_id, room.as_str(), agent_name] {
        let byte_len = u64::try_from(value.len()).expect("UTF-8 length fits frozen u64 prefix");
        digest.update(byte_len.to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

#[derive(serde::Deserialize)]
pub(super) struct RoomCreateRequest {
    /// Persistent room key, e.g. `"ocean-surface-map-fix"`. Must be non-empty.
    pub(super) key: String,
    /// Human-readable room name.
    pub(super) name: String,
    /// Optional trigger policy controlling auto-convene/notify behaviour.
    #[serde(default)]
    pub(super) trigger_policy: Option<RoomTriggerPolicy>,
    /// Optional workspace directory the room belongs to (OCEAN-260). When set,
    /// the room is bound to this project/cwd, so a room-bound agent turn resolves
    /// its owning project and `cwd` from it. Absent/empty ⇒ no binding (room
    /// agent execution is refused with `workspace_unavailable`).
    #[serde(default)]
    pub(super) workspace_root: Option<String>,
}

/// `POST /v1/rooms/persistent` — create a persistent room.
pub(super) async fn room_create(
    State(state): State<AppState>,
    Json(req): Json<RoomCreateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Confirmed build rows restore the original hint path. CI is marker-only;
    // preserve legacy flags on reads but refuse new CI activation.
    if req
        .trigger_policy
        .as_ref()
        .is_some_and(|policy| policy.on_ci_failure)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "trigger_policy_unwired",
                "error": "CI triggers are unavailable on this daemon",
            })),
        );
    }
    let key = RoomKey::new(req.key.trim());
    let workspace_root = match canonical_submitted_workspace_root(req.workspace_root) {
        Ok(root) => root,
        Err(()) => return invalid_workspace_root_response(),
    };
    let result = with_rooms(&state, |reg| {
        reg.create_in_workspace(
            key,
            &req.name,
            workspace_root,
            req.trigger_policy,
            Utc::now(),
        )
    });
    match result {
        Ok(rec) => (
            StatusCode::CREATED,
            Json(json!({ "ok": true, "room": rec.room })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent` — list all persistent rooms (no transcripts).
/// Pagination query for `GET /v1/rooms/persistent` (OCEAN-250).
#[derive(Debug, Default, Deserialize)]
pub(super) struct CloseRoomQuery {
    #[serde(default)]
    actor_id: Option<String>,
}

/// `POST /v1/rooms/persistent/{key}/close` — freeze the room.
///
/// The route the store has been waiting for. `closed_at` and the soft-closed
/// read paths (`get_including_closed`, both `*_including_closed` pagers,
/// `/snapshot`'s `closed` boolean) have existed for as long as durable rooms
/// have, but production reached them from exactly one place: `CallEnded`. A
/// persistent room a member made could be joined, written, summarized and
/// federated, and never ended — so rooms.db only ever grew, and the audit view
/// built to serve a frozen room served only finished calls.
///
/// **Two authorities, chosen by the header.** `X-Ocean-Operator` present means
/// the operator lane and every operator refusal applies (an invalid, cookie-
/// bearing, or foreign-origin credential is a 403; an unconfigured key is a
/// 503) — a presented credential is never silently downgraded to the member
/// lane, because a caller who tried to prove operator authority and failed must
/// not then be judged as whatever `?actor_id=` they also sent. Absent, the
/// route wants `?actor_id=` naming a roster member, roster-checked inside the
/// closing transaction and refused if it claims an Agent's or System's identity
/// — the same gate the attachment routes carry, ported for the same reason: an
/// agent does not close a room, and the marker this route writes would say it
/// did.
///
/// **Afterwards** the room is frozen, not gone: `GET /v1/rooms/persistent/{key}`
/// 404s, every write refuses, `/events` refuses a new connection and the tails
/// on existing ones end, and `/snapshot` answers `closed: true` with the
/// transcript still pageable in both directions and the roster and agent
/// ownership rows intact. Nothing here touches Bedrock — a federated room's
/// credential, outbox and access projection are untouched, and closing is a
/// LOCAL statement about a local room's lifecycle.
pub(super) async fn room_close(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    headers: HeaderMap,
    Query(query): Query<CloseRoomQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);

    // Presence of the header, not the outcome of authorizing it, is what picks
    // the lane. Asking `authorize` first and treating its `Missing` as "try the
    // member lane" would work today and would silently change meaning the day
    // that function grows another reason to refuse before it looks for a header.
    let operator_offered = headers.contains_key(crate::room_operator::OPERATOR_HEADER);
    let closer_id = if operator_offered {
        match state.room_operator.authorize(&headers) {
            Ok(principal) => CloserIdentity::Operator(principal.id().to_string()),
            Err(error) => return operator_refusal_response(error),
        }
    } else {
        let actor = query.actor_id.unwrap_or_default();
        let actor = actor.trim();
        if actor.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "code": "invalid_request",
                    "error": "closing a room needs ?actor_id= naming a roster member, or an X-Ocean-Operator credential",
                })),
            );
        }
        CloserIdentity::Member(actor.to_string())
    };

    // Match final admission's requests -> store lock order. No request can
    // register between the durable close and cancellation of existing turns.
    let mut requests = state.requests.write().await;
    // One lock acquisition for the forged-kind gate AND the close. The
    // attachment routes take two (their `forged_author_response` opens its own
    // guard), which leaves a window where a roster edit lands between the check
    // and the write; there is no reason to copy that here when the check has to
    // read the room the close is about anyway.
    let result = with_rooms(&state, |store| {
        if let CloserIdentity::Member(id) = &closer_id {
            let claimed_kind = store.get(&key)?.and_then(|record| {
                record
                    .room
                    .participants
                    .iter()
                    .find(|participant| &participant.id == id)
                    .map(|participant| participant.kind)
            });
            if matches!(
                claimed_kind,
                Some(RoomParticipantKind::Agent) | Some(RoomParticipantKind::System)
            ) {
                return Ok(Err(forged_closer_response()));
            }
        }
        let closer = match &closer_id {
            CloserIdentity::Member(id) => ocean_store::RoomCloser::Member(id),
            CloserIdentity::Operator(id) => ocean_store::RoomCloser::Operator(id),
        };
        store.close_with_marker(&key, closer, Utc::now()).map(Ok)
    });
    let cancelled = if matches!(&result, Ok(Ok(_))) {
        crate::room_agent_authority::cancel_room_requests_locked(&mut requests, &key)
    } else {
        Vec::new()
    };
    drop(requests);
    crate::room_agent_authority::cleanup_cancelled(&state, cancelled).await;

    match result {
        Ok(Ok((record, message))) => {
            // Post-commit, in the order every other room writer uses: the
            // transcript hint first so a tail flushes the close marker it is
            // about to stop on, then the access hint. Both are payload-free —
            // subscribers re-read SQLite, which is now the closed truth.
            publish_room_wake(&state, &key, &message);
            publish_room_access_wake(&state, &key);
            // Stop the room's federation task, if it has one.
            //
            // The STORE is what guarantees a closed room takes no more rows —
            // `ingest_confirmed_event` refuses one, atomically, however this
            // task is scheduled. This call is the operational half of that
            // refusal rather than a second copy of it: without it the
            // supervisor would keep an SSE epoch open against a room whose
            // every ingest now fails, and a store error there breaks the epoch
            // into `Recover`, so the room would reconnect-loop forever getting
            // the same refusal. Stopping it is also just true — a frozen room
            // has nothing left to receive.
            //
            // After the commit and after both wakes, so a tail flushes the
            // close marker first; and this handler holds no store guard here,
            // so awaiting is safe. Bedrock is not told anything: the room's
            // credential, outbox and access projection are untouched, and
            // closing stays a LOCAL statement about a local lifecycle.
            state.room_federation.stop_room(&key).await;
            tracing::info!(
                room = %key,
                closer_kind = closer_id.kind_label(),
                marker_seq = message.seq,
                "persistent room closed"
            );
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "room": record.room,
                    "closed": true,
                    "marker_seq": message.seq,
                })),
            )
        }
        Ok(Err(refusal)) => refusal,
        Err(e) => room_store_error_response(e),
    }
}

/// The verified closer, carried as owned strings because the store call happens
/// inside a closure that outlives the request's borrows.
enum CloserIdentity {
    Member(String),
    Operator(String),
}

impl CloserIdentity {
    /// For the log line only. The operator PRINCIPAL is not a secret and the
    /// marker quotes it, but the log line does not need to name anybody to say
    /// which authority was used.
    fn kind_label(&self) -> &'static str {
        match self {
            Self::Member(_) => "member",
            Self::Operator(_) => "operator",
        }
    }
}

/// The 403 for a client closing a room while claiming an agent's or the
/// daemon's identity. `enforce_client_artifact_author`'s rule and
/// `forged_attachment_author`'s shape, under this route's own code.
fn forged_closer_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "ok": false,
            "code": "forged_closer",
            "error": "an agent does not close a room; a client may not close one while claiming its identity",
        })),
    )
}

/// Map an operator-credential refusal onto the wire.
///
/// Deliberately its own function rather than `room_agent_authority`'s
/// `ApiError` conversion: that one maps `Missing` to 503 because on those routes
/// the operator header is the ONLY authority, and an absent one means the
/// feature is unavailable. Here an absent header is not an error at all — it
/// selects the member lane before this function is ever reached — so `Missing`
/// can only mean a header that was present and then vanished between two reads,
/// which is a 403 and not a service statement.
pub(super) fn operator_refusal_response(
    error: crate::room_operator::OperatorAuthError,
) -> (StatusCode, Json<serde_json::Value>) {
    use crate::room_operator::OperatorAuthError::*;
    let status = match error {
        Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Missing | Invalid | AmbientCredential | ForeignOrigin => StatusCode::FORBIDDEN,
    };
    (
        status,
        Json(json!({
            "ok": false,
            "code": error.code(),
            "error": error.to_string(),
        })),
    )
}

#[derive(Debug, serde::Deserialize, Default)]
pub(super) struct RoomsListQuery {
    /// Max rooms to return in this page. Omitted ⇒ the store's default cap
    /// (`DEFAULT_LIST_LIMIT`); any value is clamped to `MAX_LIST_LIMIT`.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Cursor: the room key of the last room from the previous page. Omitted ⇒
    /// the first page. Replay `next_cursor` here for the following page.
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
struct PersistentRoomReadState {
    room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_seq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    read_seq: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
struct PersistentRoomsListResponse {
    ok: bool,
    rooms: Vec<ocean_core::Room>,
    read_states: Vec<PersistentRoomReadState>,
    // Deliberately NOT `skip_serializing_if`: pre-existing pollers rely on the
    // key always being present (`"next_cursor": null` on the final page), so
    // omitting the key on a single-page response would be a silent wire
    // compatibility break.
    next_cursor: Option<String>,
    has_more: bool,
}

/// `GET /v1/rooms/persistent?limit=&cursor=` — list open persistent rooms, one
/// bounded page at a time (OCEAN-250). Rooms are ordered most-recently-updated
/// first; the `rooms` array shape is unchanged, with additive
/// `next_cursor`/`has_more` so a poller doesn't re-serialize every room each call.
pub(super) async fn rooms_list_persistent(
    State(state): State<AppState>,
    Query(q): Query<RoomsListQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    match with_rooms(&state, |reg| {
        let page = reg.list_page(q.cursor.as_deref(), q.limit)?;
        let read_states = page
            .rooms
            .iter()
            .map(|room| {
                let key = room.id.clone();
                let access = reg.room_access(&key)?;
                let principal: Option<String> = match access.state {
                    RoomAccessState::Local => Some(local_room_read_cursor_principal().to_string()),
                    RoomAccessState::Live
                    | RoomAccessState::Connecting
                    | RoomAccessState::Recovering
                    | RoomAccessState::Revoked => reg
                        .room_credential(&key)?
                        .map(|credential| credential.local_human_member_id),
                };
                let cursor = match principal.as_deref() {
                    Some(principal) => reg.room_read_cursor(&key, principal)?,
                    None => RoomReadCursorProjection {
                        read_seq: None,
                        mirrored_upstream_read_seq: None,
                    },
                };
                let latest_seq = match access.state {
                    RoomAccessState::Local => reg.room_latest_durable_seq(&key)?,
                    RoomAccessState::Live => access.last_confirmed_global_sequence,
                    RoomAccessState::Connecting
                    | RoomAccessState::Recovering
                    | RoomAccessState::Revoked => access.last_confirmed_global_sequence,
                };
                let read_seq = match access.state {
                    RoomAccessState::Local => cursor.read_seq,
                    RoomAccessState::Live
                    | RoomAccessState::Connecting
                    | RoomAccessState::Recovering
                    | RoomAccessState::Revoked => cursor.mirrored_upstream_read_seq,
                };
                Ok::<_, ocean_store::RoomStoreError>(PersistentRoomReadState {
                    room_id: room.id.to_string(),
                    latest_seq: latest_seq.map(|seq| seq.to_string()),
                    read_seq: read_seq.map(|seq| seq.to_string()),
                })
            })
            .collect::<Result<Vec<_>, ocean_store::RoomStoreError>>()?;
        Ok::<_, ocean_store::RoomStoreError>(PersistentRoomsListResponse {
            ok: true,
            rooms: page.rooms,
            read_states,
            next_cursor: page.next_cursor,
            has_more: page.has_more,
        })
    }) {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).unwrap()),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent/{key}` — one persistent room (with its transcript
/// and access projection). Open rooms only; soft-closed rooms return 404.
pub(super) async fn room_get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    match with_rooms(&state, |reg| {
        let Some(record) = reg.get(&key)? else {
            return Ok(None);
        };
        let access = reg.room_access(&key)?;
        // Which worker owns which agent in THIS room. Adjacent to the roster,
        // never a field on RoomParticipant (the federated design reserves
        // owner/sovereignty for Bedrock's authenticated principal mapping).
        // Absent key == no local ownership recorded, which is what every
        // pre-existing room reports.
        let owners = reg.agent_owners(&key)?;
        let aliases = room_retirement::aliases_projection(reg, &key)?;
        Ok(Some((record, access, owners, aliases)))
    }) {
        Ok(Some((rec, access, owners, aliases))) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "room": rec.room,
                "has_more": rec.transcript_has_more,
                "next_seq": if rec.transcript_has_more { rec.transcript.last().map(|row| row.seq) } else { None },
                "transcript": projected_transcript(rec.transcript),
                "access": access,
                "aliases": aliases.aliases,
                "aliases_truncated": aliases.truncated,
                "agent_owners": owners
                    .into_iter()
                    .map(|(agent, owner, owner_present)| json!({
                        "agent_id": agent,
                        "owner_id": owner,
                        // "owned" is not "owner still in the room". A worker can
                        // leave and the binding outlives them; the room says so
                        // rather than asserting a live claim it cannot prove.
                        "owner_present": owner_present,
                    }))
                    .collect::<Vec<_>>(),
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("no room with key '{key}'") })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent/{key}/inspect` — bounded read-only identity
/// inspection for operator migration and reconnect checks. It intentionally
/// returns only the room id/name, current local owner id, and the capped alias
/// projection; it does not expose transcript, workspace, credentials, or store
/// internals.
pub(super) async fn room_inspect(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    match with_rooms(&state, |store| {
        let Some(room) = store.inspect_room_identity(&key)? else {
            return Ok(None);
        };
        let owner = store.local_room_owner(&key)?.map(|owner| owner.member_id);
        let aliases = room_retirement::aliases_projection(store, &key)?;
        Ok(Some((room, owner, aliases)))
    }) {
        Ok(Some((room, owner, aliases))) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "closed": room.closed,
                "room": {
                    "id": room.room_id,
                    "name": room.name,
                },
                "owner": { "member_id": owner },
                "aliases": aliases.aliases,
                "aliases_truncated": aliases.truncated,
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("no room with key '{key}'") })),
        ),
        Err(error) => room_store_error_response(error),
    }
}

/// Display name the daemon owner is minted with on first use when no team
/// member id is configured: the operator's explicit `OCEAN_OWNER_NAME`, else
/// the login `USER`, else `Operator`. Only consulted while no owner row exists
/// (`SqliteRoomStore::owner_identity_as`).
pub(super) fn default_owner_display_name() -> String {
    ["OCEAN_OWNER_NAME", "USER"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "Operator".to_string())
}

/// The team member this daemon's human is, as `GET /v1/identity` (#41) and
/// `ocean-mcp` resolve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DaemonMember {
    pub(super) member_id: String,
    pub(super) display_name: Option<String>,
}

/// The same precedence and strict parsing as the daemon identity resolver:
/// `<config_dir>/member.toml` (`member_id`, optional `display_name`; unknown
/// fields, duplicate keys, nested tables or a malformed id are treated as
/// absent), then `OCEAN_MEMBER_ID`. `None` when neither names anyone; never
/// the process user.
pub(super) fn resolve_daemon_member(
    config_dir: &std::path::Path,
    env_member: Option<&str>,
) -> Option<DaemonMember> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct MemberToml {
        member_id: String,
        display_name: Option<String>,
    }
    fn valid_member_id(value: &str) -> bool {
        !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
    }
    let from_file = std::fs::read_to_string(config_dir.join("member.toml"))
        .ok()
        .and_then(|raw| toml::from_str::<MemberToml>(&raw).ok())
        .filter(|parsed| valid_member_id(&parsed.member_id))
        .map(|parsed| DaemonMember {
            member_id: parsed.member_id,
            display_name: parsed.display_name.filter(|name| {
                !name.is_empty()
                    && name.chars().count() <= 80
                    && !name.chars().any(char::is_control)
            }),
        });
    from_file.or_else(|| {
        env_member
            .map(str::trim)
            .filter(|member| valid_member_id(member))
            .map(|member_id| DaemonMember {
                member_id: member_id.to_string(),
                display_name: None,
            })
    })
}

/// Owner seed: the configured member id (if any) and the display name to mint
/// with. With a member id the default name comes from `member.toml`, then
/// `OCEAN_OWNER_NAME`, then the member id itself; the login name is used only
/// when no member id exists.
/// The config dir is the runtime's own (`OCEAN_CONFIG_DIR`,
/// `XDG_CONFIG_HOME/ocean-rs`, then `~/.config/ocean-rs` in production), the
/// directory `operator.key` and `rooms.db` live in.
fn owner_seed(state: &AppState) -> (Option<String>, String) {
    let env_member = std::env::var("OCEAN_MEMBER_ID").ok();
    match resolve_daemon_member(state.runtime.config_dir(), env_member.as_deref()) {
        Some(member) => {
            let name = member
                .display_name
                .or_else(|| {
                    std::env::var("OCEAN_OWNER_NAME")
                        .ok()
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .unwrap_or_else(|| member.member_id.clone());
            (Some(member.member_id), name)
        }
        None => (None, default_owner_display_name()),
    }
}

/// The one human this daemon belongs to (team-platform P2). Every local human
/// join and post is authored as this identity; client-claimed human ids are
/// never authority.
pub(super) fn daemon_owner(
    state: &AppState,
) -> Result<ocean_store::OwnerIdentity, ocean_store::RoomStoreError> {
    // Read member.toml before taking the store guard.
    let (member_id, default_name) = owner_seed(state);
    with_rooms(state, |reg| {
        reg.owner_identity_as(member_id.as_deref(), &default_name)
    })
}

fn owner_json(owner: &ocean_store::OwnerIdentity) -> serde_json::Value {
    json!({
        "participant_id": owner.participant_id,
        "display_name": owner.display_name,
    })
}

/// `GET /v1/me` — the daemon owner identity every surface renders as "me".
pub(super) async fn me_get(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    match daemon_owner(&state) {
        Ok(owner) => (StatusCode::OK, Json(owner_json(&owner))),
        Err(e) => room_store_error_response(e),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeUpdateRequest {
    display_name: String,
}

/// `PUT /v1/me` — rename the daemon owner. The participant id never changes;
/// the owner's local Human roster rows are renamed in the same transaction.
/// A mutating owner route: it requires the header-only Room operator before
/// the body is read, like the other owner mutations (daemon reachability,
/// including through the Surface proxy, never identifies the owner).
pub(super) async fn me_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<MeUpdateRequest>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = room_agent_authority::operator(&state, &headers) {
        return error.response();
    }
    let Ok(Json(req)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid_request" })),
        );
    };
    let (member_id, default_name) = owner_seed(&state);
    match with_rooms(&state, |reg| {
        reg.set_owner_display_name_as(member_id.as_deref(), &default_name, &req.display_name)
    }) {
        Ok(Some(owner)) => (StatusCode::OK, Json(owner_json(&owner))),
        Ok(None) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "invalid_display_name",
                "error": "display_name must be 1-64 characters",
            })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct RoomJoinRequest {
    /// Stable participant id, unique within the room. Ignored for `human`
    /// joins: a human joins as the daemon owner (team-platform P2).
    #[serde(default)]
    pub(super) id: String,
    /// Display name shown in the roster and transcript. Ignored for `human`
    /// joins, which take the owner's display name.
    #[serde(default)]
    pub(super) display_name: String,
    /// What kind of actor is joining. Defaults to `human`.
    #[serde(default = "default_participant_kind")]
    pub(super) kind: RoomParticipantKind,
    /// The WORKER who owns this agent. Only meaningful for `kind: Agent`, and
    /// the owner must already be a Human on this room's roster. This is the
    /// local half of "a worker persists alongside their agents": it makes
    /// "my agent" a real relationship in a room with no federation.
    #[serde(default)]
    pub(super) owner_id: Option<String>,
}

fn default_participant_kind() -> RoomParticipantKind {
    RoomParticipantKind::Human
}

/// `POST /v1/rooms/persistent/{key}/participants` — add a participant.
pub(super) async fn room_join(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(mut req): Json<RoomJoinRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    // Team-platform P2: a human joining through this daemon IS its owner.
    // Every surface talking to this daemon belongs to that one person, so the
    // body's id/display_name are ignored rather than trusted.
    if matches!(req.kind, RoomParticipantKind::Human) {
        match daemon_owner(&state) {
            Ok(owner) => {
                req.id = owner.participant_id;
                req.display_name = owner.display_name;
            }
            Err(e) => return room_store_error_response(e),
        }
    }
    // `System` is the DAEMON'S OWN author identity. Every audit row it writes is
    // authored `("system", System)` — the auto-convene notice, the "not bound"
    // note, the turn-failure line. If a client may join as System, the
    // `ParticipantJoined` marker it produces is a System-authored transcript row
    // that a reader cannot tell apart from a genuine daemon audit line. That is
    // transcript forgery, so System is refused at JOIN for the same reason
    // `classify_local_author` refuses it at POST (:743-748) — the two gates now
    // agree instead of only the second one holding.
    if matches!(req.kind, RoomParticipantKind::System) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "forged_participant_kind",
                "error": "'system' is the daemon's own author identity and cannot be joined",
            })),
        );
    }
    // Join and post must agree on what an id IS. `classify_local_author` treats
    // roster ids as canonical and refuses anything that is empty or not equal to
    // its own trim (:751-753). Without the same rule here, joining as `" john "`
    // succeeds and then that participant can NEVER post — a permanent, silent,
    // self-inflicted denial with no way to discover the cause. Refuse it at the
    // door instead.
    let id = req.id.trim();
    if id.is_empty() || id != req.id {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "invalid_participant_id",
                "error": "participant id must be non-empty and carry no leading or trailing whitespace",
            })),
        );
    }
    if req.display_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "invalid_display_name",
                "error": "display_name must not be empty",
            })),
        );
    }
    // Named-agent binding (TASK-9): an Agent participant MUST name a resolvable
    // folder-as-agent. Reject an unresolved name with a typed 4xx so a phantom
    // agent can never enter the roster as an Agent (it would later convene a
    // default-assistant turn it never authorized). Human/Bot/System participants
    // are unaffected — only `kind == Agent` is bound to a real AgentDef.
    if matches!(req.kind, RoomParticipantKind::Agent) {
        if let Err(_e) = resolve_named_agent(&req.id) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "code": "agent_unresolved",
                    "error": format!("agent '{}' does not resolve", req.id),
                })),
            );
        }
    }
    // An owner is only meaningful for an Agent. Refuse it elsewhere rather than
    // silently dropping it — a caller that believed it recorded ownership and
    // did not is the false-success class this work exists to remove.
    if req.owner_id.is_some() && !matches!(req.kind, RoomParticipantKind::Agent) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "owner_requires_agent",
                "error": "owner_id is only valid for a participant of kind 'agent'",
            })),
        );
    }
    let owner_id = req.owner_id.clone();
    let participant = RoomParticipant {
        id: req.id,
        kind: req.kind,
        display_name: req.display_name,
    };
    let result = with_rooms(&state, |reg| match owner_id.as_deref() {
        // The store validates the owner against the live roster INSIDE the same
        // transaction as the insert, so a concurrent leave cannot strand an
        // agent owned by someone who is gone.
        Some(owner) => reg.add_agent_participant_with_owner(&key, participant, owner, Utc::now()),
        None => reg.add_participant_with_message(&key, participant, Utc::now()),
    });
    match result {
        Ok((rec, message)) => {
            publish_room_wake(&state, &key, &message);
            (
                StatusCode::OK,
                Json(json!({ "ok": true, "room": rec.room })),
            )
        }
        Err(e) => room_store_error_response(e),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateArtifactRequest {
    pub(super) id: String,
    pub(super) kind: RoomArtifactKind,
    pub(super) title: String,
    #[serde(default)]
    pub(super) body: String,
    /// Client author id. Roster membership is validated inside the transaction;
    /// Agent and System authors are reserved for internal daemon writers.
    pub(super) author_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AmendArtifactRequest {
    /// The version the caller READ. Compare-and-swap: if the artifact has moved
    /// on, the write is refused with the actual version rather than merged.
    pub(super) expected_version: u64,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) state: Option<RoomArtifactState>,
    pub(super) author_id: String,
}

/// Classify and mutate under the same guard: a leave/rejoin cannot change the
/// author kind between the HTTP authorization decision and the committed write.
/// `None` rejects a daemon-owned author; unreadable roster state fails closed.
fn with_client_artifact_author<T>(
    rooms: &RoomStoreHandle,
    key: &RoomKey,
    author_id: &str,
    write: impl FnOnce(&mut ocean_store::SqliteRoomStore) -> Result<T, ocean_store::RoomStoreError>,
) -> Result<Option<T>, ocean_store::RoomStoreError> {
    with_rooms_handle(rooms, |store| {
        let daemon_owned = store.get(key)?.is_some_and(|record| {
            record.room.participants.iter().any(|participant| {
                participant.id == author_id
                    && matches!(
                        participant.kind,
                        RoomParticipantKind::Agent | RoomParticipantKind::System
                    )
            })
        });
        if daemon_owned {
            return Ok(None);
        }
        write(store).map(Some)
    })
}

fn forged_artifact_author_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "ok": false,
            "code": "forged_artifact_author",
            "error": "an agent's artifact is authored by the daemon, not by a client claiming its identity",
        })),
    )
}

/// `POST /v1/rooms/persistent/{key}/artifacts` — record something the room
/// produced: a task, a decision, or captured knowledge.
pub(super) async fn room_create_artifact(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<CreateArtifactRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    if req.id.trim().is_empty() || req.id != req.id.trim() {
        return invalid_request_response();
    }
    if req.title.trim().is_empty() {
        return invalid_request_response();
    }
    // Finding B: `author_id` is caller-supplied and only roster-checked, so a
    // hostile local caller could author an artifact AS somebody's agent. An
    // agent's artifact is produced by the daemon's own convene path, never by a
    // client claiming an agent's identity over the wire — the same rule
    // `classify_local_author` already applies to messages (Agent|System are
    // daemon-only author kinds). Enforce it here too instead of trusting the
    // client one layer down.
    let result = with_client_artifact_author(&state.rooms, &key, &req.author_id, |store| {
        store.create_artifact(
            &key,
            &req.id,
            req.kind,
            &req.title,
            &req.body,
            &req.author_id,
            Utc::now(),
        )
    });
    match result {
        Ok(Some((artifact, message))) => {
            // The transcript line is live on the room's SSE, so every client
            // learns the artifact exists without polling.
            publish_room_wake(&state, &key, &message);
            (
                StatusCode::CREATED,
                Json(json!({ "ok": true, "artifact": artifact })),
            )
        }
        Ok(None) => forged_artifact_author_response(),
        Err(e) => room_store_error_response(e),
    }
}

/// `POST /v1/rooms/persistent/{key}/artifacts/{artifact_id}/amend` — rewrite an
/// artifact in place under compare-and-swap.
pub(super) async fn room_amend_artifact(
    State(state): State<AppState>,
    Path((key, artifact_id)): Path<(String, String)>,
    Json(req): Json<AmendArtifactRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    if req
        .title
        .as_deref()
        .is_some_and(|title| title.trim().is_empty())
    {
        return invalid_request_response();
    }
    let result = with_client_artifact_author(&state.rooms, &key, &req.author_id, |store| {
        store.amend_artifact(
            &key,
            artifact_id.trim(),
            req.expected_version,
            req.title.as_deref(),
            req.body.as_deref(),
            req.state,
            &req.author_id,
            Utc::now(),
        )
    });
    match result {
        Ok(Some((artifact, message))) => {
            publish_room_wake(&state, &key, &message);
            (
                StatusCode::OK,
                Json(json!({ "ok": true, "artifact": artifact })),
            )
        }
        Ok(None) => forged_artifact_author_response(),
        // A stale write must hand back where to re-read from, not just "409".
        Err(ocean_store::RoomStoreError::ArtifactVersionConflict {
            expected, actual, ..
        }) => (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "code": "artifact_version_conflict",
                "expected_version": expected,
                "actual_version": actual,
                "error": format!("artifact is at version {actual}, not {expected}; re-read and retry"),
            })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent/{key}/artifacts/{artifact_id}` — one artifact.
///
/// This is the other half of the compare-and-swap contract. A 409 tells a caller
/// their version is stale and hands back the actual one, but without a
/// single-artifact read the only recovery is to re-list the whole room: fine at
/// five artifacts, absurd at two hundred. With this the conflict->re-read->retry
/// loop is one round trip.
pub(super) async fn room_get_artifact(
    State(state): State<AppState>,
    Path((key, artifact_id)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    match with_rooms(&state, |store| store.artifact(&key, artifact_id.trim())) {
        Ok(Some(artifact)) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "artifact": artifact })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "code": "unknown_artifact",
                "error": format!("room '{key}' has no artifact '{}'", artifact_id.trim()),
            })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent/{key}/artifacts` — everything this room produced.
pub(super) async fn room_list_artifacts(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    match with_rooms(&state, |store| store.artifacts(&key)) {
        Ok(artifacts) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "artifacts": artifacts })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `DELETE /v1/rooms/persistent/{key}/participants/{participant_id}` — remove a
/// participant from the roster.
pub(super) async fn room_leave(
    State(state): State<AppState>,
    Path((key, participant_id)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    let mut requests = state.requests.write().await;
    let (result, cancelled) = with_rooms(&state, |store| {
        let result = store.remove_participant_with_message(&key, participant_id.trim(), Utc::now());
        let cancelled = if result.is_ok() {
            room_agent_authority::cancel_ineligible_room_requests_locked(&mut requests, store, &key)
        } else {
            Vec::new()
        };
        (result, cancelled)
    });
    drop(requests);
    room_agent_authority::cleanup_cancelled(&state, cancelled).await;
    match result {
        Ok((rec, message)) => {
            publish_room_wake(&state, &key, &message);
            (
                StatusCode::OK,
                Json(json!({ "ok": true, "room": rec.room })),
            )
        }
        Err(e) => room_store_error_response(e),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RoomMessageRequest {
    /// Author participant id. Ignored for `human` posts, which are always
    /// authored as the daemon owner (team-platform P2); still roster-gated for
    /// `bot` posts.
    #[serde(default)]
    pub(super) author_id: String,
    /// Author kind for attribution. Defaults to `human`.
    #[serde(default = "default_participant_kind")]
    pub(super) author_kind: RoomParticipantKind,
    /// Message body. `@id` mentions in the body drive trigger evaluation.
    pub(super) body: String,
    /// When this is a reply, the `seq` of the parent message (G1-B real
    /// threads). `None` for top-level messages.
    #[serde(default)]
    pub(super) thread_parent_seq: Option<u64>,
    // NOTE (G3): there is deliberately NO `session_id` field. Session
    // attribution is derived by the daemon from the path that produced the row
    // — `room_agent_session_id` for a convened agent reply (see
    // [`append_room_agent_reply`]) — so a client can never attribute its post
    // to a session it does not own. A locally posted HTTP message has no
    // owning daemon session and is stored with `session_id = NULL`.
}

/// Why the daemon refused a locally authored post *before* it could reach the
/// transcript (G3 author authority + thread integrity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostRejection {
    /// The caller claimed an `agent`/`system` author kind. Those rows are
    /// daemon-authored only (convened replies and audit lines); accepting a
    /// client-supplied one would let a browser forge an agent utterance *and*
    /// bypass the anti-loop guard, which skips trigger evaluation for
    /// agent-authored messages.
    ForgedAuthorKind,
    /// The `(author_id, author_kind)` pair is not on the room's roster, so the
    /// caller is claiming an identity this room never admitted.
    AuthorNotInRoster,
    /// `thread_parent_seq` violated the store's one-level thread policy. The
    /// store rejected it inside the append transaction; nothing was written.
    InvalidThreadParent,
}

/// The local post path's error: either the durable store failed, or the daemon
/// itself refused the post. Keeping them apart is what lets a refusal answer
/// with a fixed 4xx while a store fault keeps its existing mapping.
#[derive(Debug)]
pub(super) enum LocalPostError {
    /// An underlying store error (unknown room, federation corruption, SQLite).
    Store(ocean_store::RoomStoreError),
    /// A daemon-side refusal with a fixed, body-free reason.
    Rejected(PostRejection),
}

impl From<ocean_store::RoomStoreError> for LocalPostError {
    fn from(e: ocean_store::RoomStoreError) -> Self {
        Self::Store(e)
    }
}

/// Map a [`PostRejection`] onto its frozen `(status, body)` pair. The body
/// carries a stable machine code and never echoes the rejected author id,
/// claimed kind, or message body.
pub(super) fn post_rejection_response(
    rejection: PostRejection,
) -> (StatusCode, Json<serde_json::Value>) {
    let (status, code) = match rejection {
        PostRejection::ForgedAuthorKind => (StatusCode::FORBIDDEN, "forged_author_kind"),
        PostRejection::AuthorNotInRoster => (StatusCode::FORBIDDEN, "author_not_in_roster"),
        PostRejection::InvalidThreadParent => (StatusCode::BAD_REQUEST, "invalid_thread_parent"),
    };
    (status, Json(json!({ "ok": false, "error": code })))
}

/// Decide whether a client may author this local post (G3).
///
/// Two rules, both fail-closed:
///
/// 1. `agent` and `system` are daemon-only author kinds. The daemon writes
///    those rows itself ([`append_room_agent_reply`] and the audit appends);
///    a request that claims one is a forgery regardless of its id.
/// 2. The `(id, kind)` pair must already be on the roster. Membership — not
///    the request body — is the authority on who may speak in a room, so an
///    unknown id, or a known id claiming the wrong kind, is refused.
pub(super) fn classify_local_author<'a>(
    roster: &'a [RoomParticipant],
    author_id: &str,
    author_kind: RoomParticipantKind,
) -> Result<&'a str, PostRejection> {
    if matches!(
        author_kind,
        RoomParticipantKind::Agent | RoomParticipantKind::System
    ) {
        return Err(PostRejection::ForgedAuthorKind);
    }
    // Roster ids are canonical authority. Do not admit a trimmed spelling and
    // then persist the caller's non-canonical bytes (for example `" john "`).
    if author_id.is_empty() || author_id != author_id.trim() {
        return Err(PostRejection::AuthorNotInRoster);
    }
    roster
        .iter()
        .find(|participant| participant.id == author_id && participant.kind == author_kind)
        .map(|participant| participant.id.as_str())
        .ok_or(PostRejection::AuthorNotInRoster)
}

/// Read just the author of one thread root, as a bounded single-row query.
///
/// Uses the `LIMIT`ed [`RoomStore::transcript_page`] with `after_seq =
/// root_seq - 1` and `limit = 1`, so this never loads a transcript to answer a
/// one-row question. Returns `None` when no row with exactly `root_seq` exists
/// in this room; a caller treats that as "no thread-reply trigger", never as an
/// error.
fn thread_root_author(
    reg: &ocean_store::SqliteRoomStore,
    key: &RoomKey,
    root_seq: u64,
) -> Result<Option<String>, ocean_store::RoomStoreError> {
    // `seq` is 0-based, and `transcript_page` is exclusive on `after_seq`, so
    // seq 0 must page from the start rather than from `-1`.
    let after_seq = root_seq.checked_sub(1);
    let page = reg.transcript_page(key, after_seq, Some(1))?;
    Ok(page
        .messages
        .into_iter()
        .find(|m| m.seq == root_seq)
        .map(|m| m.author_id))
}

/// `POST /v1/rooms/persistent/{key}/messages` — append a chat message to the
/// transcript, then evaluate the room's trigger policy against any @-mentions in
/// the body. On a positive decision that resolves to an agent participant, emit a
/// `room_trigger` notice onto the agent event bus AND queue a real agent turn for
/// that agent (it reads the room context and posts its reply back into the
/// transcript). See `spawn_room_agent_turn` for the turn path (OCEAN-111/225).
pub(super) async fn room_post_message(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<RoomMessageRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    let (owner_member_id, default_owner_name) = owner_seed(&state);
    // Classification and the Local append share one store guard. Credential
    // installation can therefore linearize only before or after this commit,
    // never between a Local check and a later append.
    let append = with_rooms(&state, |reg| {
        if reg.get(&key)?.is_none() {
            return Err(ocean_store::RoomStoreError::UnknownRoom(key.clone()).into());
        }
        if reg.room_credential(&key)?.is_some() {
            return Ok(None);
        }
        if reg.room_access(&key)?.state != RoomAccessState::Local {
            return Err(ocean_store::RoomStoreError::FederationCorruption(
                "missing credential for non-local room".into(),
            )
            .into());
        }
        let roster = reg
            .get(&key)?
            .map(|rec| rec.room.participants)
            .unwrap_or_default();
        // G3 author authority: decide who may speak BEFORE anything is
        // written, under the same guard as the append, so a roster change
        // cannot land between the decision and the row. The returned id is the
        // exact roster-owned canonical spelling used for persistence.
        //
        // Team-platform P2: a human post is authored by the daemon owner, never
        // by a client-claimed id. The owner must still be on the roster.
        let owner_id;
        let claimed_author_id = if matches!(req.author_kind, RoomParticipantKind::Human) {
            owner_id = reg
                .owner_identity_as(owner_member_id.as_deref(), &default_owner_name)?
                .participant_id;
            owner_id.as_str()
        } else {
            req.author_id.as_str()
        };
        let canonical_author_id =
            classify_local_author(&roster, claimed_author_id, req.author_kind)
                .map_err(LocalPostError::Rejected)?;
        // Read the thread root's author before appending: the reply itself is
        // not a valid trigger source, and after the append the root is one row
        // further back. `None` for a top-level post or a vanished root.
        let root_author = match req.thread_parent_seq {
            Some(parent_seq) => thread_root_author(reg, &key, parent_seq)?,
            None => None,
        };
        let msg = match reg.append_message_threaded(
            &key,
            canonical_author_id,
            req.author_kind,
            RoomMessageKind::Message,
            &req.body,
            Utc::now(),
            req.thread_parent_seq,
            // G3: session attribution is daemon-derived, never client-supplied.
            // An HTTP post has no owning daemon session.
            None,
        ) {
            Ok(msg) => msg,
            // A bad parent is a client mistake, not a server fault: keep it a
            // typed 400 instead of collapsing onto `RoomStoreError::Encode`.
            Err(ThreadAppendError::InvalidThreadParent { .. }) => {
                return Err(LocalPostError::Rejected(PostRejection::InvalidThreadParent))
            }
            Err(ThreadAppendError::Store(e)) => return Err(LocalPostError::Store(e)),
        };
        let policy = reg.trigger_policy(&key)?;
        Ok::<_, LocalPostError>(Some((msg, policy, roster, root_author)))
    });

    let (msg, policy, roster, root_author) = match append {
        Ok(Some(local)) => local,
        Ok(None) => {
            return match state
                .room_federation
                .enqueue_federated_message(&key, None, &req.body)
                .await
            {
                Ok(access) => (
                    StatusCode::ACCEPTED,
                    Json(json!({ "ok": true, "access": access })),
                ),
                Err(error) => intent_error_response(error),
            };
        }
        Err(LocalPostError::Rejected(rejection)) => return post_rejection_response(rejection),
        Err(LocalPostError::Store(ocean_store::RoomStoreError::UnknownRoom(_))) => {
            return intent_error_response(IntentError::NotFound)
        }
        Err(LocalPostError::Store(ocean_store::RoomStoreError::FederationCorruption(_))) => {
            return intent_error_response(IntentError::Store)
        }
        Err(LocalPostError::Store(e)) => return room_store_error_response(e),
    };
    publish_room_wake(&state, &key, &msg);

    // ---- Auto-convene wiring point (OCEAN-65 / OCEAN-111) -------------------
    //
    // Parse @-mentions from the message body, evaluate each against the room's
    // trigger policy, and for every positive decision that resolves to an AGENT
    // participant in the roster: (a) emit the `room_trigger` notice + an audit
    // line (the observable contract, unchanged), and (b) ACTUALLY queue an
    // agent turn that wakes the agent, gives it the room context, and posts its
    // reply back into the transcript.
    //
    // Anti-loop guardrail #1 (the cheap, total one): an agent's OWN posted
    // reply is authored as `RoomParticipantKind::Agent`, and we never evaluate
    // triggers on agent-authored messages. So an agent that @-mentions another
    // agent (or itself) in its reply can never ping-pong the room. Only
    // human/bot/system-authored lines can convene an agent.
    let mut fired = Vec::new();
    let mut convened = std::collections::HashSet::new();
    // Agents whose turn this row actually admitted (a subset of `convened`).
    let mut admitted = std::collections::HashSet::new();
    if !matches!(req.author_kind, RoomParticipantKind::Agent) {
        // Every trigger source for THIS row, in a fixed order: each @-mention in
        // body order, then (G3) the thread-root author when this post is a reply.
        // A single evaluation loop keeps one convene footprint per agent.
        let events = parse_mentions(&req.body)
            .into_iter()
            .map(|participant_id| RoomTriggerEvent::Mention { participant_id })
            .chain(
                root_author
                    .into_iter()
                    .map(|participant_id| RoomTriggerEvent::ThreadReply { participant_id }),
            );
        for event in events {
            let decision = evaluate_trigger_policy(policy.as_ref(), &event);
            if !decision.should_convene {
                continue;
            }

            // Resolve the target participant id → an AGENT participant in the
            // roster BEFORE writing any convene footprint. Only genuine `Agent`
            // participants are runnable; a mention of a human/bot/tool id (or an
            // unknown id) resolves to `None`. The policy may say "convene", but
            // if there's no agent to wake then no convene actually happens — so
            // neither the `room_trigger` event nor the `auto-convene:` transcript
            // line may fire (OCEAN-128: writing the audit line for a non-agent
            // mention claimed a convene that never occurred).
            let resolved_agent = decision
                .target_participant
                .as_deref()
                .and_then(|id| resolve_agent_participant(&roster, id));

            // `triggers_fired` reflects raw policy evaluation; record it even
            // when the mention is a non-agent so the response is honest about
            // what the policy matched. The convene FOOTPRINT (event + audit line
            // + queued turn) below is gated on an actually-resolved agent.
            fired.push(decision.clone());

            let Some(agent) = resolved_agent else {
                continue;
            };

            // One convene footprint per agent per posted row. Mentioning an
            // agent twice — or mentioning the same agent that owns the thread
            // root — must not queue two turns for one message.
            if !convened.insert(agent.id.clone()) {
                continue;
            }

            // Phase 1 authority is the first observable convene boundary. A
            // refusal writes only its content-minimal admission decision; it
            // never emits the legacy room_trigger/auto-convene footprint and
            // never reads transcript or attachment context.
            let (admission, turn_permit) = match room_agent_authority::admit_room_agent(
                &state,
                &key,
                &agent.id,
                &agent.id,
                AdmissionTrigger::from_room_event(&event),
            )
            .await
            {
                Ok(admission) => admission,
                Err(error) => {
                    tracing::info!(room = %key, agent = %agent.id, reason = error.code(),
                        "room-agent admission refused before convene footprint");
                    continue;
                }
            };

            let target = decision.target_participant.clone().unwrap_or_default();
            let reason = decision.reason.clone();
            let agent_id = agent.id.clone();
            match spawn_room_agent_turn(
                state.clone(),
                admission,
                turn_permit,
                agent,
                msg.seq,
                None,
                Uuid::new_v4(),
                None,
                Some(RoomTurnFootprint {
                    payload: json!({
                        "room": key.as_str(),
                        "target": target,
                        "reason": reason,
                        "triggered_by_seq": msg.seq,
                    }),
                    audit_line: Some(format!("auto-convene: {} ({})", target, reason)),
                }),
            )
            .await
            {
                Ok(_) => {
                    admitted.insert(agent_id);
                }
                Err(error) => {
                    tracing::info!(room = %key, reason = error.code(),
                        "room-agent convene refused before runtime dispatch");
                }
            }
        }

        // Team-platform P4: a human reply in a thread answers every run parked
        // there by `room_ask`. Each parked run is first claimed by this answer
        // with a durable compare-and-swap (a concurrent reply that loses the
        // claim leaves it alone), then the same (room, agent) session is
        // convened with the reply, and only an admitted successor closes the
        // run `Done`. A refused admission releases the claim: the run stays
        // parked and the next reply can resume it. One successor per agent per
        // row: an agent this row already admitted above closes on that turn.
        if let Some(root) = req.thread_parent_seq {
            let parked = with_rooms(&state, |store| store.parked_room_agent_runs(&key, root))
                .unwrap_or_default();
            for run in parked {
                // Held across the admission awaits below: if this request is
                // dropped mid-way the claim settles on drop, never leaks.
                let Some(claim) = crate::room_agent_runs::AnswerClaim::take(&state, &run, msg.seq)
                else {
                    continue;
                };
                let agent_id = run.agent_id.clone();
                let resumed = if admitted.contains(&agent_id) {
                    true
                } else if !convened.insert(agent_id.clone()) {
                    false
                } else {
                    let started =
                        resume_parked_room_agent(&state, &key, &roster, &agent_id, msg.seq).await;
                    if started {
                        admitted.insert(agent_id);
                    }
                    started
                };
                claim.settle(resumed);
            }
        }
    }

    (
        StatusCode::CREATED,
        Json(json!({ "ok": true, "message": msg, "triggers_fired": fired })),
    )
}

/// Convene `agent_id` on a thread answer at `answer_seq`, resuming its parked
/// `room_ask` session. `true` only once the successor turn is admitted.
async fn resume_parked_room_agent(
    state: &AppState,
    key: &RoomKey,
    roster: &[RoomParticipant],
    agent_id: &str,
    answer_seq: u64,
) -> bool {
    let Some(agent) = resolve_agent_participant(roster, agent_id) else {
        return false;
    };
    if resolve_named_agent(&agent.id).is_err() {
        return false;
    }
    let Ok((admission, turn_permit)) = room_agent_authority::admit_room_agent(
        state,
        key,
        &agent.id,
        &agent.id,
        AdmissionTrigger::ThreadReply,
    )
    .await
    else {
        return false;
    };
    spawn_room_agent_turn(
        state.clone(),
        admission,
        turn_permit,
        agent,
        answer_seq,
        None,
        Uuid::new_v4(),
        None,
        None,
    )
    .await
    .is_ok()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateInviteBody {
    #[serde(default)]
    recipient_name: Option<String>,
    #[serde(default)]
    ttl_minutes: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RedeemInviteBody {
    code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RegisterAgentsBody {
    agent_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InvokeRoomAgentBody {
    invoked_by: String,
    message_seq: u64,
    #[serde(default)]
    decision_token: Option<String>,
}

/// Explicit same-room invocation seam. The caller can point only at an already
/// durable message it authored as a non-agent/non-system roster member; labels
/// and client event ids never create authority.
pub(super) async fn room_agent_invoke(
    State(state): State<AppState>,
    Path((key, agent_member_id)): Path<(String, String)>,
    body: Result<Json<InvokeRoomAgentBody>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => return invalid_request_response(),
    };
    let room = RoomKey::new(key.trim());
    let invoked_by = body.invoked_by.trim();
    let agent_member_id = agent_member_id.trim();
    if room.as_str().is_empty()
        || invoked_by.is_empty()
        || agent_member_id.is_empty()
        || body
            .decision_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty())
    {
        return invalid_request_response();
    }
    let resolved = with_rooms(&state, |store| {
        let record = store
            .get(&room)?
            .ok_or_else(|| ocean_store::RoomStoreError::UnknownRoom(room.clone()))?;
        let binding = store
            .room_agent_binding(&room, agent_member_id)?
            .ok_or_else(|| ocean_store::RoomStoreError::UnknownAgentBinding {
                room: room.clone(),
                agent: agent_member_id.to_string(),
            })?;
        let access = store.room_access(&room)?;
        let local = access.state == RoomAccessState::Local;
        let invoker_is_member = if local {
            record.room.participants.iter().any(|participant| {
                participant.id == invoked_by
                    && !matches!(
                        participant.kind,
                        RoomParticipantKind::Agent | RoomParticipantKind::System
                    )
            })
        } else {
            access.members.iter().any(|member| {
                member.member_id == invoked_by
                    && member.actor_type == ocean_core::FederatedActorType::User
            })
        };
        let target = if local {
            record
                .room
                .participants
                .iter()
                .find(|participant| {
                    participant.id == agent_member_id
                        && participant.kind == RoomParticipantKind::Agent
                })
                .cloned()
        } else {
            access
                .members
                .iter()
                .any(|member| {
                    member.member_id == agent_member_id
                        && member.actor_type == ocean_core::FederatedActorType::Agent
                })
                .then(|| RoomParticipant {
                    id: binding.agent_package_id.clone(),
                    kind: RoomParticipantKind::Agent,
                    display_name: binding.display_name.clone(),
                })
        };
        if !invoker_is_member || target.is_none() {
            return Err(ocean_store::RoomStoreError::Encode(
                "invoke_membership_required".into(),
            ));
        }
        Ok((
            binding.agent_package_id,
            target.expect("checked above"),
            !local,
        ))
    });
    let (package_id, agent, federated) = match resolved {
        Ok(value) => value,
        Err(ocean_store::RoomStoreError::UnknownRoom(_)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"ok": false, "error": "room_not_found"})),
            );
        }
        Err(ocean_store::RoomStoreError::UnknownAgentBinding { .. }) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"ok": false, "error": "agent_binding_required"})),
            );
        }
        Err(error) => {
            let code = match error.to_string().as_str() {
                value if value.contains("invoke_message_not_found") => "invoke_message_not_found",
                value if value.contains("invoke_author_mismatch") => "invoke_author_mismatch",
                _ => "invoke_membership_required",
            };
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"ok": false, "error": code})),
            );
        }
    };
    let (admission, turn_permit) = match room_agent_authority::admit_room_agent(
        &state,
        &room,
        agent_member_id,
        &package_id,
        AdmissionTrigger::Explicit,
    )
    .await
    {
        Ok(admission) => admission,
        Err(error) => return error.response(),
    };
    // The binding gate precedes the one authoritative transcript lookup. A
    // client cannot use invoke as a transcript oracle for a room agent that is
    // not currently admissible, and the later checked registration still
    // revalidates the exact authority generation before any runtime context.
    let invocation_message = with_rooms(&state, |store| {
        room_message_at_seq(store, &room, body.message_seq)
            .ok_or_else(|| ocean_store::RoomStoreError::Encode("invoke_message_not_found".into()))
    });
    let invocation_message = match invocation_message {
        Ok(message)
            if message.author_id == invoked_by
                && !matches!(
                    message.author_kind,
                    RoomParticipantKind::Agent | RoomParticipantKind::System
                ) =>
        {
            message
        }
        Ok(_) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"ok": false, "error": "invoke_author_mismatch"})),
            );
        }
        Err(_) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"ok": false, "error": "invoke_message_not_found"})),
            );
        }
    };
    let request_id = Uuid::new_v4();
    let queued = spawn_room_agent_turn(
        state,
        admission,
        turn_permit,
        agent,
        invocation_message.seq,
        federated.then(|| agent_member_id.to_string()),
        request_id,
        body.decision_token,
        None,
    )
    .await;
    match queued {
        Ok(queued) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "ok": true,
                "status": "queued",
                "admission_id": queued.admission_id,
                "request_id": queued.request_id,
                "generation": queued.generation.to_string(),
                "session_id": queued.session_id,
            })),
        ),
        Err(error) => error.response(),
    }
}

fn invalid_request_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok": false, "error": "invalid_request"})),
    )
}

pub(super) fn intent_error_response(error: IntentError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, code) = match error {
        IntentError::Invalid => (StatusCode::BAD_REQUEST, "invalid_request"),
        IntentError::NotFound => (StatusCode::NOT_FOUND, "room_not_found"),
        IntentError::Conflict => (StatusCode::CONFLICT, "federation_conflict"),
        IntentError::Forbidden => (StatusCode::FORBIDDEN, "federation_forbidden"),
        IntentError::InviteForbidden => (StatusCode::FORBIDDEN, "invite_forbidden"),
        IntentError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "federation_unavailable"),
        IntentError::Protocol => (StatusCode::BAD_GATEWAY, "federation_protocol"),
        IntentError::Store => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    (status, Json(json!({"ok": false, "error": code})))
}

pub(super) async fn room_create_invite(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: Result<Json<CreateInviteBody>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(Json(body)) = body else {
        return invalid_request_response();
    };
    let ttl = body.ttl_minutes.unwrap_or(1440);
    if !(1..=10080).contains(&ttl) {
        return invalid_request_response();
    }
    let key = RoomKey::new(key.trim());
    match state
        .room_federation
        .create_invite(&key, body.recipient_name, ttl)
        .await
    {
        Ok(invite) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(invite).expect("InviteResponse serializes")),
        ),
        Err(error) => intent_error_response(error),
    }
}

pub(super) async fn room_redeem_invite(
    State(state): State<AppState>,
    body: Result<Json<RedeemInviteBody>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(Json(body)) = body else {
        return invalid_request_response();
    };
    if body.code.trim().is_empty() {
        return invalid_request_response();
    }
    match state.room_federation.redeem_invite(&body.code).await {
        Ok(redeemed) => {
            let mut value =
                serde_json::to_value(redeemed.access).expect("RoomAccessProjection serializes");
            value
                .as_object_mut()
                .expect("RoomAccessProjection is an object")
                .insert("room_key".into(), json!(redeemed.room_key.as_str()));
            (StatusCode::OK, Json(value))
        }
        Err(error) => intent_error_response(error),
    }
}

pub(super) async fn room_register_agents(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: Result<Json<RegisterAgentsBody>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(Json(body)) = body else {
        return invalid_request_response();
    };
    if body.agent_names.is_empty() || body.agent_names.len() > 32 {
        return invalid_request_response();
    }
    let key = RoomKey::new(key.trim());
    let preflight = with_rooms(&state, |store| {
        if store.get(&key)?.is_none() {
            return Err(ocean_store::RoomStoreError::UnknownRoom(key.clone()));
        }
        let credential = store.room_credential(&key)?;
        let access = store.room_access(&key)?;
        Ok::<_, ocean_store::RoomStoreError>((credential.is_some(), access.state))
    });
    match preflight {
        Err(ocean_store::RoomStoreError::UnknownRoom(_)) => {
            return intent_error_response(IntentError::NotFound)
        }
        Err(_) => return intent_error_response(IntentError::Store),
        Ok((false, _)) => return intent_error_response(IntentError::Conflict),
        Ok((true, RoomAccessState::Revoked)) => {
            return intent_error_response(IntentError::Forbidden)
        }
        Ok((true, _)) => {}
    }
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(body.agent_names.len());
    for requested in body.agent_names {
        if requested.trim().is_empty() {
            return invalid_request_response();
        }
        let Some((agent_name, descriptor)) = resolve_agent_registration(&requested) else {
            return invalid_request_response();
        };
        if !seen.insert(agent_name.clone()) {
            return invalid_request_response();
        }
        resolved.push((agent_name, descriptor));
    }
    let instance_id = match with_rooms(&state, |store| store.federation_instance_id()) {
        Ok(id) => id,
        Err(_) => return intent_error_response(IntentError::Store),
    };
    let inputs = resolved
        .into_iter()
        .map(|(agent_name, descriptor)| AgentRegistrationInput {
            registration_key: registration_key(&instance_id, &key, &agent_name),
            agent_name,
            descriptor,
        })
        .collect();
    match state.room_federation.register_agents(&key, inputs).await {
        Ok(access) => (
            StatusCode::OK,
            Json(serde_json::to_value(access).expect("RoomAccessProjection serializes")),
        ),
        Err(error) => intent_error_response(error),
    }
}

pub(super) async fn run_federated_trigger_dispatcher(
    state: AppState,
    mut receiver: mpsc::UnboundedReceiver<FederatedTriggerDispatch>,
    cancel: CancellationToken,
) {
    loop {
        let dispatch = tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            dispatch = receiver.recv() => match dispatch {
                Some(dispatch) => dispatch,
                None => break,
            },
        };
        let agent_name = match with_rooms(&state, |store| {
            if store.get(&dispatch.room)?.is_none() {
                return Ok(None);
            }
            let Some(credential) = store.room_credential(&dispatch.room)? else {
                return Ok(None);
            };
            let access = store.room_access(&dispatch.room)?;
            if access.state == RoomAccessState::Revoked
                || !access.members.iter().any(|member| {
                    member.member_id == dispatch.target_member_id
                        && member.actor_type == ocean_core::FederatedActorType::Agent
                        && member.owner_member_id.as_deref()
                            == Some(credential.local_human_member_id.as_str())
                        && member.local_binding_available == Some(true)
                })
            {
                return Ok(None);
            }
            store.resolve_room_agent(&dispatch.room, &dispatch.target_member_id)
        }) {
            Ok(Some(name)) => name,
            _ => continue,
        };
        let agent = RoomParticipant {
            id: agent_name.clone(),
            kind: RoomParticipantKind::Agent,
            display_name: agent_name.clone(),
        };
        let (admission, turn_permit) = match room_agent_authority::admit_room_agent(
            &state,
            &dispatch.room,
            &dispatch.target_member_id,
            &agent_name,
            match dispatch.trigger_kind {
                FederatedTriggerKind::Mention => AdmissionTrigger::Mention,
                FederatedTriggerKind::ThreadReply => AdmissionTrigger::ThreadReply,
                FederatedTriggerKind::Unknown => AdmissionTrigger::Unknown,
            },
        )
        .await
        {
            Ok(admission) => admission,
            Err(_) => continue,
        };
        if let Err(error) = spawn_room_agent_turn(
            state.clone(),
            admission,
            turn_permit,
            agent,
            dispatch.local_seq,
            Some(dispatch.target_member_id.clone()),
            Uuid::new_v4(),
            None,
            Some(RoomTurnFootprint {
                payload: json!({
                    "room": dispatch.room.as_str(),
                    "target": dispatch.target_member_id,
                    "agent_name": agent_name,
                    "reason": dispatch.reason,
                    "triggered_by_seq": dispatch.local_seq,
                    "ledger_event_id": dispatch.ledger_event_id,
                }),
                audit_line: None,
            }),
        )
        .await
        {
            tracing::info!(room = %dispatch.room, agent = %dispatch.target_member_id,
                reason = error.code(),
                "federated room-agent turn refused before runtime dispatch");
        }
    }
}

/// Extract `@id` mentions from a message body. A mention is `@` followed by a
/// run of id-safe characters (alphanumerics, `-`, `_`, `.`). Returns ids without the
/// leading `@`, de-duplicated in first-seen order.
pub(super) fn parse_mentions(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.') {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > start {
                let id = body[start..j].to_string();
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

// ---- Auto-convene: participant→session resolution + turn queueing (OCEAN-111)

/// Fixed namespace for deriving a stable per-(room, agent) session id with UUID
/// v5. Same room + same agent participant ⇒ same session every time, so the
/// agent RESUMES its room transcript across mentions instead of forking a fresh
/// session on every wake. The constant itself is arbitrary but must never
/// change, or existing room-agent sessions would orphan.
const AUTHORIZED_ROOM_AGENT_SESSION_NS: Uuid =
    Uuid::from_u128(0x0ce1_a112_0000_4780_8000_526f_6f6d_4131);

#[cfg(test)]
const ROOM_AGENT_SESSION_NS: Uuid = Uuid::from_u128(0x0ce1_a111_0000_4780_8000_526f_6f6d_4147);

/// How many recent transcript lines to feed the woken agent as context. Enough
/// to ground the reply in the conversation without bloating the prompt.
const ROOM_CONTEXT_TAIL: usize = 20;

/// Resolve a mentioned participant id to a runnable AGENT participant. Returns
/// the participant only when it exists in the roster AND is of kind `Agent` —
/// a mention of a human/bot/tool/system id (or an unknown id) resolves to
/// `None`, so the notice still fires but no turn is queued.
pub(super) fn resolve_agent_participant(
    roster: &[RoomParticipant],
    participant_id: &str,
) -> Option<RoomParticipant> {
    roster
        .iter()
        .find(|p| p.id == participant_id && matches!(p.kind, RoomParticipantKind::Agent))
        .cloned()
}

/// Deterministic session id for a (room, agent-participant) pair. Stable across
/// daemon restarts and repeated mentions so the agent keeps one durable
/// transcript per room.
#[cfg(test)]
pub(super) fn room_agent_session_id(room: &RoomKey, participant_id: &str) -> AgentSessionId {
    let seed = format!("{}:{}", room.as_str(), participant_id);
    sdk_sid(Uuid::new_v5(&ROOM_AGENT_SESSION_NS, seed.as_bytes()))
}

/// Collision-safe durable session identity for one admitted Room/agent/generation.
pub(super) fn authorized_room_agent_session_id(
    room: &RoomKey,
    agent_member_id: &str,
    generation: u64,
) -> AgentSessionId {
    let mut seed = Vec::with_capacity(room.as_str().len() + agent_member_id.len() + 24);
    for value in [room.as_str().as_bytes(), agent_member_id.as_bytes()] {
        seed.extend_from_slice(&(value.len() as u64).to_be_bytes());
        seed.extend_from_slice(value);
    }
    seed.extend_from_slice(&generation.to_be_bytes());
    sdk_sid(Uuid::new_v5(&AUTHORIZED_ROOM_AGENT_SESSION_NS, &seed))
}

fn room_message_at_seq(
    store: &ocean_store::SqliteRoomStore,
    room: &RoomKey,
    seq: u64,
) -> Option<RoomMessage> {
    store
        .transcript_page(room, seq.checked_sub(1), Some(1))
        .ok()?
        .messages
        .into_iter()
        .find(|message| message.seq == seq)
}

/// Read exactly the transcript set authorized by the binding. Admission has
/// already succeeded before this function is reachable.
fn authorized_room_transcript_context(
    state: &AppState,
    admission: &RoomAgentAdmission,
    triggered_by_seq: u64,
    cancel: &tokio_util::sync::CancellationToken,
) -> Vec<RoomMessage> {
    with_rooms(state, |store| {
        if cancel.is_cancelled()
            || !room_agent_authority::current_binding_on(
                store,
                &admission.room,
                &admission.agent_member_id,
                admission.generation,
            )
            .ok()
            .flatten()
            .is_some_and(|binding| {
                binding.agent_definition_digest == admission.package.definition_digest
            })
        {
            return Vec::new();
        }
        match admission.context_policy {
            ContextPolicy::InvocationOnly => {
                let Some(trigger) = room_message_at_seq(store, &admission.room, triggered_by_seq)
                else {
                    return Vec::new();
                };
                let mut rows = trigger
                    .thread_parent_seq
                    .and_then(|seq| room_message_at_seq(store, &admission.room, seq))
                    .into_iter()
                    .collect::<Vec<_>>();
                if rows.last().is_none_or(|row| row.seq != trigger.seq) {
                    rows.push(trigger);
                }
                rows
            }
            ContextPolicy::RoomRecent | ContextPolicy::RoomHistory => {
                let latest = store
                    .room_latest_durable_seq(&admission.room)
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                let after = latest.checked_sub(ROOM_CONTEXT_TAIL as u64);
                store
                    .transcript_page(&admission.room, after, Some(ROOM_CONTEXT_TAIL))
                    .map(|page| page.messages)
                    .unwrap_or_default()
            }
        }
    })
}

fn build_room_prompt(
    room: &RoomKey,
    agent: &RoomParticipant,
    tail: &[ocean_core::RoomMessage],
    triggered_by_seq: u64,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "You are \"{}\" (participant id `{}`), an agent in the Ocean room \"{}\". \
You were just @-mentioned. Read the recent transcript below and reply directly \
to the mention. Your reply will be posted back into the room for everyone to \
see, so address the room — do not narrate that you are an agent or that you \
were mentioned.\n\n",
        agent.display_name,
        agent.id,
        room.as_str(),
    ));
    out.push_str("--- recent room transcript ---\n");
    for m in tail {
        let marker = if m.seq == triggered_by_seq {
            "  «— mention"
        } else {
            ""
        };
        out.push_str(&format!(
            "[#{seq}] {author}: {body}{marker}\n",
            seq = m.seq,
            author = rendered_author_id(m.author_id.clone()),
            body = room_history_text(m.body.clone(), m.author_kind, m.kind),
            marker = marker,
        ));
    }
    out.push_str("--- end transcript ---\n\nYour reply:");
    out
}

/// Queue an agent turn in response to a room mention, run it asynchronously, and
/// post the reply back into the room. The room store mutex is NEVER held across
/// the await: every store touch goes through `with_rooms`, whose std guard is
/// dropped synchronously before `runtime.prompt(...).await`.
///
/// Anti-loop guardrail #2: the reply is posted with `author_kind = Agent`, and
/// `room_post_message` refuses to evaluate triggers on agent-authored messages,
/// so a reply can never re-convene anyone.
#[derive(Debug, Clone)]
struct RoomTurnFootprint {
    payload: serde_json::Value,
    audit_line: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct QueuedRoomAgentTurn {
    pub(super) request_id: Uuid,
    pub(super) session_id: AgentSessionId,
    pub(super) admission_id: String,
    pub(super) generation: u64,
}

#[derive(Debug)]
enum RoomTurnStartError {
    Authority(ApiError),
    WorkspaceUnavailable,
    RoomHistoryUnavailable,
    RoomResourcesUnavailable,
}

impl RoomTurnStartError {
    fn code(&self) -> &'static str {
        match self {
            Self::Authority(error) => error.code(),
            Self::WorkspaceUnavailable => "workspace_unavailable",
            Self::RoomHistoryUnavailable => "room_history_unavailable",
            Self::RoomResourcesUnavailable => "room_resources_unavailable",
        }
    }

    fn response(self) -> (StatusCode, Json<serde_json::Value>) {
        match self {
            Self::Authority(error) => error.response(),
            Self::WorkspaceUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "workspace_unavailable"})),
            ),
            Self::RoomHistoryUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "room_history_unavailable"})),
            ),
            Self::RoomResourcesUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "room_resources_unavailable"})),
            ),
        }
    }
}

/// One Room task guard uses the existing registry finalizer and legacy bus.
/// It is armed before spawn so abort-before-first-poll also settles the request.
struct RoomTurnFinalizer {
    state: AppState,
    admission: RoomAgentAdmission,
    request_id: Uuid,
    session_id: AgentSessionId,
    cancel: CancellationToken,
    done: CancellationToken,
    watcher: Option<tokio::task::JoinHandle<()>>,
    armed: bool,
}

fn interrupted_room_result(
    request_id: Uuid,
    session_id: AgentSessionId,
) -> ocean_core::PromptResponse {
    ocean_core::PromptResponse {
        ok: false,
        request_id: Some(request_id),
        session_id: Some(core_sid(session_id)),
        code: None,
        wall_ms: 0,
        stdout: String::new(),
        stderr: "room_turn_interrupted".into(),
        cwd: String::new(),
        usage: Default::default(),
    }
}

impl RoomTurnFinalizer {
    fn disarm(&mut self) {
        self.armed = false;
        self.done.cancel();
        if let Some(watcher) = self.watcher.take() {
            watcher.abort();
        }
    }
}
impl Drop for RoomTurnFinalizer {
    fn drop(&mut self) {
        self.done.cancel();
        if let Some(watcher) = self.watcher.take() {
            watcher.abort();
        }
        // Settled turns reach Drop only after authoritative output settlement.
        // Cancel cached capability clones without rewriting terminal facts.
        self.cancel.cancel();
        if !self.armed {
            return;
        }
        let state = self.state.clone();
        let admission = self.admission.clone();
        let request_id = self.request_id;
        let session_id = self.session_id;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let result = interrupted_room_result(request_id, session_id);
                record_prompt_result(&state, request_id, &result, None, None).await;
                if room_agent_authority::append_remote_output_outcome(&state, &admission, "refused", "turn_interrupted").is_err() {
                    tracing::warn!(%request_id, error_code = "room_turn_outcome_audit_failed", "Room turn interruption audit unavailable");
                }
                emit_session_changed(&state.agent_events, session_id);
            });
        } else {
            tracing::error!(%request_id, error_code = "room_turn_finalizer_unavailable", "Room task dropped after runtime shutdown");
        }
    }
}

async fn cancel_room_request(state: &AppState, request_id: Uuid, cancel: &CancellationToken) {
    let permission = {
        let mut requests = state.requests.write().await;
        let permission = requests.get_mut(&request_id).and_then(|control| {
            if control.status.state.is_cancellable() {
                control.status.state = RequestState::Cancelling;
                control.status.message = Some("room_request_authority_changed".into());
                control.status.updated_at = Some(Utc::now());
                control.cancel.cancel();
                control.status.permission_id
            } else {
                None
            }
        });
        cancel.cancel();
        permission
    };
    if let Some(permission) = permission {
        cancel_permission_waiter(&state.permissions, permission, request_id).await;
    }
}

async fn watch_room_request(
    state: AppState,
    authority: RoomOperationAuthority,
    turn_cwd: crate::room_resources::TurnCwd,
    request_id: Uuid,
    done: CancellationToken,
) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_millis(100));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { biased;
            _ = done.cancelled() => break,
            _ = state.shutdown.cancelled() => { cancel_room_request(&state, request_id, &authority.cancel).await; break; }
            _ = authority.cancel.cancelled() => { cancel_room_request(&state, request_id, &authority.cancel).await; break; }
            _ = ticks.tick() => {
                let current = authority.is_current().ok() == Some(true) && with_rooms(&state, |store| {
                    crate::room_resources::resolve_turn_cwd(store, &authority.room, &authority.member).ok().as_ref() == Some(&turn_cwd)
                });
                if !current { cancel_room_request(&state, request_id, &authority.cancel).await; break; }
            }
        }
    }
}

/// Register a generation-bound turn before any room context read, write its
/// durable allow audit, then emit the legacy convene footprint and start the
/// runtime. Refusals never reach the footprint or request registry.
#[allow(clippy::too_many_arguments)]
async fn spawn_room_agent_turn(
    state: AppState,
    admission: RoomAgentAdmission,
    turn_permit: tokio::sync::OwnedSemaphorePermit,
    agent: RoomParticipant,
    triggered_by_seq: u64,
    federated_member_id: Option<String>,
    request_id: Uuid,
    decision_token: Option<String>,
    footprint: Option<RoomTurnFootprint>,
) -> Result<QueuedRoomAgentTurn, RoomTurnStartError> {
    let room = admission.room.clone();
    let cancel = admission.operation_cancel.clone();
    // Covers pre-registration refusals as well as abort-before-first-poll.
    // Move this guard into the executing future so successful queueing does
    // not end the admitted capability lifetime before its output settles.
    let operation_lifetime = cancel.clone().drop_guard();
    let operation_authority =
        RoomOperationAuthority::from_admission(&state, &admission, cancel.clone());
    // Manifest §5: the agent's own default folder, then the room's, then the
    // legacy workspace_root. One function decides for the turn AND for
    // `inspect`, so what the operator sees is what runs.
    let turn_cwd = with_rooms(&state, |reg| {
        crate::room_resources::resolve_turn_cwd(reg, &room, &admission.agent_member_id)
    })
    .map_err(|error| RoomTurnStartError::Authority(ApiError::from(error)))?;
    let Some(cwd) = turn_cwd.cwd().map(str::to_string) else {
        room_agent_authority::append_remote_output_outcome(
            &state,
            &admission,
            "refused",
            "workspace_unavailable",
        )
        .map_err(RoomTurnStartError::Authority)?;
        return Err(RoomTurnStartError::WorkspaceUnavailable);
    };
    let project_id = state
        .runtime
        .project_for_workspace(&cwd)
        .ok()
        .flatten()
        .map(|project| project.id);
    let room_history = if admission.context_policy == ContextPolicy::RoomHistory {
        match state.runtime.admit_room_history(
            &admission,
            Arc::new(DurableRoomHistorySource {
                authority: operation_authority.clone(),
            }),
        ) {
            Ok(history) => Some(history),
            Err(_) => {
                room_agent_authority::append_remote_output_outcome(
                    &state,
                    &admission,
                    "refused",
                    "room_history_unavailable",
                )
                .map_err(RoomTurnStartError::Authority)?;
                return Err(RoomTurnStartError::RoomHistoryUnavailable);
            }
        }
    } else {
        None
    };
    // Phase 2d: every grant that admits this agent becomes a catalog line and
    // the two reserved tools; no grant, no tools. The authority re-validates
    // on every call, so the catalog is display only.
    let catalog = with_rooms(&state, |store| {
        crate::room_resources::admitted_resource_catalog(store, &room, &admission.agent_member_id)
    })
    .map_err(|error| RoomTurnStartError::Authority(ApiError::from(error)))?;
    let room_resources = if catalog.is_empty() {
        None
    } else {
        match state.runtime.admit_room_resources(
            &admission,
            Arc::new(crate::room_resources::DurableRoomResourceAuthority {
                authority: operation_authority.clone(),
                actor: "agent",
            }),
            catalog,
        ) {
            Ok(resources) => Some(resources),
            Err(_) => {
                room_agent_authority::append_remote_output_outcome(
                    &state,
                    &admission,
                    "refused",
                    "room_resources_unavailable",
                )
                .map_err(RoomTurnStartError::Authority)?;
                return Err(RoomTurnStartError::RoomResourcesUnavailable);
            }
        }
    };
    let session_id =
        authorized_room_agent_session_id(&room, &admission.agent_member_id, admission.generation);
    let is_new = state.runtime.session_detail(core_sid(session_id)).is_err();
    let session_lease = state.runtime.session_operation(core_sid(session_id)).await;
    let permission_mode = effective_permission_mode();
    let mut prompt_req = PromptRequest {
        prompt: String::new(),
        images: None,
        request_id: Some(request_id),
        session_id: Some(core_sid(session_id)),
        create_if_missing: is_new,
        max_turns: None,
        yolo: permission_mode == PermissionMode::SkipAll,
        cwd,
        project_id,
        client_type: Some("room".to_string()),
        decision_token,
    };
    let authority = RoomAgentRequestAuthority {
        room: room.clone(),
        agent_member_id: admission.agent_member_id.clone(),
        generation: admission.generation,
        admission_id: admission.admission_id.clone(),
        decision_id: admission.decision_id.clone(),
        approved_definition_digest: admission.package.definition_digest.clone(),
        session_id: core_sid(session_id),
    };
    let registration = register_room_agent_request_checked(
        &state.requests,
        &mut prompt_req,
        format!("room agent {} in room {}", agent.id, room.as_str()),
        authority,
        cancel.clone(),
        || room_agent_authority::append_admission_allow(&state, &admission),
    )
    .await;
    let (_registered_request, cancel) = match registration {
        Ok(value) => value,
        Err(error) => {
            tracing::info!(room = %room, agent = %admission.agent_member_id, reason = error.code(),
                "room-agent admission changed before request registration");
            return Err(RoomTurnStartError::Authority(error));
        }
    };
    let done = CancellationToken::new();
    let mut terminal = RoomTurnFinalizer {
        state: state.clone(),
        admission: admission.clone(),
        request_id,
        session_id,
        cancel: cancel.clone(),
        done: done.clone(),
        watcher: None,
        armed: true,
    };
    emit_session_changed(&state.agent_events, session_id);

    if operation_authority.is_current().ok() == Some(true) {
        if let Some(footprint) = footprint {
            state.agent_events.emit(AgentTurnEvent::Extension {
                extension: "room_trigger".into(),
                payload: footprint.payload,
                scope: None,
            });
            if let Some(line) = footprint.audit_line {
                let _ = append_room_message(
                    &state,
                    &room,
                    "system",
                    RoomParticipantKind::System,
                    RoomMessageKind::System,
                    &line,
                );
            }
        }
    }
    let queued = QueuedRoomAgentTurn {
        request_id,
        session_id,
        admission_id: admission.admission_id.clone(),
        generation: admission.generation,
    };
    let requests = state.requests.clone();
    let handle = tokio::spawn(async move {
        let _operation_lifetime = operation_lifetime;
        let _turn_permit = turn_permit;
        terminal.watcher = Some(tokio::spawn(watch_room_request(
            state.clone(),
            operation_authority.clone(),
            turn_cwd,
            request_id,
            done,
        )));
        let initially_current = operation_authority.is_current().ok() == Some(true);
        let tail = if initially_current {
            authorized_room_transcript_context(&state, &admission, triggered_by_seq, &cancel)
        } else {
            Vec::new()
        };
        if !initially_current {
            cancel_room_request(&state, request_id, &cancel).await;
        }
        let prompt = build_room_prompt(&room, &agent, &tail, triggered_by_seq);
        prompt_req.prompt = match admission.package.instructions_layer.as_deref() {
            Some(instructions) => super::compose_folder_agent_prompt(instructions, &prompt),
            None => prompt,
        };
        // Team-platform P4: this room's own overrides for the agent (local only).
        let settings = &admission.settings_snapshot;
        if let Some(instructions) = settings.instructions.as_deref() {
            prompt_req.prompt =
                super::compose_folder_agent_prompt(instructions, &prompt_req.prompt);
        }

        // G3: the card and the reply attach to the ROOT of the line that
        // convened the agent. When the trigger row is itself a thread reply,
        // its own `thread_parent_seq` is the root; a top-level trigger is its
        // own root.
        let thread_root = with_rooms(&state, |store| {
            room_message_at_seq(store, &room, triggered_by_seq)
        })
        .and_then(|m| m.thread_parent_seq)
        .unwrap_or(triggered_by_seq);

        // Team-platform P3: one live work card per agent turn, folded from the
        // turn's own session events. Subscribe before the turn can emit.
        let tracker = std::sync::Arc::new(std::sync::Mutex::new(
            crate::room_agent_runs::RunTracker::start(
                state.clone(),
                room.clone(),
                &agent.id,
                session_id,
                triggered_by_seq,
                thread_root,
                prompt_req.cwd.clone(),
            ),
        ));
        // The turn's own runtime events drive the card (room turns have no
        // product SSE bridge onto the agent bus).
        let (run_sink, run_events) = mpsc::unbounded_channel();
        let run_watch = tokio::spawn(crate::room_agent_runs::watch_runtime_events(
            tracker.clone(),
            run_events,
        ));
        let with_tracker = |f: &mut dyn FnMut(&mut crate::room_agent_runs::RunTracker)| {
            let mut guard = match tracker.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            f(&mut guard);
        };

        // Team-platform P4: the run's own decision token binds every permission
        // waiter of this turn, and each wait is mirrored onto the work card so
        // the owner can decide in the room (`POST .../runs/{run_id}/permission`).
        let decision_token = match tracker.lock() {
            Ok(guard) => guard.mint_decision_token(),
            Err(poisoned) => poisoned.into_inner().mint_decision_token(),
        };
        let hook_tracker = tracker.clone();
        let wait_hook: PermissionWaitHook = Arc::new(
            move |pending: Option<(PermissionId, &str, &serde_json::Value)>| {
                let mut t = match hook_tracker.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match pending {
                    Some((permission_id, tool, args)) => {
                        t.awaiting_permission(permission_id.to_string(), tool, args)
                    }
                    None => t.permission_resolved(),
                }
            },
        );
        let control = build_room_prompt_control(
            &state,
            request_id,
            Some(core_sid(session_id)),
            permission_mode,
            cancel.clone(),
            decision_token,
            wait_hook,
        )
        .with_event_sink(run_sink);
        let control = room_agent_authority::apply_admission_to_control(control, &admission);
        let control = room_agent_authority::attach_operation_authority(
            control,
            &state,
            &admission,
            cancel.clone(),
        );
        let control = match room_resources {
            Some(resources) => control.with_room_resources(resources),
            None => control,
        };
        let control = match room_history {
            Some(history) => control.with_room_history(history),
            None => control,
        };
        // Room tools post into this run's thread. Local rooms only: federated
        // threads are not writable yet.
        let control = if federated_member_id.is_none() {
            let tools = crate::room_tools::room_turn_tools(crate::room_tools::RoomTurnBinding {
                state: state.clone(),
                admission: admission.clone(),
                session_id,
                cancel: cancel.clone(),
                thread_root,
                tracker: tracker.clone(),
            });
            match state.runtime.admit_room_turn_tools(&admission, tools) {
                Ok(tools) => control.with_room_turn_tools(tools),
                Err(_) => control,
            }
        } else {
            control
        };
        #[cfg(test)]
        capture_room_turn(&agent.id, &prompt_req.prompt, &control);

        if operation_authority.is_current().ok() != Some(true) {
            cancel_room_request(&state, request_id, &cancel).await;
        }

        let result = tokio::select! { biased;
            _ = cancel.cancelled() => interrupted_room_result(request_id, session_id),
            result = state.runtime.prompt_with_lease(prompt_req, control, &session_lease) => result,
        };
        if cancel.is_cancelled() || operation_authority.is_current().ok() != Some(true) {
            cancel_room_request(&state, request_id, &cancel).await;
        }
        record_prompt_result(&state, request_id, &result, None, None).await;
        terminal.disarm();
        emit_session_changed(&state.agent_events, session_id);
        // The turn dropped its sink; drain the watcher before the terminal
        // write so no step is lost and no trailing event lands after it.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), run_watch).await;

        let parked = match tracker.lock() {
            Ok(guard) => guard.is_parked(),
            Err(poisoned) => poisoned.into_inner().is_parked(),
        };
        // The card projects the request's authoritative terminal state: a
        // request the registry settled `Cancelled` while its admission is
        // still current was cancelled by its owner, not failed.
        let request_cancelled = state
            .requests
            .read()
            .await
            .get(&request_id)
            .is_some_and(|control| control.status.state == RequestState::Cancelled);
        if cancel.is_cancelled() || request_cancelled {
            if room_agent_authority::append_remote_output_outcome(
                &state,
                &admission,
                "refused",
                "room_request_authority_changed",
            )
            .is_err()
            {
                tracing::warn!(%request_id, error_code = "room_turn_outcome_audit_failed", "Room cancellation audit unavailable");
            }
            if request_cancelled
                && room_agent_authority::admission_generation_is_current(&state, &admission)
            {
                with_tracker(&mut |t| t.finish_cancelled());
            } else {
                with_tracker(&mut |t| t.finish_failed("room_request_authority_changed"));
            }
            return;
        }
        if !result.ok {
            with_tracker(&mut |t| t.finish_failed("turn_failed"));
        }
        if result.ok {
            let body = clamp_room_message_body(result.stdout.trim());
            let mut reply_seq = None;
            // A `room_ask` already posted the question and parked the run.
            if !body.is_empty() && !parked {
                if let Some(member_id) = federated_member_id.as_deref() {
                    if state
                        .room_federation
                        .enqueue_authorized_federated_agent_message(
                            &room,
                            member_id,
                            admission.generation,
                            &admission.admission_id,
                            body.as_ref(),
                            &operation_authority,
                        )
                        .await
                        .is_err()
                    {
                        let reason = if room_agent_authority::admission_generation_is_current(
                            &state, &admission,
                        ) {
                            "remote_enqueue_failed"
                        } else {
                            "authority_changed_before_remote_enqueue"
                        };
                        if let Err(error) = room_agent_authority::append_remote_output_outcome(
                            &state, &admission, "refused", reason,
                        ) {
                            tracing::warn!(room = %room, agent = %agent.id,
                                reason = error.code(),
                                "failed to persist federated room-agent refusal audit");
                        }
                        tracing::warn!(room = %room, outcome = "agent_reply_enqueue_failed",
                            "federated agent reply enqueue failed");
                    }
                } else {
                    match append_authorized_room_agent_reply(
                        &state,
                        &admission,
                        body.as_ref(),
                        Some(thread_root),
                        session_id,
                        &cancel,
                    ) {
                        Ok(message) => reply_seq = Some(message.seq),
                        Err(_) => {
                            if room_agent_authority::append_remote_output_outcome(
                                &state,
                                &admission,
                                "refused",
                                "authority_changed_before_local_output",
                            )
                            .is_err()
                            {
                                tracing::warn!(%request_id, error_code = "room_turn_outcome_audit_failed", "Room output refusal audit unavailable");
                            }
                        }
                    }
                }
            }
            with_tracker(&mut |t| t.finish_done(body.as_ref(), reply_seq));
        } else if federated_member_id.is_none() {
            if append_authorized_room_agent_failure(&state, &admission, session_id, &cancel)
                .is_err()
            {
                tracing::warn!(room = %room, agent = %agent.id, error_code = "room_failure_write_refused",
                    "failed room-agent turn lost authority before durable failure row");
                if room_agent_authority::append_remote_output_outcome(
                    &state,
                    &admission,
                    "refused",
                    "authority_changed_before_failure_output",
                )
                .is_err()
                {
                    tracing::warn!(%request_id, error_code = "room_turn_outcome_audit_failed", "Room failure refusal audit unavailable");
                }
            }
        } else if room_agent_authority::append_remote_output_outcome(
            &state,
            &admission,
            "failed",
            "turn_failed",
        )
        .is_err()
        {
            tracing::warn!(%request_id, error_code = "room_turn_outcome_audit_failed",
                "Room failure audit unavailable");
        }
    });
    attach_request_handle(&requests, request_id, handle).await;
    Ok(queued)
}

// Execute the real admitted path while preserving a saved memory-handle clone
// in the caller. This helper is test-only; production interfaces stay private.
#[cfg(test)]
pub(super) async fn complete_room_memory_lifetime_fixture(
    state: &AppState,
    admission: RoomAgentAdmission,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let room = admission.room.clone();
    let token = admission.operation_cancel.clone();
    let total = state.turn_limiter.available_permits() + 1;
    let trigger = append_room_message(
        state,
        &room,
        "human",
        RoomParticipantKind::Human,
        RoomMessageKind::Message,
        "complete the isolated fake turn",
    )
    .unwrap();
    let agent = RoomParticipant {
        id: admission.agent_member_id.clone(),
        kind: RoomParticipantKind::Agent,
        display_name: admission.package.display_name.clone(),
    };
    let queued = spawn_room_agent_turn(
        state.clone(),
        admission,
        permit,
        agent,
        trigger.seq,
        None,
        Uuid::new_v4(),
        None,
        None,
    )
    .await
    .unwrap();
    for _ in 0..200 {
        if state
            .requests
            .read()
            .await
            .get(&queued.request_id)
            .is_some_and(|request| request.status.state == RequestState::Completed)
            && state.turn_limiter.available_permits() == total
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.requests.read().await[&queued.request_id].status.state,
        RequestState::Completed
    );
    assert_eq!(state.turn_limiter.available_permits(), total);
    assert!(token.is_cancelled());
    assert!(with_rooms(state, |store| store.get(&room))
        .unwrap()
        .unwrap()
        .transcript
        .iter()
        .any(|row| row.author_kind == RoomParticipantKind::Agent
            && row.kind == RoomMessageKind::Message));
}

#[derive(serde::Deserialize)]
pub(super) struct TranscriptQuery {
    /// If set, return only entries with `seq > after_seq` (live-tail).
    #[serde(default)]
    pub(super) after_seq: Option<u64>,
    /// Max rows to return in this page (OCEAN-249). Omitted ⇒ the store's default
    /// cap; any value is clamped to `MAX_TRANSCRIPT_LIMIT`. Transcript reads are
    /// never unbounded — page with the returned `next_seq` cursor.
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

/// Read one bounded transcript page for a room, transparently falling back to the
/// soft-closed audit view (OCEAN-249 + OCEAN-170).
///
/// The open path defers to `transcript_page` (the `LIMIT`ed query). For a closed
/// room — a finished call's frozen transcript that must stay queryable — the audit
/// getter still returns a (now `MAX_TRANSCRIPT_LIMIT`-bounded) record, so we apply
/// the same `after_seq` filter and `limit + 1` sentinel paging in memory to hand
/// back an identical `TranscriptPage` shape regardless of room state. `Ok(None)`
/// from the audit view (room never existed) is mapped back to `UnknownRoom` so the
/// handlers preserve their 404.
fn read_transcript_page(
    reg: &ocean_store::SqliteRoomStore,
    key: &RoomKey,
    after_seq: Option<u64>,
    limit: Option<usize>,
) -> Result<ocean_store::TranscriptPage, ocean_store::RoomStoreError> {
    reg.transcript_page_including_closed(key, after_seq, limit)
}

/// `GET /v1/rooms/persistent/{key}/transcript?after_seq=N&limit=M` — read one
/// bounded page of a room's transcript, optionally only entries after a given seq.
///
/// Bounded + paginated (OCEAN-249): the read is capped (default cap when `limit`
/// is omitted, clamped to `MAX_TRANSCRIPT_LIMIT`), and the response carries
/// additive `next_seq` (cursor to replay as `after_seq`) and `has_more` fields so
/// a client can page through a long transcript instead of forcing a full-table
/// read on every call. The `transcript` array shape is unchanged.
///
/// Falls back to the audit (soft-closed) view when the room is closed: a finished
/// call closes its room on `CallEnded` (OCEAN-170), but its transcript must stay
/// queryable afterwards — that frozen record is the whole reason it was persisted.
pub(super) async fn room_transcript(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    let result = with_rooms(&state, |reg| {
        let closed = reg.get(&key)?.is_none();
        read_transcript_page(reg, &key, q.after_seq, q.limit).map(|page| (page, closed))
    });
    match result {
        Ok((page, closed)) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "closed": closed,
                "transcript": projected_transcript(page.messages),
                "next_seq": page.next_seq,
                "has_more": page.has_more,
            })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `GET /v1/rooms/persistent/{key}/snapshot` — full room hydration in one read:
/// the room entity (id, name, roster, timestamps, trigger policy), its complete
/// transcript, and `last_seq` so the caller can immediately tail live updates via
/// `GET /v1/rooms/persistent/{key}/events?after_seq=last_seq`.
///
/// This is the store-backed realization of the collaboration model's "Room
/// hydration / snapshot" step (OCEAN-232): switching into a room must load full
/// state, not just subscribe to future events. Persistent rooms carry everything
/// hydration needs, so this endpoint serves the durable snapshot directly.
///
/// Like `room_get`/`room_transcript`, falls back to the soft-closed audit view so
/// a finished call's frozen room (closed on `CallEnded`, OCEAN-170) stays
/// hydratable for replay.
pub(super) async fn room_snapshot(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    // Hydrate room metadata (entity + roster) and the FIRST bounded transcript page
    // under one lock. The transcript is no longer the room's entire log poured into
    // one response (OCEAN-249): a long-lived call room would make every hydration a
    // full-table read. We serve `limit` rows + a `next_seq` cursor so the client
    // immediately knows whether to page (`/transcript?after_seq=next_seq`) or tail
    // (`/events?after_seq=last_seq`). Both reads prefer the live room and fall back
    // to the soft-closed audit view (OCEAN-170). The std mutex guard is dropped
    // inside `with_rooms`; it is never held across an `.await`.
    let result = with_rooms(&state, |reg| {
        // Room metadata: live first, then audit for a soft-closed room.
        let closed = reg.get(&key)?.is_none();
        let record = reg.get_including_closed(&key)?;
        let Some(record) = record else {
            return Ok(None);
        };
        // First bounded page of the transcript (from the start of the log).
        let page = read_transcript_page(reg, &key, q.after_seq, q.limit)?;
        // Access projection (S2-P1): the room's federated state, outbox, and
        // member roster (Local if no access row exists).
        let access = reg.room_access(&key)?;
        let aliases = room_retirement::aliases_projection(reg, &key)?;
        Ok(Some((record, page, access, closed, aliases)))
    });
    match result {
        Ok(Some((rec, page, access, closed, aliases))) => {
            let last_seq = page.messages.last().map(|m| m.seq);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "closed": closed,
                    "room": rec.room.clone(),
                    "participants": rec.room.participants,
                    "transcript": projected_transcript(page.messages),
                    "last_seq": last_seq,
                    "next_seq": page.next_seq,
                    "has_more": page.has_more,
                    "access": access,
                    "aliases": aliases.aliases,
                    "aliases_truncated": aliases.truncated,
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("no room with key '{key}'") })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RoomReadCursorPatchRequest {
    pub(super) read_seq: u64,
}

/// Canonical `GET`/`PATCH .../read-cursor` response body — used for BOTH
/// Local and Live rooms so callers see one schema regardless of access
/// state. `read_seq` is stringified (matching every other sequence number
/// in this API) because raw `u64` values above 2^53 are not
/// JS-number-precision-safe.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct RoomReadCursorBody {
    room_id: String,
    read_seq: Option<String>,
}

fn local_room_read_cursor_principal() -> &'static str {
    "daemon-local-room-read-cursor"
}

/// The read-cursor mirror principal for a Live room is the local human's
/// per-room federated member id installed with the room credential — NOT a
/// fixed constant. `room_federation::FederationSupervisor::room_get_read_cursor`
/// / `room_patch_read_cursor` always mutate `room_read_cursor_mirrors` keyed
/// by `credential.local_human_member_id`, so any reader here must resolve the
/// exact same key or it will silently observe an always-empty cursor (H1).
fn live_room_read_cursor_principal(
    store: &ocean_store::SqliteRoomStore,
    key: &RoomKey,
) -> Result<Option<String>, ocean_store::RoomStoreError> {
    Ok(store
        .room_credential(key)?
        .map(|credential| credential.local_human_member_id))
}

fn room_read_cursor_unsupported_response(
    state: RoomAccessState,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "ok": false,
            "code": "room_read_cursor_unsupported",
            "error": format!("read cursor unsupported for access state '{state:?}'")
                .to_lowercase()
                .replace("roomaccessstate::", "")
        })),
    )
}

pub(super) async fn room_get_read_cursor(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    match with_rooms(&state, |store| {
        let access = store.room_access(&key)?;
        match access.state {
            RoomAccessState::Local => Ok(Ok(store
                .room_read_cursor(&key, local_room_read_cursor_principal())?
                .read_seq)),
            RoomAccessState::Live => Ok(Err(RoomAccessState::Live)),
            other => Ok(Err(other)),
        }
    }) {
        Ok(Ok(read_seq)) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "cursor": RoomReadCursorBody {
                    room_id: key.as_str().to_string(),
                    read_seq: read_seq.map(|seq| seq.to_string()),
                }
            })),
        ),
        Ok(Err(RoomAccessState::Live)) => {
            match state.room_federation.room_get_read_cursor(&key).await {
                Ok(cursor) => (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "cursor": RoomReadCursorBody {
                            room_id: key.as_str().to_string(),
                            read_seq: cursor
                                .mirrored_upstream_read_seq
                                .map(|seq| seq.to_string()),
                        }
                    })),
                ),
                Err(crate::room_federation::IntentError::Forbidden) => (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "ok": false, "error": "membership_revoked" })),
                ),
                Err(crate::room_federation::IntentError::Conflict) => {
                    room_read_cursor_unsupported_response(RoomAccessState::Live)
                }
                Err(crate::room_federation::IntentError::NotFound) => (
                    StatusCode::NOT_FOUND,
                    Json(
                        json!({ "ok": false, "error": format!("no open room with key '{}'", key) }),
                    ),
                ),
                Err(_) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "ok": false, "error": "federated read cursor unavailable" })),
                ),
            }
        }
        Ok(Err(state)) => room_read_cursor_unsupported_response(state),
        Err(e) => room_store_error_response(e),
    }
}

pub(super) async fn room_patch_read_cursor(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    let req: RoomReadCursorPatchRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return invalid_request_response(),
    };
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    match with_rooms(&state, |store| {
        let access = store.room_access(&key)?;
        if access.state == RoomAccessState::Local {
            let cursor = store.update_room_read_cursor(
                &key,
                local_room_read_cursor_principal(),
                RoomReadCursorUpdateRequest {
                    read_seq: req.read_seq,
                },
            )?;
            return Ok(Ok(cursor.read_seq));
        }
        if access.state == RoomAccessState::Live {
            return Ok(Err(RoomAccessState::Live));
        }
        Ok(Err(access.state))
    }) {
        Ok(Ok(read_seq)) => {
            publish_room_read_cursor_wake(&state, &key);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "cursor": RoomReadCursorBody {
                        room_id: key.as_str().to_string(),
                        read_seq: read_seq.map(|seq| seq.to_string()),
                    }
                })),
            )
        }
        Ok(Err(RoomAccessState::Live)) => match state
            .room_federation
            .room_patch_read_cursor(&key, req.read_seq)
            .await
        {
            Ok(cursor) => (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "cursor": RoomReadCursorBody {
                        room_id: key.as_str().to_string(),
                        read_seq: cursor.mirrored_upstream_read_seq.map(|seq| seq.to_string()),
                    }
                })),
            ),
            Err(crate::room_federation::IntentError::Conflict) => {
                room_read_cursor_unsupported_response(RoomAccessState::Live)
            }
            Err(crate::room_federation::IntentError::Forbidden) => (
                StatusCode::FORBIDDEN,
                Json(json!({ "ok": false, "error": "membership_revoked" })),
            ),
            Err(crate::room_federation::IntentError::NotFound) => (
                StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "error": format!("no open room with key '{}'", key) })),
            ),
            Err(crate::room_federation::IntentError::Protocol) => invalid_request_response(),
            Err(_) => (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "ok": false, "error": "federated read cursor unavailable" })),
            ),
        },
        Ok(Err(state)) => room_read_cursor_unsupported_response(state),
        Err(e) => room_store_error_response(e),
    }
}

#[derive(Debug, serde::Deserialize, Default)]
pub(super) struct RoomEventsQuery {
    /// Replay starts strictly after this room-scoped sequence number.
    #[serde(default)]
    after_seq: Option<u64>,
}

type RoomEventsError = (StatusCode, Json<serde_json::Value>);
type RoomTailSeam = (oneshot::Sender<()>, oneshot::Receiver<()>);

fn room_events_error(status: StatusCode, code: &str, error: impl Into<String>) -> RoomEventsError {
    (
        status,
        Json(json!({ "ok": false, "code": code, "error": error.into() })),
    )
}

fn room_resume_seq(
    headers: &HeaderMap,
    query: &RoomEventsQuery,
) -> Result<Option<u64>, RoomEventsError> {
    let Some(raw) = headers.get("last-event-id") else {
        return Ok(query.after_seq);
    };
    let value = raw.to_str().map_err(|_| {
        room_events_error(
            StatusCode::BAD_REQUEST,
            "invalid_last_event_id",
            "Last-Event-ID must be an unsigned integer",
        )
    })?;
    value.parse::<u64>().map(Some).map_err(|_| {
        room_events_error(
            StatusCode::BAD_REQUEST,
            "invalid_last_event_id",
            "Last-Event-ID must be an unsigned integer",
        )
    })
}

/// Page every durable row after `last_sent_seq`, sending in ascending seq order.
/// Each query is bounded; newly committed rows may extend the loop, while a
/// caught-up empty/final page returns control to the wake receiver.
async fn send_room_catch_up(
    state: &AppState,
    room: &RoomKey,
    last_sent_seq: &mut Option<u64>,
    tx: &mpsc::Sender<RoomMessage>,
) -> Result<bool, ocean_store::RoomStoreError> {
    loop {
        let page = with_rooms(state, |store| {
            store.transcript_page_including_closed(room, *last_sent_seq, Some(128))
        })?;
        if page.messages.is_empty() {
            return Ok(true);
        }
        for message in page.messages {
            if last_sent_seq.is_some_and(|last| message.seq <= last) {
                continue;
            }
            let seq = message.seq;
            if tx.send(message).await.is_err() {
                return Ok(false);
            }
            *last_sent_seq = Some(seq);
        }
        if !page.has_more {
            return Ok(true);
        }
    }
}

async fn run_room_tail(
    state: AppState,
    room: RoomKey,
    resume: Option<u64>,
    mut hints: broadcast::Receiver<RoomWakeHint>,
    tx: mpsc::Sender<RoomMessage>,
    seam: Option<RoomTailSeam>,
) {
    let mut last_sent_seq = resume;
    match send_room_catch_up(&state, &room, &mut last_sent_seq, &tx).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(room = %room, %error, "room SSE replay failed");
            return;
        }
    }

    if !with_rooms(&state, |store| store.get(&room).ok().flatten().is_some()) {
        return;
    }
    // Tests can hold this exact replay/live seam open. The broadcast receiver was
    // already subscribed, so hints accumulate while replay is paused; production
    // passes `None` and continues immediately.
    if let Some((ready, release)) = seam {
        let _ = ready.send(());
        tokio::select! {
            _ = tx.closed() => return,
            _ = release => {}
        }
    }

    loop {
        let hint = tokio::select! {
            _ = tx.closed() => return,
            hint = hints.recv() => hint,
        };
        match hint {
            Ok(hint) if hint.room != room => continue,
            Ok(hint) if Some(hint.seq) <= last_sent_seq => {
                if !with_rooms(&state, |store| store.get(&room).ok().flatten().is_some()) {
                    return;
                }
                continue;
            }
            Ok(hint) => {
                let expected = last_sent_seq.map_or(0, |last| last.saturating_add(1));
                if hint.seq > expected {
                    tracing::debug!(
                        room = %room,
                        observed_seq = hint.seq,
                        ?last_sent_seq,
                        "room SSE observed seq gap; paging durable log"
                    );
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(room = %room, skipped, "room SSE wake receiver lagged; paging durable log");
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }

        match send_room_catch_up(&state, &room, &mut last_sent_seq, &tx).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(room = %room, %error, "room SSE durable catch-up failed");
                return;
            }
        }
        if !with_rooms(&state, |store| store.get(&room).ok().flatten().is_some()) {
            return;
        }
    }
}

#[allow(dead_code)]
fn room_message_tail(
    state: AppState,
    room: RoomKey,
    resume: Option<u64>,
    hints: broadcast::Receiver<RoomWakeHint>,
    seam: Option<RoomTailSeam>,
) -> ReceiverStream<RoomMessage> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(run_room_tail(state, room, resume, hints, tx, seam));
    ReceiverStream::new(rx)
}

/// `GET /v1/rooms/persistent/{key}/events?after_seq=N` — durable replay plus a
/// room-scoped live SSE tail. Every frame is `event: room_message`, `id: <seq>`
/// with the exact existing `RoomMessage` JSON. SQLite is authoritative; the
/// bounded broadcast carries wake hints only.
///
/// S2-P1 merged SSE: also carries `event: room_access` frames (no `id`) with
/// RoomAccessProjection JSON. An initial access frame ships before any messages;
/// a separate access-tail task re-reads on access wake hints.
pub(super) async fn room_events(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    Query(query): Query<RoomEventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, RoomEventsError> {
    let resume = room_resume_seq(&headers, &query)?;
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return Err(room_events_error(
            StatusCode::BAD_REQUEST,
            "invalid_room_key",
            "invalid room key; must be non-empty",
        ));
    }
    let room = RoomKey::new(trimmed);
    if room.as_str().starts_with("call:") {
        return Err(room_events_error(
            StatusCode::BAD_REQUEST,
            "call_room_events_unsupported",
            "call-prefixed rooms do not support room event streams",
        ));
    }

    // Subscribe to ALL wake buses BEFORE the first replay query. The cursor
    // tail gets its own independent access-bus subscription (in addition to
    // `access_hints` consumed by `run_room_access_tail` below) so an access
    // wake alone — e.g. a federated Connecting/Recovering -> Live transition
    // that does not itself carry a fresh upstream cursor frame — is enough to
    // make the cursor tail re-check and emit (see `run_room_read_cursor_tail`).
    let message_hints = state.room_wakes.subscribe();
    let access_hints = state.room_access_wakes.subscribe();
    let cursor_access_hints = state.room_access_wakes.subscribe();
    let cursor_hints = state.room_read_cursor_wakes.subscribe();
    // Team-platform P3: agent-run cards ride the access wake bus (a run write
    // publishes an access wake; the access tail dedups unchanged projections).
    let run_hints = state.room_access_wakes.subscribe();

    // Verify room exists (open rooms only) and read initial access snapshot.
    //
    // `initial_cursor` is `None` for the transitional/dead access states
    // (Connecting/Recovering/Revoked) where the read-cursor projection is
    // undefined (F1/F3). The cursor tail below is spawned unconditionally
    // regardless of the *current* access state: it re-reads access fresh on
    // every wake hint, so a connection opened mid-Connecting/Recovering stays
    // subscribed through the transition and still emits the current cursor
    // the moment access becomes Live, without requiring the client to
    // reconnect (see `run_room_read_cursor_tail`).
    let (initial_access, initial_cursor) = match with_rooms(&state, |store| {
        if store.get(&room)?.is_none() {
            return Err(ocean_store::RoomStoreError::UnknownRoom(room.clone()));
        }
        let access = store.room_access(&room)?;
        let cursor = match access.state {
            RoomAccessState::Local => Some(RoomReadCursorBody {
                room_id: room.as_str().to_string(),
                read_seq: store
                    .room_read_cursor(&room, local_room_read_cursor_principal())?
                    .read_seq
                    .map(|seq| seq.to_string()),
            }),
            RoomAccessState::Live => Some(RoomReadCursorBody {
                room_id: room.as_str().to_string(),
                read_seq: match live_room_read_cursor_principal(store, &room)? {
                    Some(principal) => store
                        .room_read_cursor(&room, &principal)?
                        .mirrored_upstream_read_seq
                        .map(|seq| seq.to_string()),
                    None => None,
                },
            }),
            _ => None,
        };
        Ok((access, cursor))
    }) {
        Ok(proj) => proj,
        Err(ocean_store::RoomStoreError::UnknownRoom(_)) => {
            return Err(room_events_error(
                StatusCode::NOT_FOUND,
                "room_not_found",
                format!("no open room with key '{room}'"),
            ));
        }
        Err(error) => return Err(room_store_error_response(error)),
    };

    // Existing message tail as a stream of SSE events.
    let msg_stream = room_message_tail(state.clone(), room.clone(), resume, message_hints, None)
        .map(|message| -> Result<Event, Infallible> {
            let seq = message.seq.to_string();
            let data = serde_json::to_string(&projected_room_message(message.clone()))
                .expect("RoomMessage serializable");
            Ok(Event::default().id(seq).event("room_message").data(data))
        });

    // Access tail.
    let (access_tx, access_rx) = mpsc::channel::<RoomAccessProjection>(16);
    tokio::spawn(run_room_access_tail(
        state.clone(),
        room.clone(),
        Some(initial_access.clone()),
        access_hints,
        access_tx,
    ));
    let acc_stream = ReceiverStream::new(access_rx).map(|proj| -> Result<Event, Infallible> {
        let data = serde_json::to_string(&proj).expect("RoomAccessProjection serializable");
        Ok(Event::default().event("room_access").data(data))
    });

    // Cursor tail: spawned unconditionally, regardless of the access state
    // observed above. If `/events` is opened while a federated room is
    // Connecting or Recovering — normal startup/reconnect states — the tail
    // must stay subscribed on `cursor_hints` for this same long-lived
    // connection rather than being dropped, so it can wake and emit the
    // current cursor the instant access becomes Live without requiring the
    // client to reconnect. `run_room_read_cursor_tail` re-reads access fresh
    // on every wake hint and already suppresses emissions for the
    // unsupported states (Connecting/Recovering/Revoked) on its own.
    let (cursor_tx, cursor_rx) = mpsc::channel::<RoomReadCursorBody>(16);
    tokio::spawn(run_room_read_cursor_tail(
        state.clone(),
        room.clone(),
        initial_cursor.clone(),
        cursor_hints,
        cursor_access_hints,
        cursor_tx,
    ));
    let cursor_stream = ReceiverStream::new(cursor_rx).map(|cursor| -> Result<Event, Infallible> {
        let data = serde_json::to_string(&cursor).expect("RoomReadCursorBody serializable");
        Ok(Event::default().event("room_read_cursor").data(data))
    });

    // Merge: initial access frame first, then interleave messages + access updates.
    let init_data =
        serde_json::to_string(&initial_access).expect("RoomAccessProjection serializable");
    let init_event = Ok(Event::default().event("room_access").data(init_data));
    let cursor_init = initial_cursor
        .map(|cursor| {
            let data = serde_json::to_string(&cursor).expect("RoomReadCursorBody serializable");
            Ok(Event::default().event("room_read_cursor").data(data))
        })
        .into_iter();
    let cursor_events: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        Box::pin(tokio_stream::iter(cursor_init).chain(cursor_stream));

    // Agent-run tail: recent runs first, then every changed run.
    let (run_tx, run_rx) = mpsc::channel::<RoomAgentRun>(32);
    tokio::spawn(run_room_agent_run_tail(
        state.clone(),
        room.clone(),
        run_hints,
        run_tx,
    ));
    let run_stream = ReceiverStream::new(run_rx).map(|run| -> Result<Event, Infallible> {
        let data = serde_json::to_string(&run).expect("RoomAgentRun serializable");
        Ok(Event::default().event("room_agent_run").data(data))
    });

    let merged = tokio_stream::once(init_event)
        .chain(msg_stream.merge(acc_stream))
        .merge(cursor_events)
        .merge(run_stream);
    let stream = sse_until_shutdown(merged, state.shutdown.clone());
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL)))
}

// ── Team-platform P3 agent-run SSE tail ──────────────────────────────────────

/// Agent-run tail: send the room's recent runs, then on every access wake (or
/// lag) re-read them and send each run whose projection changed. Selects
/// `tx.closed()` while idle so client disconnect cleans up.
async fn run_room_agent_run_tail(
    state: AppState,
    room: RoomKey,
    mut hints: broadcast::Receiver<RoomAccessWakeHint>,
    tx: mpsc::Sender<RoomAgentRun>,
) {
    let mut seen: std::collections::HashMap<String, RoomAgentRun> =
        std::collections::HashMap::new();
    let mut first = true;
    loop {
        if !first {
            let should_read = tokio::select! {
                _ = tx.closed() => return,
                res = hints.recv() => match res {
                    Ok(hint) => hint.room == room,
                    Err(broadcast::error::RecvError::Lagged(_)) => true,
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            if !should_read {
                continue;
            }
        }
        first = false;
        let runs = match with_rooms(&state, |store| {
            store.room_agent_runs(&room, crate::room_agent_runs::ROOM_RUNS_LIMIT)
        }) {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(room = %room, %e, "room agent run tail read failed");
                return;
            }
        };
        for run in runs {
            if seen.get(&run.run_id) == Some(&run) {
                continue;
            }
            seen.insert(run.run_id.clone(), run.clone());
            if tx.send(run).await.is_err() {
                return;
            }
        }
    }
}

/// `GET /v1/rooms/persistent/{key}/runs` — the room's recent agent work
/// cards, oldest first (team-platform P3).
pub(super) async fn room_agent_runs_list(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    match with_rooms(&state, |store| {
        store.room_agent_runs(&key, crate::room_agent_runs::ROOM_RUNS_LIMIT)
    }) {
        Ok(runs) => (StatusCode::OK, Json(json!({ "ok": true, "runs": runs }))),
        Err(e) => room_store_error_response(e),
    }
}

/// Body for `POST /v1/rooms/persistent/{key}/runs/{run_id}/permission`. The
/// decision is bound to the exact pending request the owner saw: its
/// `permission_id` (required, like the body of `/v1/permissions/{id}/decision`)
/// and, when the client has it, the pending `tool` name.
#[derive(Debug, Deserialize)]
pub(super) struct RoomRunPermissionDecisionBody {
    pub(super) permission_id: String,
    #[serde(default)]
    pub(super) tool: Option<String>,
    #[serde(flatten)]
    pub(super) decision: PermissionDecisionBody,
}

fn stale_permission(error: &'static str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": error })),
    )
}

/// `POST /v1/rooms/persistent/{key}/runs/{run_id}/permission` — the owner's
/// in-room approve/deny for a run's pending tool permission (team-platform
/// P4). The existing header-only Room operator authorizes the decision; the
/// body names the exact pending request (`permission_id`, optional `tool`).
/// Anything but the run's current pending request (stale, already decided,
/// another tool) is a 409 that leaves every waiter untouched, so a retried or
/// double-clicked Allow can never approve a later request and AllowSession only
/// applies to the request the owner saw. The run's daemon-held token then binds
/// the decision to that waiter through the same authority as
/// `/v1/permissions/{id}/decision`.
pub(super) async fn room_agent_run_permission(
    State(state): State<AppState>,
    Path((key, run_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<RoomRunPermissionDecisionBody>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = room_agent_authority::operator(&state, &headers) {
        return error.response();
    }

    let Ok(Json(body)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid_request" })),
        );
    };
    let Ok(permission_id) = body.permission_id.trim().parse::<PermissionId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid_request" })),
        );
    };
    let key = RoomKey::new(key.trim());
    let run = match with_rooms(&state, |store| store.room_agent_run(run_id.trim())) {
        Ok(Some(run)) if run.room_id == key => run,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "error": "run_not_found" })),
            )
        }
        Err(e) => return room_store_error_response(e),
    };
    let Some(pending) = run.pending_permission.as_ref() else {
        return stale_permission("no_pending_permission");
    };
    if pending.permission_id.parse::<PermissionId>().ok() != Some(permission_id) {
        return stale_permission("stale_permission");
    }
    if body
        .tool
        .as_deref()
        .is_some_and(|tool| tool.trim() != pending.tool)
    {
        return stale_permission("permission_tool_mismatch");
    }
    let Some(token) = crate::room_agent_runs::run_decision_token(&run.run_id) else {
        return stale_permission("no_pending_permission");
    };
    // The live waiter must be the same call the card projects.
    match state.permissions.read().await.get(&permission_id) {
        None => return stale_permission("permission_already_decided"),
        Some(waiter) if waiter.status.tool != pending.tool => {
            return stale_permission("permission_tool_mismatch")
        }
        Some(_) => {}
    }
    let (status, resp) =
        resolve_permission_waiter(&state, permission_id, body.decision, Some(&token)).await;
    if status == StatusCode::NOT_FOUND {
        // Lost a race with another decision or a cancellation.
        return stale_permission("permission_already_decided");
    }
    (
        status,
        Json(json!({ "ok": resp.ok, "message": resp.message })),
    )
}

/// Longest per-room instructions overlay (characters).
const ROOM_AGENT_INSTRUCTIONS_CHARS: usize = 4000;
/// Longest model alias (characters).
const ROOM_AGENT_MODEL_CHARS: usize = 128;

/// Trim, drop empties, and bound a settings body.
fn normalize_agent_settings(mut s: RoomAgentSettings) -> Result<RoomAgentSettings, &'static str> {
    let clean = |v: Option<String>| v.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    s.instructions = clean(s.instructions);
    s.model = clean(s.model);
    if s.instructions
        .as_ref()
        .is_some_and(|i| i.chars().count() > ROOM_AGENT_INSTRUCTIONS_CHARS)
    {
        return Err("instructions_too_long");
    }
    if let Some(m) = &s.model {
        if m.chars().count() > ROOM_AGENT_MODEL_CHARS
            || !m
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
        {
            return Err("invalid_model");
        }
    }
    Ok(s)
}

/// The room's agent participant `agent_id`, or a typed error response.
fn room_agent_or_error(
    state: &AppState,
    key: &RoomKey,
    agent_id: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match with_rooms(state, |store| store.get(key)) {
        Ok(Some(rec)) => {
            if resolve_agent_participant(&rec.room.participants, agent_id).is_some() {
                Ok(())
            } else {
                Err((
                    StatusCode::NOT_FOUND,
                    Json(json!({ "ok": false, "error": "agent_not_in_room" })),
                ))
            }
        }
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "room_not_found" })),
        )),
        Err(e) => Err(room_store_error_response(e)),
    }
}

/// `GET /v1/rooms/persistent/{key}/agents/{agent_id}/settings` (P4, local only).
pub(super) async fn room_agent_settings_get(
    State(state): State<AppState>,
    Path((key, agent_id)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = RoomKey::new(key.trim());
    if let Err(resp) = room_agent_or_error(&state, &key, &agent_id) {
        return resp;
    }
    match with_rooms(&state, |store| store.room_agent_settings(&key, &agent_id)) {
        Ok(settings) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "settings": settings })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

/// `PUT /v1/rooms/persistent/{key}/agents/{agent_id}/settings` (P4, local
/// only). Replaces the agent's overrides for this room; empty clears them.
pub(super) async fn room_agent_settings_put(
    State(state): State<AppState>,
    Path((key, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<RoomAgentSettings>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = room_agent_authority::operator(&state, &headers) {
        return error.response();
    }

    let Ok(Json(settings)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid_request" })),
        );
    };
    let settings = match normalize_agent_settings(settings) {
        Ok(s) => s,
        Err(code) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": code })),
            )
        }
    };
    let key = RoomKey::new(key.trim());
    if let Err(resp) = room_agent_or_error(&state, &key, &agent_id) {
        return resp;
    }
    match with_rooms(&state, |store| {
        store.put_room_agent_settings(&key, &agent_id, &settings)
    }) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "settings": settings })),
        ),
        Err(e) => room_store_error_response(e),
    }
}

// ── S2-P1 access projection SSE tail ─────────────────────────────────────────

/// Run an access-projection tail: on every wake hint (or lag), re-read the
/// durable access projection and send it downstream if changed. Selects
/// `tx.closed()` while idle so client disconnect cleans up.
async fn run_room_access_tail(
    state: AppState,
    room: RoomKey,
    mut last_access: Option<RoomAccessProjection>,
    mut hints: broadcast::Receiver<RoomAccessWakeHint>,
    tx: mpsc::Sender<RoomAccessProjection>,
) {
    loop {
        let should_read = tokio::select! {
            _ = tx.closed() => return,
            res = hints.recv() => match res {
                Ok(hint) => hint.room == room,
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        };
        if !with_rooms(&state, |store| store.get(&room).ok().flatten().is_some()) {
            return;
        }
        if !should_read {
            continue;
        }
        let proj = match with_rooms(&state, |store| store.room_access(&room)) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(room = %room, %e, "room access tail read failed");
                return;
            }
        };
        if last_access.as_ref() != Some(&proj) {
            last_access = Some(proj.clone());
            if tx.send(proj).await.is_err() {
                return;
            }
        }
    }
}

/// Read-cursor tail: on every wake hint (or lag), re-derive the wire-safe
/// `RoomReadCursorBody` and send it downstream if changed. `read_seq` is
/// stringified here (not left as a raw `u64` on the wire) so this event uses
/// the exact same JS-number-precision-safe schema as the REST
/// `GET`/`PATCH .../read-cursor` handlers (`RoomReadCursorBody`) — one shape
/// for both Local and Live rooms, chosen by which store field is
/// authoritative for the current access state (F3).
///
/// Local and Live are the only access states with a defined, trustworthy
/// read-cursor projection (this matches `room_read_cursor_unsupported_response`
/// in the REST handlers, which reject Connecting/Recovering/Revoked the same
/// way). For those three transitional/dead states this tail must NOT fall
/// back to reading under `local_room_read_cursor_principal()` — that principal
/// was never written to for a federated room, so doing so would silently
/// replace a real federated cursor value with an empty one and flicker the
/// client to a cleared "local" cursor on every Connecting/Recovering/Revoked
/// hop. Instead it skips the emission entirely, leaving `last_cursor`
/// (and the client's last-rendered projection) untouched. The federated
/// credential row is never touched by this skip, so the moment the room
/// returns to Live the credential-scoped principal resolves exactly as
/// before and the tail resumes emitting from where it left off (F1).
///
/// Also selects on `access_hints`: the upstream mirror does not necessarily
/// re-emit a `room_read_cursor` federation frame at the exact moment access
/// flips Connecting/Recovering -> Live (that mirror value may already be
/// durable from before the reconnect), so an access wake alone must be
/// enough to re-derive and emit the current cursor for a connection that was
/// opened mid-transition and has been sitting on a stale/absent projection.
async fn run_room_read_cursor_tail(
    state: AppState,
    room: RoomKey,
    mut last_cursor: Option<RoomReadCursorBody>,
    mut hints: broadcast::Receiver<RoomReadCursorWakeHint>,
    mut access_hints: broadcast::Receiver<RoomAccessWakeHint>,
    tx: mpsc::Sender<RoomReadCursorBody>,
) {
    loop {
        let should_read = tokio::select! {
            _ = tx.closed() => return,
            res = hints.recv() => match res {
                Ok(hint) => hint.room == room,
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
            res = access_hints.recv() => match res {
                Ok(hint) => hint.room == room,
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        };
        if !with_rooms(&state, |store| store.get(&room).ok().flatten().is_some()) {
            return;
        }
        if !should_read {
            continue;
        }
        let read_seq = match with_rooms(&state, |store| -> Result<_, ocean_store::RoomStoreError> {
            let access = store.room_access(&room)?;
            match access.state {
                RoomAccessState::Local => Ok(Some(
                    store
                        .room_read_cursor(&room, local_room_read_cursor_principal())?
                        .read_seq,
                )),
                RoomAccessState::Live => match live_room_read_cursor_principal(store, &room)? {
                    Some(principal) => Ok(Some(
                        store
                            .room_read_cursor(&room, &principal)?
                            .mirrored_upstream_read_seq,
                    )),
                    None => Ok(Some(None)),
                },
                // Read-cursor is unsupported while the federated link is not
                // confirmed Live — skip the emission (F1) rather than
                // resolving a principal at all.
                RoomAccessState::Connecting
                | RoomAccessState::Recovering
                | RoomAccessState::Revoked => Ok(None),
            }
        }) {
            Ok(read_seq) => read_seq,
            Err(e) => {
                tracing::warn!(room = %room, %e, "room read cursor tail read failed");
                return;
            }
        };
        let Some(read_seq) = read_seq else {
            continue;
        };
        let cursor = RoomReadCursorBody {
            room_id: room.as_str().to_string(),
            read_seq: read_seq.map(|seq| seq.to_string()),
        };
        if last_cursor.as_ref() != Some(&cursor) {
            last_cursor = Some(cursor.clone());
            if tx.send(cursor).await.is_err() {
                return;
            }
        }
    }
}

// ── S2-P1 outbox retry endpoint ─────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetryOutboxRequest {
    /// Outbox item to retry, identified by client_event_id. Must be non-empty;
    /// empty string is rejected at the handler level.
    pub(super) client_event_id: String,
}

/// Map a `RetryOutboxError` to an HTTP status + typed JSON body.
fn retry_outbox_error_response(
    err: ocean_store::RetryOutboxError,
) -> (StatusCode, Json<serde_json::Value>) {
    use ocean_store::RetryOutboxError::*;
    let (status, code) = match &err {
        RoomNotFound(_) => (StatusCode::NOT_FOUND, "room_not_found"),
        RoomNotFederated(_) => (StatusCode::CONFLICT, "room_not_federated"),
        RoomAccessRevoked(_) => (StatusCode::FORBIDDEN, "room_access_revoked"),
        OutboxItemNotFound { .. } => (StatusCode::NOT_FOUND, "outbox_item_not_found"),
        OutboxItemNotFailed { .. } => (StatusCode::CONFLICT, "outbox_item_not_failed"),
        Store(se) => match se {
            ocean_store::RoomStoreError::BadKey(_) => (StatusCode::BAD_REQUEST, "bad_key"),
            ocean_store::RoomStoreError::UnknownRoom(_) => {
                (StatusCode::NOT_FOUND, "room_not_found")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        },
    };
    let body = match &err {
        Store(_) => json!({ "ok": false, "code": code, "error": "internal store error" }),
        _ => json!({ "ok": false, "code": code, "error": err.to_string() }),
    };
    (status, Json(body))
}

/// `POST /v1/rooms/persistent/{key}/outbox/retry` — retry a failed outbox item.
///
/// Returns `202 Accepted` with the updated access projection on success.
/// Mapping: 202 (retried), 400 (bad key / malformed body / empty id), 403 (revoked),
/// 404 (room or item not found), 409 (not federated / item not in Failed state), 500 (store).
pub(super) async fn room_retry_outbox(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    let req: RetryOutboxRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "ok": false, "code": "invalid_retry_request", "error": e.to_string() }),
                ),
            );
        }
    };
    // Reject whitespace-only ids, but pass the original nonempty opaque id.
    let trimmed_id = req.client_event_id.trim();
    if trimmed_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "ok": false, "code": "invalid_retry_request", "error": "client_event_id must be non-empty" }),
            ),
        );
    }
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "invalid room key; must be non-empty" })),
        );
    }
    let key = RoomKey::new(trimmed);
    let result = with_rooms(&state, |store| {
        store.retry_failed_outbox(&key, &req.client_event_id)
    });
    match result {
        Ok(proj) => {
            publish_room_access_wake(&state, &key);
            (
                StatusCode::ACCEPTED,
                Json(json!({ "ok": true, "access": proj })),
            )
        }
        Err(e) => retry_outbox_error_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{
        authorize_room_agent_fixture, authorize_room_agent_package_fixture,
        canonical_test_workspace, fake_convene_state, write_agent_fixture, TestEnvRestore,
        AUTO_CONVENE_ENV_LOCK,
    };
    use axum::{
        body::{Body, Bytes},
        response::IntoResponse,
    };
    use ocean_store::ActivationPolicy;
    use serde_json::Value;
    use tower::ServiceExt;

    #[tokio::test]
    async fn room_create_holds_unwired_flags_without_touching_legacy_policy() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&["OCEAN_CONFIG_DIR", "OCEAN_MODEL", "OCEAN_YOLO"]);
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let legacy = RoomKey::new("legacy-policy");
        let legacy_policy = RoomTriggerPolicy {
            on_build_failure: true,
            on_ci_failure: true,
            ..Default::default()
        };
        with_rooms(&state, |store| {
            store.create(
                legacy.clone(),
                "Legacy",
                Some(legacy_policy.clone()),
                Utc::now(),
            )
        })
        .unwrap();
        let before = with_rooms(&state, |store| store.get(&legacy))
            .unwrap()
            .unwrap();
        for (key, policy) in [
            (
                "ci-policy",
                RoomTriggerPolicy {
                    on_ci_failure: true,
                    ..Default::default()
                },
            ),
            ("legacy-policy", legacy_policy.clone()),
        ] {
            let (status, body) = room_create(
                State(state.clone()),
                Json(RoomCreateRequest {
                    key: key.into(),
                    name: "New".into(),
                    trigger_policy: Some(policy),
                    workspace_root: None,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body.0["code"], "trigger_policy_unwired");
            if key != legacy.as_str() {
                assert!(with_rooms(&state, |store| store.get(&RoomKey::new(key)))
                    .unwrap()
                    .is_none());
            }
        }
        assert_eq!(
            with_rooms(&state, |store| store.get(&legacy))
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(
            with_rooms(&state, |store| store.trigger_policy(&legacy)).unwrap(),
            Some(legacy_policy)
        );
        let (status, _) = room_create(
            State(state.clone()),
            Json(RoomCreateRequest {
                key: "build-hint-policy".into(),
                name: "Build hint".into(),
                trigger_policy: Some(RoomTriggerPolicy {
                    on_build_failure: true,
                    ..Default::default()
                }),
                workspace_root: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(
            with_rooms(&state, |store| store
                .trigger_policy(&RoomKey::new("build-hint-policy")))
            .unwrap()
            .unwrap()
            .on_build_failure
        );
        let (status, _) = room_create(
            State(state),
            Json(RoomCreateRequest {
                key: "ordinary-policy".into(),
                name: "Ordinary".into(),
                trigger_policy: Some(RoomTriggerPolicy {
                    on_mention: true,
                    ..Default::default()
                }),
                workspace_root: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    #[derive(Clone)]
    struct RouteBedrock {
        roster: Arc<tokio::sync::Mutex<serde_json::Value>>,
    }

    impl RouteBedrock {
        fn new() -> Self {
            Self {
                roster: Arc::new(tokio::sync::Mutex::new(json!({"members":[]}))),
            }
        }
    }

    async fn route_control_register(Path(room): Path<String>) -> axum::response::Response {
        (
            StatusCode::CREATED,
            Json(json!({
                "room_id":room,
                "owner":{
                    "member_id":"11111111-1111-4111-8111-111111111111",
                    "actor_type":"user",
                    "role_in_room":"owner",
                    "display_name":"Owner"
                }
            })),
        )
            .into_response()
    }

    async fn route_control_invite(Json(body): Json<serde_json::Value>) -> axum::response::Response {
        let room = body["room_id"].as_str().unwrap();
        (
            StatusCode::CREATED,
            Json(json!({
                "code":"route-share-code",
                "invite":{
                    "role":"contributor",
                    "scopes":[format!("/rooms/{room}")],
                    "expiresAt":"2026-07-18T00:00:00Z"
                }
            })),
        )
            .into_response()
    }

    async fn route_control_redeem() -> axum::response::Response {
        (
            StatusCode::CREATED,
            Json(json!({
                "invite":{
                    "role":"contributor",
                    "scopes":["/rooms/route-redeem"],
                    "expiresAt":"2026-07-18T00:00:00Z"
                },
                "record":{
                    "role":"contributor",
                    "scopes":["/rooms/route-redeem"]
                }
            })),
        )
            .into_response()
    }

    async fn route_control_self_join(
        Path(_room): Path<String>,
        _body: Bytes,
    ) -> axum::response::Response {
        (
            StatusCode::CREATED,
            Json(json!({
                "member":{
                    "member_id":"22222222-2222-4222-8222-222222222222",
                    "actor_type":"user",
                    "role_in_room":"member",
                    "display_name":"Joined Human"
                }
            })),
        )
            .into_response()
    }

    async fn route_control_agents(
        State(fake): State<RouteBedrock>,
        Path(_room): Path<String>,
        Json(body): Json<serde_json::Value>,
    ) -> axum::response::Response {
        let requested = body["agents"].as_array().unwrap();
        let members: Vec<_> = requested
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                json!({
                    "member_id":format!("33333333-3333-4333-8333-{index:012}"),
                    "owner_member_id":"22222222-2222-4222-8222-222222222222",
                    "actor_type":"agent",
                    "role_in_room":"member",
                    "display_name":agent["display_name"],
                    "public_agent_descriptor":{
                        "display_name":agent["display_name"],
                        "skills_count":agent["skills_count"],
                        "subagent_names":agent["subagent_names"]
                    },
                    "joined_at":"2026-07-17T00:00:00Z"
                })
            })
            .collect();
        let mut roster = vec![json!({
            "member_id":"22222222-2222-4222-8222-222222222222",
            "actor_type":"user",
            "role_in_room":"member",
            "display_name":"Joined Human",
            "joined_at":"2026-07-17T00:00:00Z"
        })];
        roster.extend(members.iter().cloned());
        *fake.roster.lock().await = json!({"members":roster});
        (StatusCode::CREATED, Json(json!({"members":members}))).into_response()
    }

    async fn route_control_members(
        State(fake): State<RouteBedrock>,
        Path(_room): Path<String>,
    ) -> axum::response::Response {
        Json(fake.roster.lock().await.clone()).into_response()
    }

    async fn route_room_read_cursor_unauthorized() -> axum::response::Response {
        StatusCode::UNAUTHORIZED.into_response()
    }

    async fn start_route_read_cursor_unauthorized_bedrock() -> (String, tokio::task::JoinHandle<()>)
    {
        let app = axum::Router::new()
            .route(
                "/api/v1/rooms/{room}/read-cursor",
                axum::routing::get(route_room_read_cursor_unauthorized),
            )
            .with_state(RouteBedrock::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{address}"), server)
    }

    async fn route_room_read_cursor_ok(Path(room): Path<String>) -> axum::response::Response {
        Json(json!({"room_id": room, "sequence": "77", "clamped": false})).into_response()
    }

    /// M6 regression fixture: an upstream that truthfully answers GET
    /// `.../read-cursor` so the Live-room HTTP handler round trip can be
    /// exercised end to end and checked against the SAME response schema
    /// the Local-room path produces.
    async fn start_route_read_cursor_ok_bedrock() -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new()
            .route(
                "/api/v1/rooms/{room}/read-cursor",
                axum::routing::get(route_room_read_cursor_ok),
            )
            .with_state(RouteBedrock::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{address}"), server)
    }

    async fn start_route_bedrock() -> (String, tokio::task::JoinHandle<()>) {
        let fake = RouteBedrock::new();
        let app = axum::Router::new()
            .route(
                "/api/v1/rooms/{room}/register",
                axum::routing::post(route_control_register),
            )
            .route("/api/v1/invites", axum::routing::post(route_control_invite))
            .route(
                "/api/v1/invites/redeem",
                axum::routing::post(route_control_redeem),
            )
            .route(
                "/api/v1/rooms/{room}/members/self",
                axum::routing::post(route_control_self_join),
            )
            .route(
                "/api/v1/rooms/{room}/members/agents",
                axum::routing::post(route_control_agents),
            )
            .route(
                "/api/v1/rooms/{room}/members",
                axum::routing::get(route_control_members),
            )
            .with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{address}"), server)
    }

    fn with_route_supervisor(mut state: AppState, base: &str) -> AppState {
        state.room_federation = crate::room_federation::FederationSupervisor::for_test(
            base,
            state.rooms.clone(),
            state.room_wakes.clone(),
            state.room_access_wakes.clone(),
            state.room_read_cursor_wakes.clone(),
            state.shutdown.clone(),
            std::time::Duration::from_secs(60),
        );
        state
    }

    fn dispatch_agent_projection(
        member_id: &str,
        owner_member_id: &str,
        display_name: &str,
    ) -> ocean_core::FederatedRoomMemberProjection {
        ocean_core::FederatedRoomMemberProjection {
            member_id: member_id.into(),
            owner_member_id: Some(owner_member_id.into()),
            actor_type: ocean_core::FederatedActorType::Agent,
            role_in_room: ocean_core::FederatedRoomRole::Member,
            display_name: display_name.into(),
            public_agent_descriptor: Some(PublicAgentDescriptor {
                display_name: display_name.into(),
                description: None,
                model_alias: None,
                skills_count: 0,
                subagent_names: vec![],
            }),
            joined_at: "2026-07-17T00:00:00Z".into(),
            derived_presence: Some(ocean_core::MemberPresence::Live),
            local_binding_available: Some(true),
        }
    }

    fn dispatch_human_projection(member_id: &str) -> ocean_core::FederatedRoomMemberProjection {
        ocean_core::FederatedRoomMemberProjection {
            member_id: member_id.into(),
            owner_member_id: None,
            actor_type: ocean_core::FederatedActorType::User,
            role_in_room: ocean_core::FederatedRoomRole::Member,
            display_name: "Reclassified Human".into(),
            public_agent_descriptor: None,
            joined_at: "2026-07-17T00:00:00Z".into(),
            derived_presence: Some(ocean_core::MemberPresence::Unavailable),
            local_binding_available: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rooms_list_persistent_includes_ordered_read_states_for_local_and_live() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let local = RoomKey::new("list-read-local");
        let live = RoomKey::new("list-read-live");
        with_rooms(&state, |store| {
            store.create(local.clone(), "Local", None, Utc::now())?;
            store.append_message(
                &local,
                "u1",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "local first",
                Utc::now(),
            )?;
            store.update_room_read_cursor(
                &local,
                local_room_read_cursor_principal(),
                RoomReadCursorUpdateRequest { read_seq: 0 },
            )?;

            store.create(live.clone(), "Live", None, Utc::now())?;
            store.update_room_access_safe(
                &live,
                Some(RoomAccessState::Live),
                None,
                Some(u64::MAX),
            )?;
            store.install_room_credential(&live, "bearer-secret", "live-principal")?;
            store.set_room_read_cursor_mirror(
                &live,
                "live-principal",
                None,
                Some((1u64 << 53) + 7),
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, body) = rooms_list_persistent(
            State(state.clone()),
            Query(RoomsListQuery {
                limit: Some(10),
                cursor: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["ok"], json!(true));
        let rooms = body.0["rooms"].as_array().unwrap();
        let read_states = body.0["read_states"].as_array().unwrap();
        assert_eq!(rooms.len(), 2);
        assert_eq!(read_states.len(), 2);
        for (room, read_state) in rooms.iter().zip(read_states.iter()) {
            assert_eq!(read_state["room_id"], room["id"]);
        }
        assert_eq!(
            read_states[0],
            json!({
                "room_id": live.as_str(),
                "latest_seq": u64::MAX.to_string(),
                "read_seq": ((1u64 << 53) + 7).to_string()
            })
        );
        assert_eq!(
            read_states[1],
            json!({
                "room_id": local.as_str(),
                "latest_seq": "0",
                "read_seq": "0"
            })
        );
        let encoded = serde_json::to_string(&body.0).unwrap();
        assert!(encoded.contains(&format!("\"latest_seq\":\"{}\"", u64::MAX)));
        assert!(encoded.contains("\"read_seq\":\"9007199254740999\""));
        assert!(!encoded.contains("bearer-secret"));
        assert!(!encoded.contains("live-principal"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rooms_list_persistent_uses_durable_metadata_for_non_live_federated_states() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("list-read-connecting");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Connecting", None, Utc::now())?;
            store.update_room_access_safe(
                &key,
                Some(RoomAccessState::Connecting),
                None,
                Some(42),
            )?;
            store.install_room_credential(&key, "bearer-secret", "connecting-principal")?;
            store.set_room_read_cursor_mirror(&key, "connecting-principal", None, Some(7))?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, body) = rooms_list_persistent(
            State(state),
            Query(RoomsListQuery {
                limit: Some(10),
                cursor: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0["read_states"],
            json!([{
                "room_id": key.as_str(),
                "latest_seq": "42",
                "read_seq": "7"
            }])
        );
        let encoded = serde_json::to_string(&body.0).unwrap();
        assert!(!encoded.contains("bearer-secret"));
        assert!(!encoded.contains("connecting-principal"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_handlers_truthfully_distinguish_absent_zero_and_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-handler-local");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Cursor Handler Local", None, Utc::now())?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, body) =
            room_get_read_cursor(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0,
            json!({"ok": true, "cursor": {"room_id": "cursor-handler-local", "read_seq": null}})
        );

        with_rooms(&state, |store| {
            store.append_message(
                &key,
                "u1",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "first",
                Utc::now(),
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, body) = room_patch_read_cursor(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Bytes::from_static(br#"{"read_seq":0}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0,
            json!({"ok": true, "cursor": {"room_id": "cursor-handler-local", "read_seq": "0"}})
        );

        let (status, body) =
            room_get_read_cursor(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0,
            json!({"ok": true, "cursor": {"room_id": "cursor-handler-local", "read_seq": "0"}})
        );

        let (status, body) = room_patch_read_cursor(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Bytes::from_static(br#"{"read_seq":0,"extra":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.0["ok"], json!(false));

        let federated = RoomKey::new("cursor-handler-live");
        with_rooms(&state, |store| {
            store.create(federated.clone(), "Cursor Handler Live", None, Utc::now())?;
            store.update_room_access_safe(&federated, Some(RoomAccessState::Live), None, None)?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, body) =
            room_get_read_cursor(State(state.clone()), Path(federated.as_str().to_string())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.0["code"], json!("room_read_cursor_unsupported"));

        let (status, body) = room_patch_read_cursor(
            State(state),
            Path(federated.as_str().to_string()),
            Bytes::from_static(br#"{"read_seq":0}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.0["code"], json!("room_read_cursor_unsupported"));
    }

    /// M6 + H1 regression: the Live-room `GET .../read-cursor` response uses
    /// the SAME `{room_id, read_seq}` schema as the Local-room response
    /// (previously it returned `{room_id, sequence}`, a different shape),
    /// and the value it reports is read back from the SAME store principal
    /// the SSE read-cursor tail resolves for that room — the per-credential
    /// `local_human_member_id`, not a fixed placeholder string that nothing
    /// ever writes to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_live_get_matches_local_schema_and_sse_principal() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-handler-live-ok");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Cursor Handler Live Ok", None, Utc::now())?;
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, None)?;
            store.install_room_credential(&key, "bearer", "member-live-ok")?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (base, server) = start_route_read_cursor_ok_bedrock().await;
        state = with_route_supervisor(state, &base);

        let (status, body) =
            room_get_read_cursor(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0,
            json!({"ok": true, "cursor": {"room_id": key.as_str(), "read_seq": "77"}})
        );

        // The federation client durably mirrors what the upstream reported
        // (77) keyed by the credential's `local_human_member_id`.
        let cursor = with_rooms(&state, |store| {
            store.room_read_cursor(&key, "member-live-ok")
        })
        .unwrap();
        assert_eq!(cursor.mirrored_upstream_read_seq, Some(77));

        // H1: both the initial SSE snapshot logic and the read-cursor tail
        // must resolve that exact same principal, or they would silently
        // observe an always-empty cursor for this (and every) Live room.
        let resolved = with_rooms(&state, |store| {
            let access = store.room_access(&key)?;
            assert_eq!(access.state, RoomAccessState::Live);
            match live_room_read_cursor_principal(store, &key)? {
                Some(principal) => store.room_read_cursor(&key, &principal),
                None => panic!("expected a room credential principal for a Live room"),
            }
        })
        .unwrap();
        assert_eq!(resolved.mirrored_upstream_read_seq, Some(77));

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let hints = state.room_read_cursor_wakes.subscribe();
        let access_hints = state.room_access_wakes.test_subscribe();
        tokio::spawn(run_room_read_cursor_tail(
            state.clone(),
            key.clone(),
            Some(RoomReadCursorBody {
                room_id: key.as_str().to_string(),
                read_seq: None,
            }),
            hints,
            access_hints,
            tx,
        ));
        publish_room_read_cursor_wake(&state, &key);
        let tailed = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tailed.room_id, key.as_str());
        assert_eq!(tailed.read_seq, Some("77".to_string()));

        server.abort();
        state.room_federation.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_live_get_truthfully_reports_revoked_without_mutating_mirror() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-handler-live-revoked");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Cursor Handler Live Revoked", None, Utc::now())?;
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, None)?;
            store.install_room_credential(&key, "bearer", "principal")?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (base, server) = start_route_read_cursor_unauthorized_bedrock().await;
        state = with_route_supervisor(state, &base);

        let (status, body) =
            room_get_read_cursor(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.0, json!({"ok": false, "error": "membership_revoked"}));

        let cursor = with_rooms(&state, |store| store.room_read_cursor(&key, "principal")).unwrap();
        assert_eq!(cursor.read_seq, None);
        assert_eq!(cursor.mirrored_upstream_read_seq, None);

        server.abort();
        state.room_federation.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_tail_emits_absent_then_zero_on_wake() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-tail-local");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Cursor Tail Local", None, Utc::now())?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let hints = state.room_read_cursor_wakes.subscribe();
        let access_hints = state.room_access_wakes.test_subscribe();
        tokio::spawn(run_room_read_cursor_tail(
            state.clone(),
            key.clone(),
            Some(RoomReadCursorBody {
                room_id: key.as_str().to_string(),
                read_seq: None,
            }),
            hints,
            access_hints,
            tx,
        ));

        with_rooms(&state, |store| {
            store.append_message(
                &key,
                "u1",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "first",
                Utc::now(),
            )?;
            store.update_room_read_cursor(
                &key,
                local_room_read_cursor_principal(),
                RoomReadCursorUpdateRequest { read_seq: 0 },
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        publish_room_read_cursor_wake(&state, &key);

        let cursor = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cursor.room_id, key.as_str());
        assert_eq!(cursor.read_seq, Some("0".to_string()));
    }

    /// F1 regression: while the federated link is not confirmed Live
    /// (Connecting/Recovering/Revoked), the read-cursor tail must skip
    /// emissions entirely rather than falling back to
    /// `local_room_read_cursor_principal()` — that principal is never
    /// written to for a federated room, so reading it would flicker the
    /// client from the last real federated cursor value to a fabricated
    /// cleared/local one on every transient hop. The federated credential
    /// principal must also still resolve correctly once the room returns to
    /// Live, proving it was never disturbed by the skip.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_tail_retains_federated_principal_and_skips_unsupported_transitions() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-tail-transition");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Cursor Tail Transition", None, Utc::now())?;
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, Some(1))?;
            store.install_room_credential(&key, "bearer-secret", "federated-principal")?;
            store.set_room_read_cursor_mirror(&key, "federated-principal", None, Some(5))?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let hints = state.room_read_cursor_wakes.subscribe();
        let access_hints = state.room_access_wakes.test_subscribe();
        tokio::spawn(run_room_read_cursor_tail(
            state.clone(),
            key.clone(),
            None,
            hints,
            access_hints,
            tx,
        ));

        publish_room_read_cursor_wake(&state, &key);
        let live = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live.read_seq, Some("5".to_string()));

        // Connecting: read-cursor unsupported. Must skip, not flicker.
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Connecting), None, None)
        })
        .unwrap();
        publish_room_read_cursor_wake(&state, &key);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "expected no room_read_cursor emission while access is Connecting"
        );

        // Revoked: same — still skip, credential row untouched.
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Revoked), None, None)
        })
        .unwrap();
        publish_room_read_cursor_wake(&state, &key);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "expected no room_read_cursor emission while access is Revoked"
        );

        // Back to Live: the federated credential principal must still
        // resolve correctly (never cleared to the local principal), so the
        // mirrored value is read again without any manual re-installation.
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, Some(2))?;
            store.set_room_read_cursor_mirror(&key, "federated-principal", Some(5), Some(9))
        })
        .unwrap();
        publish_room_read_cursor_wake(&state, &key);
        let resumed = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.read_seq, Some("9".to_string()));
    }

    /// PR #366 review comment 3727657446 regression: `/events` opened while a
    /// federated room is `Connecting` must not drop the cursor tail. The
    /// handler used to gate spawning `run_room_read_cursor_tail` on
    /// `read_cursor_supported`, so a connection opened mid-Connecting never
    /// started a tail at all and would never observe a `room_read_cursor`
    /// frame later, even after access became `Live`, without the client
    /// reconnecting. Proves over the real HTTP SSE handler, on the same
    /// still-open connection:
    /// - the initial snapshot is `room_access: Connecting` with no
    ///   `room_read_cursor` bootstrap frame (projection undefined);
    /// - no `room_read_cursor` frame arrives while still Connecting;
    /// - once access flips to Live — mirroring
    ///   `room_federation::commit_access`, which publishes only an access
    ///   wake, not a read-cursor wake — the same connection still emits the
    ///   current cursor, proving the tail stayed subscribed through the
    ///   transition and reacted to the access wake alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_events_http_cursor_tail_survives_connecting_and_emits_on_live_without_reconnect()
    {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-transition-live");
        create_plain_room(&state, &key);
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Connecting), None, None)?;
            store.install_room_credential(&key, "bearer-secret", "transition-principal")?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let app = super::super::room_routes().with_state(state.clone());
        let request = axum::http::Request::builder()
            .uri(format!("/v1/rooms/persistent/{key}/events"))
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();

        // Only one frame up front: the initial room_access snapshot
        // (Connecting). No room_read_cursor bootstrap frame — the projection
        // is undefined while access is unsupported.
        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), body.frame())
            .await
            .expect("frame timeout")
            .expect("SSE body ended")
            .expect("SSE body error");
        let text = String::from_utf8_lossy(&frame.into_data().unwrap_or_default()).to_string();
        assert!(
            text.contains("event: room_access\n"),
            "expected initial room_access frame, got: {text:?}"
        );
        assert!(
            text.contains("\"state\":\"connecting\""),
            "expected Connecting access snapshot, got: {text:?}"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), body.frame())
                .await
                .is_err(),
            "expected no room_read_cursor frame while access is Connecting"
        );

        // Same still-open connection observes a reconnect completion: access
        // flips straight to Live and the federated mirror already carries a
        // durable cursor value from before the reconnect (no fresh upstream
        // `room_read_cursor` federation frame arrives).
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, None)?;
            store.set_room_read_cursor_mirror(&key, "transition-principal", None, Some(42))?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        publish_room_access_wake(&state, &key);

        let mut saw_access_live = false;
        let mut saw_cursor = false;
        for _ in 0..4 {
            if saw_access_live && saw_cursor {
                break;
            }
            let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
                .await
                .expect("frame timeout")
                .expect("SSE body ended")
                .expect("SSE body error");
            let text = String::from_utf8_lossy(&frame.into_data().unwrap_or_default()).to_string();
            if text.contains("event: room_access\n") && text.contains("\"state\":\"live\"") {
                saw_access_live = true;
            }
            if text.contains("event: room_read_cursor\n") {
                saw_cursor = true;
                let data = text
                    .lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .expect("data line");
                let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
                assert_eq!(parsed, json!({ "room_id": key.as_str(), "read_seq": "42" }));
            }
        }
        assert!(saw_access_live, "expected a Live room_access frame");
        assert!(
            saw_cursor,
            "expected the still-open connection to emit the current room_read_cursor without reconnecting"
        );
    }

    /// F3 regression: the SSE `room_read_cursor` wire uses the same
    /// JS-number-precision-safe decimal-string schema as REST
    /// (`RoomReadCursorBody`) — a `read_seq` above 2^53 must serialize as a
    /// quoted string, never a bare JS-unsafe number, for both the initial
    /// bootstrap frame and subsequent tail emissions.
    ///
    /// Uses a Live room's federated mirror (`set_room_read_cursor_mirror`)
    /// rather than the Local `room_read_cursors` path: `update_room_read_cursor`
    /// clamps the requested value to the room's message high-water seq, so a
    /// fixture with no messages could never durably persist a raw >2^53
    /// value there. The federated mirror has no such clamp (it stores
    /// whatever the upstream reports), matching how
    /// `rooms_list_persistent_includes_ordered_read_states_for_local_and_live`
    /// already proves this exact value round-trips through the REST list
    /// endpoint.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_read_cursor_sse_wire_uses_js_safe_decimal_strings_above_2_53() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("cursor-precision-live");
        create_plain_room(&state, &key);
        let huge = (1u64 << 53) + 11;
        with_rooms(&state, |store| {
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, None)?;
            store.install_room_credential(&key, "bearer-secret", "huge-principal")?;
            store.set_room_read_cursor_mirror(&key, "huge-principal", None, Some(huge))?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let app = super::super::room_routes().with_state(state);
        let request = axum::http::Request::builder()
            .uri(format!("/v1/rooms/persistent/{key}/events"))
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();

        // Only two frames expected up front (no messages, no access churn):
        // the S2-P1 initial room_access frame and the read-cursor bootstrap.
        // Order between them is not contractually fixed (they're merged
        // concurrently), so read both and match by event name.
        let frame_a = tokio::time::timeout(std::time::Duration::from_millis(500), body.frame())
            .await
            .expect("frame a timeout")
            .expect("SSE body ended")
            .expect("SSE body error");
        let text_a = String::from_utf8_lossy(&frame_a.into_data().unwrap_or_default()).to_string();
        let frame_b = tokio::time::timeout(std::time::Duration::from_millis(500), body.frame())
            .await
            .expect("frame b timeout")
            .expect("SSE body ended")
            .expect("SSE body error");
        let text_b = String::from_utf8_lossy(&frame_b.into_data().unwrap_or_default()).to_string();

        let cursor_wire = if text_a.contains("event: room_read_cursor\n") {
            text_a
        } else {
            assert!(
                text_b.contains("event: room_read_cursor\n"),
                "expected a room_read_cursor bootstrap frame, got: {text_a:?} / {text_b:?}"
            );
            text_b
        };
        let data = cursor_wire
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("data line");
        // Wire-level proof: the huge value must appear as a quoted decimal
        // string, never a bare JSON number (which would silently lose
        // precision in JS's IEEE-754 f64 doubles above 2^53).
        assert!(
            data.contains(&format!("\"read_seq\":\"{huge}\"")),
            "expected JS-safe quoted decimal string in: {data:?}"
        );
        assert!(
            !data.contains(&format!("\"read_seq\":{huge}")),
            "read_seq must never be a bare unsafe number: {data:?}"
        );
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(
            parsed,
            json!({ "room_id": key.as_str(), "read_seq": huge.to_string() })
        );
    }

    #[test]
    fn g3_author_classification_is_exact_and_fail_closed() {
        let roster = vec![
            RoomParticipant {
                id: "john".into(),
                kind: RoomParticipantKind::Human,
                display_name: "John".into(),
            },
            RoomParticipant {
                id: "helper".into(),
                kind: RoomParticipantKind::Agent,
                display_name: "Helper".into(),
            },
        ];

        assert_eq!(
            classify_local_author(&roster, "john", RoomParticipantKind::Human),
            Ok("john")
        );
        assert_eq!(
            classify_local_author(&roster, " john ", RoomParticipantKind::Human),
            Err(PostRejection::AuthorNotInRoster)
        );
        assert_eq!(
            classify_local_author(&roster, "unknown", RoomParticipantKind::Human),
            Err(PostRejection::AuthorNotInRoster)
        );
        assert_eq!(
            classify_local_author(&roster, "john", RoomParticipantKind::Bot),
            Err(PostRejection::AuthorNotInRoster)
        );
        assert_eq!(
            classify_local_author(&roster, "helper", RoomParticipantKind::Agent),
            Err(PostRejection::ForgedAuthorKind)
        );
        assert_eq!(
            classify_local_author(&roster, "system", RoomParticipantKind::System),
            Err(PostRejection::ForgedAuthorKind)
        );
    }

    #[test]
    fn g3_message_wire_rejects_client_session_attribution() {
        assert!(serde_json::from_value::<RoomMessageRequest>(json!({
            "author_id": "john",
            "author_kind": "human",
            "body": "hello",
            "session_id": "00000000-0000-0000-0000-000000000000"
        }))
        .is_err());
    }

    /// Seed the daemon owner so tests can name the human every local join and
    /// post is authored as (team-platform P2).
    pub(crate) fn seed_owner(state: &AppState, id: &str, display_name: &str) {
        with_rooms(state, |store| {
            store.replace_owner_identity(&ocean_store::OwnerIdentity {
                participant_id: id.into(),
                display_name: display_name.into(),
            })
        })
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2_human_join_and_post_are_authored_by_the_daemon_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        seed_owner(&state, "ada", "Ada");
        let key = RoomKey::new("p2-owner");
        with_rooms(&state, |store| {
            store.create(key.clone(), "P2", None, Utc::now())
        })
        .unwrap();

        // A post before the owner joined is refused and writes nothing.
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: String::new(),
                author_kind: RoomParticipantKind::Human,
                body: "too early".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.0,
            json!({"ok": false, "error": "author_not_in_roster"})
        );

        // A human join claiming another identity joins as the owner.
        let (status, body) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "mallory".into(),
                display_name: "Mallory".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let roster = body.0["room"]["participants"].as_array().unwrap().clone();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0]["id"], "ada");
        assert_eq!(roster[0]["display_name"], "Ada");

        // A post claiming another human id is authored by the owner.
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "mallory".into(),
                author_kind: RoomParticipantKind::Human,
                body: "hello".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body.0["message"]["author_id"], "ada");

        // Renaming the owner renames the roster row; the id is stable.
        let (status, body) = me_put(
            State(state.clone()),
            operator_headers(),
            Ok(Json(MeUpdateRequest {
                display_name: "Ada King".into(),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.0,
            json!({"participant_id": "ada", "display_name": "Ada King"})
        );
        let (_, body) = me_get(State(state.clone())).await;
        assert_eq!(body.0["display_name"], "Ada King");
        let roster = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .room
            .participants;
        assert_eq!(roster[0].display_name, "Ada King");
    }

    fn operator_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            crate::room_operator::OPERATOR_HEADER,
            "test-room-operator".parse().unwrap(),
        );
        headers
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2_owner_rename_requires_the_room_operator_before_the_body() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        seed_owner(&state, "ada", "Ada");
        let mut wrong = HeaderMap::new();
        wrong.insert(
            crate::room_operator::OPERATOR_HEADER,
            "wrong".parse().unwrap(),
        );
        let mut cookie = operator_headers();
        cookie.insert("cookie", "ambient=1".parse().unwrap());
        for (headers, expected) in [
            (HeaderMap::new(), StatusCode::SERVICE_UNAVAILABLE),
            (wrong, StatusCode::FORBIDDEN),
            (cookie, StatusCode::FORBIDDEN),
        ] {
            let (status, _) = me_put(
                State(state.clone()),
                headers,
                Ok(Json(MeUpdateRequest {
                    display_name: "Mallory".into(),
                })),
            )
            .await;
            assert_eq!(status, expected);
            let (_, body) = me_get(State(state.clone())).await;
            assert_eq!(body.0["display_name"], "Ada", "nothing renamed");
        }
        // Through the registered router as well: no header, no rename.
        let response = room_routes()
            .with_state(state.clone())
            .oneshot(
                axum::http::Request::put("/v1/me")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"display_name":"Mallory"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::OK);
        let (_, body) = me_get(State(state.clone())).await;
        assert_eq!(body.0["display_name"], "Ada");
        let (status, body) = me_put(
            State(state.clone()),
            operator_headers(),
            Ok(Json(MeUpdateRequest {
                display_name: "Ada King".into(),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["display_name"], "Ada King");
    }

    #[test]
    fn daemon_member_resolves_like_the_identity_route() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("member.toml");
        // Neither set: nobody, never the process user.
        assert_eq!(resolve_daemon_member(tmp.path(), None), None);
        // Env alone, trimmed and validated.
        assert_eq!(
            resolve_daemon_member(tmp.path(), Some("  jay  ")),
            Some(DaemonMember {
                member_id: "jay".into(),
                display_name: None
            })
        );
        assert_eq!(resolve_daemon_member(tmp.path(), Some("not an id")), None);
        // member.toml wins over the env and carries its display name.
        std::fs::write(
            &file,
            "# who this box is\nmember_id = \"smaths\"\ndisplay_name = \"John\"\n",
        )
        .unwrap();
        assert_eq!(
            resolve_daemon_member(tmp.path(), Some("jay")),
            Some(DaemonMember {
                member_id: "smaths".into(),
                display_name: Some("John".into())
            })
        );
        // Malformed, unknown-field, duplicate or nested files are absent.
        for bad in [
            "member_id = jay",
            "member_id = \"first\"\nmember_id = \"second\"",
            "[section]\nmember_id = \"nested\"",
            "member_id = \"jay\"\nextra = 1",
            "member_id = \"not a member id\"",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(
                resolve_daemon_member(tmp.path(), Some("fallback")).map(|m| m.member_id),
                Some("fallback".into()),
                "{bad:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2_owner_identity_is_the_configured_member_not_the_login_name() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        // Minted from the login name before member.toml existed.
        seed_owner(&state, "loginname", "loginname");
        std::fs::write(
            state.runtime.config_dir().join("member.toml"),
            "member_id = \"smaths\"\ndisplay_name = \"John\"\n",
        )
        .unwrap();
        let (status, body) = me_get(State(state.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["participant_id"], "smaths");
        assert_ne!(body.0["participant_id"], "loginname");

        assert_eq!(body.0["display_name"], "John");
        // Human posts are authored as that member id.
        let key = RoomKey::new("p2-member");
        with_rooms(&state, |store| {
            store.create(key.clone(), "P2", None, Utc::now())
        })
        .unwrap();
        let (status, body) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: String::new(),
                display_name: String::new(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["room"]["participants"][0]["id"], "smaths");
        assert_eq!(body.0["room"]["participants"][0]["display_name"], "John");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn g3_local_post_enforces_author_and_thread_authority_without_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        seed_owner(&state, "john", "John");
        let key = RoomKey::new("g3-authority");
        with_rooms(&state, |store| {
            store.create(key.clone(), "G3 Authority", None, Utc::now())?;
            store.add_participant(
                &key,
                RoomParticipant {
                    id: "john".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "John".into(),
                },
                Utc::now(),
            )?;
            store.add_participant(
                &key,
                RoomParticipant {
                    id: "helper".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Helper".into(),
                },
                Utc::now(),
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let initial_len = with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .len();
        // Client-claimed human ids are ignored (P2: humans are the daemon
        // owner), so only daemon-only kinds and off-roster bots are refusable.
        for (author_id, author_kind, expected_error) in [
            ("helper", RoomParticipantKind::Agent, "forged_author_kind"),
            ("system", RoomParticipantKind::System, "forged_author_kind"),
            ("unknown", RoomParticipantKind::Bot, "author_not_in_roster"),
            (" john ", RoomParticipantKind::Bot, "author_not_in_roster"),
        ] {
            let (status, body) = room_post_message(
                State(state.clone()),
                Path(key.as_str().to_string()),
                Json(RoomMessageRequest {
                    author_id: author_id.into(),
                    author_kind,
                    body: "must not persist".into(),
                    thread_parent_seq: None,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body.0, json!({"ok": false, "error": expected_error}));
            assert_eq!(
                with_rooms(&state, |store| store.transcript(&key, None))
                    .unwrap()
                    .len(),
                initial_len,
                "rejected author {author_id:?} must not write"
            );
        }

        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: " john ".into(),
                author_kind: RoomParticipantKind::Human,
                body: "valid post".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            body.0["message"]["author_id"], "john",
            "a non-canonical client-claimed human id is replaced by the owner id"
        );
        assert_eq!(body.0["message"]["session_id"], serde_json::Value::Null);

        let before_invalid_parent = with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .len();
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "john".into(),
                author_kind: RoomParticipantKind::Human,
                body: "orphan reply".into(),
                thread_parent_seq: Some(u64::MAX),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.0,
            json!({"ok": false, "error": "invalid_thread_parent"})
        );
        assert_eq!(
            with_rooms(&state, |store| store.transcript(&key, None))
                .unwrap()
                .len(),
            before_invalid_parent,
            "invalid thread parent must not write"
        );

        // One-level policy, exercised for real (not a nonexistent seq): a reply
        // to a REPLY row is exactly what live QA found silently accepted-or-lost.
        // Build root -> reply, then post against the reply and require the same
        // typed 400 with no write.
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "john".into(),
                author_kind: RoomParticipantKind::Human,
                body: "thread root".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let root_seq = body.0["message"]["seq"].as_u64().unwrap();
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "john".into(),
                author_kind: RoomParticipantKind::Human,
                body: "first reply".into(),
                thread_parent_seq: Some(root_seq),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let reply_seq = body.0["message"]["seq"].as_u64().unwrap();
        let before_nested = with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .len();
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "john".into(),
                author_kind: RoomParticipantKind::Human,
                body: "nested reply".into(),
                thread_parent_seq: Some(reply_seq),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.0,
            json!({"ok": false, "error": "invalid_thread_parent"})
        );
        assert_eq!(
            with_rooms(&state, |store| store.transcript(&key, None))
                .unwrap()
                .len(),
            before_nested,
            "reply-to-a-reply must not write"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn g3_thread_reply_deduplicates_dispatch_and_attributes_agent_reply() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "helper", "", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let (_replay, mut trigger_rx) = state.agent_events.subscribe_with_replay(None);

        let key = RoomKey::new("g3-thread-dispatch");
        with_rooms(&state, |store| {
            store.create_in_workspace(
                key.clone(),
                "G3 Thread Dispatch",
                Some(canonical_test_workspace(tmp.path())),
                Some(RoomTriggerPolicy {
                    on_mention: true,
                    on_thread_reply: true,
                    ..Default::default()
                }),
                Utc::now(),
            )?;
            store.add_participant(
                &key,
                RoomParticipant {
                    id: "john".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "John".into(),
                },
                Utc::now(),
            )?;
            store.add_agent_participant_with_owner(
                &key,
                RoomParticipant {
                    id: "helper".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Helper".into(),
                },
                "john",
                Utc::now(),
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "helper",
            ActivationPolicy::TaskAndThread,
            ContextPolicy::InvocationOnly,
        );
        let root = append_room_agent_reply(&state, &key, "helper", "agent root", None)
            .expect("agent root append");

        // Both the explicit mention and the thread-root author resolve to helper.
        // Policy evaluation reports both, but dispatch must queue helper once.
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "john".into(),
                author_kind: RoomParticipantKind::Human,
                body: "@helper following up".into(),
                thread_parent_seq: Some(root.seq),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let trigger_seq = body.0["message"]["seq"].as_u64().unwrap();
        assert!(trigger_seq > root.seq, "trigger must be a later reply row");
        assert_eq!(body.0["message"]["thread_parent_seq"], root.seq);
        assert_eq!(body.0["triggers_fired"].as_array().unwrap().len(), 2);

        let mut room_trigger_count = 0;
        while let Ok(event) = trigger_rx.try_recv() {
            if matches!(
                event.event,
                AgentTurnEvent::Extension { ref extension, .. } if extension == "room_trigger"
            ) {
                room_trigger_count += 1;
            }
        }
        assert_eq!(room_trigger_count, 1, "one agent gets one dispatch per row");

        let expected_session =
            authorized_room_agent_session_id(&key, "helper", generation).to_string();
        let mut generated_reply = None;
        for _ in 0..100 {
            generated_reply = with_rooms(&state, |store| store.transcript(&key, None))
                .unwrap()
                .into_iter()
                .find(|message| {
                    message.seq > trigger_seq
                        && message.author_id == "helper"
                        && message.session_id.as_deref() == Some(expected_session.as_str())
                });
            if generated_reply.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let generated_reply = generated_reply.expect("convened agent reply must persist");
        // The convened answer hangs under the thread ROOT — not the reply row
        // that mentioned the agent, which one-level threading would reject.
        assert_eq!(generated_reply.thread_parent_seq, Some(root.seq));
        assert_eq!(
            generated_reply.session_id.as_deref(),
            Some(expected_session.as_str())
        );

        // A reply cannot parent another reply. Agent output degrades to top-level
        // rather than disappearing, while retaining daemon-derived attribution.
        let fallback = append_room_agent_reply(
            &state,
            &key,
            "helper",
            "fallback reply",
            Some(generated_reply.seq),
        )
        .expect("invalid agent parent must fall back");
        assert_eq!(fallback.thread_parent_seq, None);
        let legacy_session = room_agent_session_id(&key, "helper").to_string();
        assert_eq!(
            fallback.session_id.as_deref(),
            Some(legacy_session.as_str())
        );
    }

    #[test]
    fn p2c_registration_key_matches_frozen_known_answer() {
        let key = registration_key(
            "11111111-1111-4111-8111-111111111111",
            &RoomKey::new("room-a"),
            "sage",
        );
        assert!(
            key == "b8b1b37415ebcbcf56fc283df2f49841bd9e06775758115995e66869061ffd34",
            "registration-key known-answer mismatch"
        );
    }

    #[test]
    fn p2c_registration_key_prefixes_utf8_byte_lengths() {
        assert!(
            registration_key("é", &RoomKey::new("room-a"), "sage")
                == "54147903ac5f28cb0a613e96d8d74ed2b0ce053ca3e8c8020634c1abace3befb",
            "UTF-8 byte-length registration-key mismatch"
        );
        assert!(
            registration_key("ab", &RoomKey::new("c"), "d")
                != registration_key("a", &RoomKey::new("bc"), "d"),
            "length prefixes must separate otherwise ambiguous concatenations"
        );
    }

    #[test]
    fn p2c_control_bodies_deny_unknown_fields() {
        assert!(
            serde_json::from_str::<CreateInviteBody>(r#"{"ttl_minutes":1440,"role":"admin"}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<RedeemInviteBody>(
            r#"{"code":"secret","token":"forbidden"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<RegisterAgentsBody>(
            r#"{"agent_names":["sage"],"path":"/tmp"}"#
        )
        .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2c_control_routes_return_frozen_raw_success_envelopes() {
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let agents_root = tmp.path().join("route-agents");
        write_agent_fixture(&agents_root, "route-agent", r#"model = "fake-ok""#, None);
        let max_agent_names: Vec<_> = (0..32).map(|index| format!("route-max-{index}")).collect();
        for name in &max_agent_names {
            write_agent_fixture(&agents_root, name, r#"model = "fake-ok""#, None);
        }
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let (base, server) = start_route_bedrock().await;

        let invite_state = with_route_supervisor(fake_convene_state(&tmp), &base);
        let invite_key = RoomKey::new("route-invite");
        with_rooms(&invite_state, |store| {
            store.create(invite_key.clone(), "Route Invite", None, Utc::now())
        })
        .unwrap();
        let response = crate::room_routes()
            .with_state(invite_state.clone())
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/route-invite/invites")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"recipient_name":"Peer","ttl_minutes":1440}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let invite: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(invite.as_object().unwrap().len(), 4);
        assert!(invite["code"] == "route-share-code", "invite code mismatch");
        assert_eq!(invite["expires_at"], "2026-07-18T00:00:00Z");
        assert_eq!(invite["room_key"], "route-invite");
        assert_eq!(invite["room_name"], "Route Invite");
        invite_state.room_federation.shutdown().await;

        let redeem_state = with_route_supervisor(fake_convene_state(&tmp), &base);
        let response = crate::room_routes()
            .with_state(redeem_state.clone())
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/invites/redeem")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"code":"route-code"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let redeem: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(redeem.get("ok").is_none());
        assert!(redeem.get("access").is_none());
        assert!(redeem.get("state").is_some());
        assert_eq!(redeem["room_key"], "route-redeem");
        let redeem_json = redeem.to_string();
        assert!(!redeem_json.contains("route-code"));
        assert!(!redeem_json.contains("token"));
        redeem_state.room_federation.shutdown().await;

        let agent_state = with_route_supervisor(fake_convene_state(&tmp), &base);
        let agent_key = RoomKey::new("route-agents");
        with_rooms(&agent_state, |store| {
            store.create(agent_key.clone(), "Route Agents", None, Utc::now())?;
            store.install_room_credential(
                &agent_key,
                "agent-route-bearer",
                "22222222-2222-4222-8222-222222222222",
            )?;
            store.update_room_access_safe(
                &agent_key,
                Some(RoomAccessState::Connecting),
                None,
                None,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let response = crate::room_routes()
            .with_state(agent_state.clone())
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/route-agents/members/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agent_names":["route-agent"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let agents: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(agents.get("ok").is_none());
        assert!(agents.get("access").is_none());
        assert_eq!(agents["members"].as_array().unwrap().len(), 2);
        assert_eq!(
            agents["members"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|member| member["actor_type"] == "agent")
                .count(),
            1
        );
        let agents_json = agents.to_string();
        for private_field in ["registration_key", "bearer", "tools", "path"] {
            assert!(!agents_json.contains(private_field));
        }
        agent_state.room_federation.shutdown().await;

        let max_state = with_route_supervisor(fake_convene_state(&tmp), &base);
        let max_key = RoomKey::new("route-agents-max");
        with_rooms(&max_state, |store| {
            store.create(max_key.clone(), "Route Agents Max", None, Utc::now())?;
            store.install_room_credential(
                &max_key,
                "agent-route-bearer",
                "22222222-2222-4222-8222-222222222222",
            )?;
            store.update_room_access_safe(
                &max_key,
                Some(RoomAccessState::Connecting),
                None,
                None,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let request_body = serde_json::to_vec(&json!({"agent_names":max_agent_names})).unwrap();
        let response = crate::room_routes()
            .with_state(max_state.clone())
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/route-agents-max/members/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let maximum: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 128 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(maximum["members"].as_array().unwrap().len(), 33);
        assert_eq!(
            maximum["members"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|member| member["actor_type"] == "agent")
                .count(),
            32
        );
        max_state.room_federation.shutdown().await;
        server.abort();
    }

    #[tokio::test]
    async fn p2c_http_message_ignores_claimed_identity_and_closed_agent_route_is_404() {
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("p2c-http-room");
        let human = "11111111-1111-4111-8111-111111111111";
        with_rooms(&state, |store| {
            store.create(key.clone(), "P2C HTTP", None, Utc::now())?;
            store.install_room_credential(&key, "private-bearer", human)?;
            store.update_room_access_safe(&key, Some(RoomAccessState::Live), None, None)?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let (status, Json(body)) = room_post_message(
            State(state.clone()),
            Path(key.as_str().into()),
            Json(RoomMessageRequest {
                author_id: "browser-forgery".into(),
                author_kind: RoomParticipantKind::Agent,
                body: "federated intent".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["ok"], true);
        let projection = with_rooms(&state, |store| store.room_access(&key)).unwrap();
        assert_eq!(projection.outbox.len(), 1);
        assert_eq!(projection.outbox[0].author_member_id, human);
        assert!(with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .is_empty());

        with_rooms(&state, |store| store.close(&key)).unwrap();
        let (status, Json(body)) = room_register_agents(
            State(state),
            Path(key.as_str().into()),
            Ok(Json(RegisterAgentsBody {
                agent_names: vec!["does-not-exist".into()],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "room_not_found");
    }

    #[tokio::test]
    async fn p2c_message_errors_use_frozen_stable_envelopes() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let request = || RoomMessageRequest {
            author_id: "human".into(),
            author_kind: RoomParticipantKind::Human,
            body: "intent".into(),
            thread_parent_seq: None,
        };

        let (status, Json(body)) = room_post_message(
            State(state.clone()),
            Path("missing-room".into()),
            Json(request()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({"ok":false,"error":"room_not_found"}));

        let closed = RoomKey::new("closed-message-room");
        let corrupt = RoomKey::new("nonlocal-without-credential");
        with_rooms(&state, |store| {
            store.create(closed.clone(), "Closed", None, Utc::now())?;
            store.close(&closed)?;
            store.create(corrupt.clone(), "Corrupt", None, Utc::now())?;
            store.update_room_access_safe(
                &corrupt,
                Some(RoomAccessState::Connecting),
                None,
                None,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let (status, Json(body)) = room_post_message(
            State(state.clone()),
            Path(closed.as_str().into()),
            Json(request()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({"ok":false,"error":"room_not_found"}));

        let (status, Json(body)) = room_post_message(
            State(state.clone()),
            Path(corrupt.as_str().into()),
            Json(request()),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({"ok":false,"error":"internal_error"}));
        assert!(with_rooms(&state, |store| store.transcript(&corrupt, None))
            .unwrap()
            .is_empty());
        assert!(with_rooms(&state, |store| store.pending_outbox(&corrupt))
            .unwrap()
            .is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2c_message_conversion_race_commits_local_or_pending_never_both() {
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("p2c-conversion-race");
        let human = "11111111-1111-4111-8111-111111111111";
        with_rooms(&state, |store| {
            store.create(key.clone(), "Conversion Race", None, Utc::now())
        })
        .unwrap();
        // G3: the local branch only accepts an admitted author, so the race is
        // still a race between a local commit and a federated hand-off. P2: that
        // author is the daemon owner.
        seed_owner(&state, "claimed-human", "Claimed Human");
        join_participant(
            &state,
            &key,
            "claimed-human",
            RoomParticipantKind::Human,
            "Claimed Human",
        );
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let post = tokio::spawn({
            let state = state.clone();
            let key = key.clone();
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                room_post_message(
                    State(state),
                    Path(key.as_str().into()),
                    Json(RoomMessageRequest {
                        author_id: "claimed-human".into(),
                        author_kind: RoomParticipantKind::Human,
                        body: "conversion race".into(),
                        thread_parent_seq: None,
                    }),
                )
                .await
            }
        });
        let install = tokio::spawn({
            let state = state.clone();
            let key = key.clone();
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                with_rooms(&state, |store| {
                    store.install_room_credential(&key, "conversion-bearer", human)?;
                    store.update_room_access_safe(
                        &key,
                        Some(RoomAccessState::Connecting),
                        None,
                        None,
                    )?;
                    Ok::<_, ocean_store::RoomStoreError>(())
                })
                .unwrap();
            }
        });
        barrier.wait().await;
        let (post, install) = tokio::join!(post, install);
        let (status, _) = post.unwrap();
        install.unwrap();
        let transcript = with_rooms(&state, |store| store.transcript(&key, None)).unwrap();
        // Only chat rows are the race's output; the roster join above is fixture
        // setup and is always present.
        let chat: Vec<_> = transcript
            .iter()
            .filter(|m| m.kind == RoomMessageKind::Message)
            .collect();
        let pending = with_rooms(&state, |store| store.pending_outbox(&key)).unwrap();
        match status {
            StatusCode::CREATED => {
                assert_eq!(chat.len(), 1);
                assert!(pending.is_empty());
            }
            StatusCode::ACCEPTED => {
                assert!(chat.is_empty());
                assert_eq!(pending.len(), 1);
            }
            other => panic!("unexpected conversion-race status {other}"),
        }
    }

    #[tokio::test]
    async fn p2c_http_control_validation_is_400_before_network() {
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let state = fake_convene_state(&tmp);
        let noncanonical = RoomKey::new("Not-Canonical");
        with_rooms(&state, |store| {
            store.create(noncanonical.clone(), "Noncanonical", None, Utc::now())
        })
        .unwrap();
        let (status, Json(body)) = room_create_invite(
            State(state.clone()),
            Path(noncanonical.as_str().into()),
            Ok(Json(CreateInviteBody {
                recipient_name: None,
                ttl_minutes: Some(1440),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");

        let (status, _) = room_create_invite(
            State(state.clone()),
            Path("room".into()),
            Ok(Json(CreateInviteBody {
                recipient_name: None,
                ttl_minutes: Some(0),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = room_redeem_invite(
            State(state.clone()),
            Ok(Json(RedeemInviteBody { code: " ".into() })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = room_register_agents(
            State(state.clone()),
            Path("room".into()),
            Ok(Json(RegisterAgentsBody {
                agent_names: vec![],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let oversized_names: Vec<_> = (0..33).map(|index| format!("agent-{index}")).collect();
        let response = crate::room_routes()
            .with_state(state)
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/room/members/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"agent_names":oversized_names})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let (status, Json(body)) = intent_error_response(IntentError::InviteForbidden);
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "invite_forbidden");
        let (status, Json(body)) = intent_error_response(IntentError::Forbidden);
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "federation_forbidden");
        let (status, Json(body)) = intent_error_response(IntentError::Conflict);
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "federation_conflict");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p2c_dispatch_uses_local_name_but_agent_reply_reenters_outbox() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(
            &agents_root,
            "bound-agent",
            r#"model = "fake-ok""#,
            Some("FEDERATED_AGENT_INSTRUCTIONS"),
        );
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        clear_turn_captures();

        let key = RoomKey::new("p2c-dispatch-room");
        let human = "11111111-1111-4111-8111-111111111111";
        let member = "33333333-3333-4333-8333-333333333333";
        let mut owner = dispatch_human_projection(human);
        owner.role_in_room = ocean_core::FederatedRoomRole::Owner;
        with_rooms(&state, |store| {
            store.create_in_workspace(
                key.clone(),
                "Dispatch",
                Some(canonical_test_workspace(tmp.path())),
                None,
                Utc::now(),
            )?;
            store.install_room_credential(&key, "private-bearer", human)?;
            store.bind_room_agent(&key, member, "bound-agent", "registration-key")?;
            store.update_room_access_safe(
                &key,
                Some(RoomAccessState::Live),
                Some(&[
                    dispatch_agent_projection(member, human, "bound-agent"),
                    owner,
                ]),
                None,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let generation = authorize_room_agent_package_fixture(
            &state,
            &key,
            member,
            "bound-agent",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );
        assert!(
            with_rooms(&state, |store| {
                room_agent_authority::current_binding_on(store, &key, member, generation)
            })
            .unwrap()
            .is_some(),
            "sovereign fixture must have current owner/binding authority"
        );
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let dispatcher = tokio::spawn(run_federated_trigger_dispatcher(
            state.clone(),
            rx,
            cancel.clone(),
        ));
        tx.send(FederatedTriggerDispatch {
            room: key.clone(),
            ledger_event_id: "ledger-trigger".into(),
            local_seq: 7,
            target_member_id: member.into(),
            trigger_kind: FederatedTriggerKind::Mention,
            reason: format!("on_mention: @{member} mentioned"),
        })
        .unwrap();

        let capture = wait_for_turn_capture("bound-agent")
            .await
            .expect("federated dispatch must run the bound local agent");
        // TASK-54: the instructions layer is framed with the folder-as-agent
        // sentinels for display stripping.
        assert!(capture.prompt.starts_with(
            "[folder-agent instructions]\nFEDERATED_AGENT_INSTRUCTIONS\n[end folder-agent instructions]\n\n"
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let outbox = with_rooms(&state, |store| store.pending_outbox(&key)).unwrap();
                if !outbox.is_empty() {
                    assert_eq!(outbox.len(), 1);
                    assert_eq!(outbox[0].author_member_id, member);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("agent reply must become a Pending outbox item");
        assert!(with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .iter()
            .all(|row| row.kind == RoomMessageKind::System));

        // Main follows this order: federation producers stop, the dedicated
        // dispatcher cancellation fires, and the retained JoinHandle is joined.
        drop(tx);
        cancel.cancel();
        dispatcher.await.unwrap();
    }

    #[tokio::test]
    async fn p2c_unresolved_and_stale_dispatches_emit_no_turn_and_only_denial_audits() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("stale-dispatch-agents");
        write_agent_fixture(&agents_root, "stale-agent", r#"model = "fake-ok""#, None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        clear_turn_captures();
        let unresolved = RoomKey::new("p2c-unresolved-dispatch");
        let removed = RoomKey::new("p2c-removed-binding");
        let reclassified = RoomKey::new("p2c-reclassified-binding");
        let remote = RoomKey::new("p2c-remote-binding");
        let member = "33333333-3333-4333-8333-333333333333";
        with_rooms(&state, |store| {
            for (key, agent_name) in [
                (&unresolved, "missing-folder-agent"),
                (&removed, "stale-agent"),
                (&reclassified, "stale-agent"),
                (&remote, "stale-agent"),
            ] {
                store.create(key.clone(), key.as_str(), None, Utc::now())?;
                store.install_room_credential(key, "private-bearer", "human")?;
                store.bind_room_agent(key, member, agent_name, "private-key")?;
            }
            store.update_room_access_safe(
                &unresolved,
                Some(RoomAccessState::Live),
                Some(&[dispatch_agent_projection(
                    member,
                    "human",
                    "missing-folder-agent",
                )]),
                None,
            )?;
            store.update_room_access_safe(
                &removed,
                Some(RoomAccessState::Live),
                Some(&[]),
                None,
            )?;
            store.update_room_access_safe(
                &reclassified,
                Some(RoomAccessState::Live),
                Some(&[dispatch_human_projection(member)]),
                None,
            )?;
            store.update_room_access_safe(
                &remote,
                Some(RoomAccessState::Live),
                Some(&[dispatch_agent_projection(
                    member,
                    "remote-human",
                    "stale-agent",
                )]),
                None,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let dispatcher = tokio::spawn(run_federated_trigger_dispatcher(
            state.clone(),
            rx,
            CancellationToken::new(),
        ));
        for (room, ledger) in [
            (unresolved.clone(), "unresolved-ledger"),
            (removed.clone(), "removed-ledger"),
            (reclassified.clone(), "reclassified-ledger"),
            (remote.clone(), "remote-ledger"),
        ] {
            tx.send(FederatedTriggerDispatch {
                room,
                ledger_event_id: ledger.into(),
                local_seq: 1,
                target_member_id: member.into(),
                trigger_kind: FederatedTriggerKind::Mention,
                reason: format!("on_mention: @{member} mentioned"),
            })
            .unwrap();
        }
        drop(tx);
        dispatcher.await.unwrap();

        assert!(ROOM_TURN_CAPTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        for key in [&unresolved, &removed, &reclassified, &remote] {
            let transcript = with_rooms(&state, |store| store.transcript(key, None)).unwrap();
            assert!(transcript.iter().all(|row| {
                row.kind == RoomMessageKind::System && row.body.contains("\"outcome\":\"refused\"")
            }));
            assert!(with_rooms(&state, |store| store.pending_outbox(key))
                .unwrap()
                .is_empty());
        }
    }

    fn clear_turn_captures() {
        ROOM_TURN_CAPTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    async fn wait_for_turn_capture(agent_id: &str) -> Option<RoomTurnCapture> {
        for _ in 0..200 {
            let capture = ROOM_TURN_CAPTURES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .find(|capture| capture.agent_id == agent_id)
                .cloned();
            if capture.is_some() {
                return capture;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        None
    }

    fn create_mention_room(state: &AppState, key: &RoomKey) {
        let workspace = canonical_test_workspace(state.runtime.config_dir());
        with_rooms(state, |store| {
            store
                .create_in_workspace(
                    key.clone(),
                    "Named Agent Seam",
                    Some(workspace),
                    Some(RoomTriggerPolicy {
                        on_mention: true,
                        ..Default::default()
                    }),
                    Utc::now(),
                )
                .expect("room fixture");
        });
    }

    fn create_plain_room(state: &AppState, key: &RoomKey) {
        with_rooms(state, |store| {
            store
                .create(key.clone(), key.as_str(), None, Utc::now())
                .expect("room fixture");
        });
    }

    /// Admit a `human` Human participant (G3 author authority): a locally posted
    /// message is refused with 403 unless its `(id, kind)` pair is already on the
    /// roster. The join itself commits a `ParticipantJoined` row, so fixtures that
    /// assert on `seq` or on tail ordering account for it explicitly.
    ///
    /// Team-platform P2: local human posts are authored by the daemon owner, so
    /// this fixture also seeds the owner as `human`.
    fn join_human(state: &AppState, key: &RoomKey) {
        seed_owner(state, "human", "Human");
        join_participant(state, key, "human", RoomParticipantKind::Human, "Human");
    }

    fn join_participant(
        state: &AppState,
        key: &RoomKey,
        id: &str,
        kind: RoomParticipantKind,
        display_name: &str,
    ) {
        with_rooms(state, |store| {
            store
                .add_participant(
                    key,
                    RoomParticipant {
                        id: id.into(),
                        kind,
                        display_name: display_name.into(),
                    },
                    Utc::now(),
                )
                .expect("roster fixture");
        });
    }

    async fn paused_tail(
        state: &AppState,
        key: &RoomKey,
        resume: Option<u64>,
    ) -> (ReceiverStream<RoomMessage>, oneshot::Sender<()>) {
        let hints = state.room_wakes.subscribe();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let stream = room_message_tail(
            state.clone(),
            key.clone(),
            resume,
            hints,
            Some((ready_tx, release_rx)),
        );
        tokio::time::timeout(std::time::Duration::from_secs(1), ready_rx)
            .await
            .expect("tail replay ready timeout")
            .expect("tail replay task dropped");
        (stream, release_tx)
    }

    async fn next_message(stream: &mut ReceiverStream<RoomMessage>) -> RoomMessage {
        tokio::time::timeout(std::time::Duration::from_millis(250), stream.next())
            .await
            .expect("room message exceeded 250ms")
            .expect("room tail ended")
    }

    async fn wait_for_wake_receivers(state: &AppState, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if state.room_wakes.receiver_count() == expected {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("room tail retained its wake receiver after client disconnect");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_releases_wake_receiver_when_client_disconnects_idle() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("disconnect-release");
        create_plain_room(&state, &key);
        assert_eq!(state.room_wakes.receiver_count(), 0);

        // Disconnect while the test seam is paused: the tail must observe the
        // closed mpsc receiver without waiting forever on the seam release.
        let (paused, release) = paused_tail(&state, &key, None).await;
        assert_eq!(state.room_wakes.receiver_count(), 1);
        drop(paused);
        wait_for_wake_receivers(&state, 0).await;
        assert!(release.send(()).is_err(), "paused tail task still alive");

        // Disconnect again after entering the ordinary idle live wait. No room
        // hint is published, so only `tx.closed()` can release the task.
        let (live, release) = paused_tail(&state, &key, None).await;
        assert_eq!(state.room_wakes.receiver_count(), 1);
        release.send(()).expect("release live tail");
        tokio::task::yield_now().await;
        drop(live);
        wait_for_wake_receivers(&state, 0).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_fans_out_post_once_in_order_under_250ms() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("fanout");
        create_plain_room(&state, &key);
        // The author must be admitted before it may post (G3). Its join row is
        // seq 0, so both tails resume after it and the posts are seq 1 and 2.
        join_human(&state, &key);

        let (mut first, release_first) = paused_tail(&state, &key, Some(0)).await;
        let (mut second, release_second) = paused_tail(&state, &key, Some(0)).await;
        release_first.send(()).unwrap();
        release_second.send(()).unwrap();

        let (status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "first".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let returned_at = tokio::time::Instant::now();
        let first_a = next_message(&mut first).await;
        let first_b = next_message(&mut second).await;
        assert!(returned_at.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(first_a, first_b);
        assert_eq!(first_a.seq, 1);

        let (status, _) = room_post_message(
            State(state),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "second".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(next_message(&mut first).await.seq, 2);
        assert_eq!(next_message(&mut second).await.seq, 2);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), first.next())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), second.next())
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_last_event_resume_has_no_gap_or_duplicate() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("resume");
        create_plain_room(&state, &key);
        for body in ["zero", "one", "two", "three"] {
            append_room_message(
                &state,
                &key,
                "human",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                body,
            )
            .unwrap();
        }

        let (mut resumed, release) = paused_tail(&state, &key, Some(1)).await;
        release.send(()).unwrap();
        assert_eq!(next_message(&mut resumed).await.seq, 2);
        assert_eq!(next_message(&mut resumed).await.seq, 3);
        append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "four",
        )
        .unwrap();
        assert_eq!(next_message(&mut resumed).await.seq, 4);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), resumed.next())
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_isolates_other_rooms() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let room_a = RoomKey::new("room-a");
        let room_b = RoomKey::new("room-b");
        create_plain_room(&state, &room_a);
        create_plain_room(&state, &room_b);
        let (mut tail_a, release) = paused_tail(&state, &room_a, None).await;
        release.send(()).unwrap();

        append_room_message(
            &state,
            &room_b,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "private to B",
        )
        .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(75), tail_a.next())
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_join_leave_and_auto_convene_audit_are_live() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        seed_owner(&state, "amy", "Amy");
        let plain = RoomKey::new("roster-live");
        create_plain_room(&state, &plain);
        let (mut roster_tail, release) = paused_tail(&state, &plain, None).await;
        release.send(()).unwrap();

        let (status, _) = room_join(
            State(state.clone()),
            Path(plain.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "amy".into(),
                display_name: "Amy".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            next_message(&mut roster_tail).await.kind,
            RoomMessageKind::ParticipantJoined
        );
        let (status, _) = room_leave(
            State(state.clone()),
            Path((plain.as_str().to_string(), "amy".into())),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            next_message(&mut roster_tail).await.kind,
            RoomMessageKind::ParticipantLeft
        );

        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "helper", "", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let convene = RoomKey::new("convene-live");
        create_mention_room(&state, &convene);
        // Author admission (G3) is seq 0 and the agent join is seq 1, so the
        // tail resumes after both and sees only the post + audit rows.
        join_human(&state, &convene);
        let (join_status, _) = room_join(
            State(state.clone()),
            Path(convene.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "helper".into(),
                display_name: "Helper".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(join_status, StatusCode::OK);
        let _generation = authorize_room_agent_fixture(
            &state,
            &convene,
            "helper",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );
        let authority_seq = with_rooms(&state, |store| store.transcript(&convene, None))
            .unwrap()
            .last()
            .expect("authority audit")
            .seq;
        let (mut convene_tail, release) = paused_tail(&state, &convene, Some(authority_seq)).await;
        release.send(()).unwrap();
        let (post_status, _) = room_post_message(
            State(state),
            Path(convene.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "@helper report".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(post_status, StatusCode::CREATED);
        assert_eq!(next_message(&mut convene_tail).await.body, "@helper report");
        let admission = next_message(&mut convene_tail).await;
        assert_eq!(admission.kind, RoomMessageKind::System);
        assert!(admission.body.contains("admission_id") && admission.body.contains("admitted"));
        let audit = next_message(&mut convene_tail).await;
        assert_eq!(audit.kind, RoomMessageKind::System);
        assert!(audit.body.starts_with("auto-convene: helper"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_buffers_replay_live_seam_hint() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("seam");
        create_plain_room(&state, &key);
        let (mut tail, release) = paused_tail(&state, &key, None).await;
        append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "during seam",
        )
        .unwrap();
        release.send(()).unwrap();
        let message = next_message(&mut tail).await;
        assert_eq!(message.seq, 0);
        assert_eq!(message.body, "during seam");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_recovers_forced_broadcast_lag_from_durable_pages() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut state = fake_convene_state(&tmp);
        state.room_wakes = RoomWakeBus::new(2);
        let key = RoomKey::new("lagged");
        create_plain_room(&state, &key);
        let (mut tail, release) = paused_tail(&state, &key, None).await;
        for i in 0..40 {
            append_room_message(
                &state,
                &key,
                "human",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                &format!("line-{i}"),
            )
            .unwrap();
        }
        release.send(()).unwrap();

        let mut seen = Vec::new();
        for _ in 0..40 {
            seen.push(next_message(&mut tail).await.seq);
        }
        assert_eq!(seen, (0..40).collect::<Vec<_>>());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), tail.next())
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_events_http_frame_uses_seq_id_and_exact_room_message_json() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("wire-frame");
        create_plain_room(&state, &key);
        let expected = append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "exact JSON",
        )
        .unwrap();
        let app = super::super::room_routes().with_state(state);
        let request = axum::http::Request::builder()
            .uri("/v1/rooms/persistent/wire-frame/events")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut body = response.into_body();
        // First frame is always room_access (S2-P1 contract). Skip it.
        let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
            .await
            .expect("access frame exceeded 250ms")
            .expect("SSE body ended")
            .expect("SSE body error");
        let access_wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame")).unwrap();
        assert!(
            access_wire.contains("event: room_access"),
            "expected room_access first, got: {access_wire:?}"
        );
        // Optional Local-room read cursor bootstrap may arrive before transcript replay.
        let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
            .await
            .expect("next frame exceeded 250ms")
            .expect("SSE body ended")
            .expect("SSE body error");
        let mut wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame"))
            .unwrap()
            .to_string();
        if wire.contains("event: room_read_cursor\n") {
            let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
                .await
                .expect("message frame exceeded 250ms")
                .expect("SSE body ended")
                .expect("SSE body error");
            wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame"))
                .unwrap()
                .to_string();
        }
        assert!(wire.contains("event: room_message\n"), "wire: {wire:?}");
        assert!(wire.contains("id: 0\n"), "wire: {wire:?}");
        let data = wire
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("data line");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(data).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_events_http_last_event_id_wins_and_replays_strictly_after() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("header-resume");
        create_plain_room(&state, &key);
        for body in ["zero", "one", "two"] {
            append_room_message(
                &state,
                &key,
                "human",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                body,
            )
            .unwrap();
        }
        let app = super::super::room_routes().with_state(state);
        let request = axum::http::Request::builder()
            .uri("/v1/rooms/persistent/header-resume/events?after_seq=0")
            .header("last-event-id", "1")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        // Skip initial room_access frame (S2-P1 contract).
        let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
            .await
            .expect("access frame exceeded 250ms")
            .expect("SSE body ended")
            .expect("SSE body error");
        let access_wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame")).unwrap();
        assert!(
            access_wire.contains("event: room_access"),
            "expected room_access first, got: {access_wire:?}"
        );
        // Optional Local-room read cursor bootstrap may arrive before replay.
        let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
            .await
            .expect("resume frame exceeded 250ms")
            .expect("SSE body ended")
            .expect("SSE body error");
        let mut wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame"))
            .unwrap()
            .to_string();
        if wire.contains("event: room_read_cursor\n") {
            let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
                .await
                .expect("resume message frame exceeded 250ms")
                .expect("SSE body ended")
                .expect("SSE body error");
            wire = std::str::from_utf8(frame.data_ref().expect("SSE data frame"))
                .unwrap()
                .to_string();
        }
        assert!(wire.contains("id: 2\n"), "wire: {wire:?}");
        assert!(wire.contains("\"body\":\"two\""), "wire: {wire:?}");
        assert!(!wire.contains("\"body\":\"one\""), "wire: {wire:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_events_rejects_invalid_resume_unknown_closed_and_call_rooms() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let open = RoomKey::new("open-room");
        let closed = RoomKey::new("closed-room");
        let call = RoomKey::new("call:excluded");
        for key in [&open, &closed, &call] {
            create_plain_room(&state, key);
        }
        with_rooms(&state, |store| store.close(&closed)).unwrap();

        let mut invalid = HeaderMap::new();
        invalid.insert("last-event-id", "not-a-number".parse().unwrap());
        let result = room_events(
            State(state.clone()),
            Path(open.as_str().to_string()),
            Query(RoomEventsQuery { after_seq: Some(7) }),
            invalid,
        )
        .await;
        let Err((status, Json(body))) = result else {
            panic!("invalid Last-Event-ID unexpectedly opened a stream");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_last_event_id");

        let mut numeric = HeaderMap::new();
        numeric.insert("last-event-id", "11".parse().unwrap());
        assert_eq!(
            room_resume_seq(&numeric, &RoomEventsQuery { after_seq: Some(7) }).unwrap(),
            Some(11),
            "numeric Last-Event-ID must win over after_seq"
        );

        for (key, expected_status, expected_code) in [
            ("missing", StatusCode::NOT_FOUND, "room_not_found"),
            (closed.as_str(), StatusCode::NOT_FOUND, "room_not_found"),
            (
                call.as_str(),
                StatusCode::BAD_REQUEST,
                "call_room_events_unsupported",
            ),
        ] {
            let result = room_events(
                State(state.clone()),
                Path(key.to_string()),
                Query(RoomEventsQuery::default()),
                HeaderMap::new(),
            )
            .await;
            let Err((status, Json(body))) = result else {
                panic!("rejected room unexpectedly opened a stream");
            };
            assert_eq!(status, expected_status);
            assert_eq!(body["code"], expected_code);
        }
    }

    /// END TO END: a worker adds their agent over HTTP, and the room reports
    /// that the agent is THEIRS. This is the whole point of the feature — the
    /// store gates prove the write, this proves it is reachable and projected.
    /// Mutation: make `room_join` ignore `owner_id` (always take the
    /// non-owner store path) -> agent_owners comes back empty -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_records_agent_ownership_and_room_get_projects_it() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "researcher", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        // P2: the human join below is authored as the daemon owner.
        seed_owner(&state, "alice", "Alice");
        let key = RoomKey::new("owned-room");
        create_mention_room(&state, &key);

        // The worker joins first — an agent cannot be owned by someone absent.
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "alice".into(),
                display_name: "Alice".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Then adds THEIR agent.
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "researcher".into(),
                display_name: "Researcher".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, Json(body)) =
            room_get(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["agent_owners"],
            json!([{ "agent_id": "researcher", "owner_id": "alice", "owner_present": true }]),
            "the room must report whose agent this is"
        );
    }

    /// An owner named for a participant who is not on the roster is refused,
    /// and the refusal writes nothing — no roster row, no join marker.
    /// Mutation: delete the store's `None =>` owner arm -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_with_an_absent_owner_is_refused_and_writes_nothing() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "researcher", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("absent-owner");
        create_mention_room(&state, &key);

        let (status, _body) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "researcher".into(),
                display_name: "Researcher".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("nobody".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let room = with_rooms(&state, |store| store.get(&key))
            .expect("room lookup")
            .expect("room exists");
        assert!(
            room.room.participants.is_empty(),
            "a refused owner must leave no roster row"
        );
        assert!(
            room.transcript.is_empty(),
            "a refused owner must forge no join marker"
        );
    }

    /// Only an Agent may carry an owner; anything else is refused rather than
    /// silently dropped. A caller that believed it recorded ownership and did
    /// not is the false-success class.
    /// Mutation: delete the `owner_requires_agent` arm -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_id_on_a_non_agent_is_refused() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("owner-on-human");
        create_mention_room(&state, &key);

        let (status, Json(body)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "bob".into(),
                display_name: "Bob".into(),
                kind: RoomParticipantKind::Human,
                owner_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], json!("owner_requires_agent"));
    }

    #[test]
    fn client_artifact_author_guard_serializes_roster_replacement() {
        let rooms = Arc::new(Mutex::new(
            ocean_store::SqliteRoomStore::open_in_memory().unwrap(),
        ));
        let key = RoomKey::new("artifact-roster-race");
        with_rooms_handle(&rooms, |store| {
            store.create(key.clone(), "Race", None, Utc::now()).unwrap();
            store
                .add_participant(
                    &key,
                    RoomParticipant {
                        id: "author".into(),
                        kind: RoomParticipantKind::Human,
                        display_name: "Author".into(),
                    },
                    Utc::now(),
                )
                .unwrap();
        });
        let competing_rooms = rooms.clone();
        let competing_key = key.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let mut replacement = None;
        let artifact = with_client_artifact_author(&rooms, &key, "author", |store| {
            replacement = Some(std::thread::spawn(move || {
                assert!(matches!(
                    competing_rooms.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                started_tx.send(()).unwrap();
                with_rooms_handle(&competing_rooms, |store| {
                    store
                        .remove_participant(&competing_key, "author", Utc::now())
                        .unwrap();
                    store
                        .add_participant(
                            &competing_key,
                            RoomParticipant {
                                id: "author".into(),
                                kind: RoomParticipantKind::Agent,
                                display_name: "Author".into(),
                            },
                            Utc::now(),
                        )
                        .unwrap();
                });
            }));
            started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            store.create_artifact(
                &key,
                "real",
                RoomArtifactKind::Task,
                "Real",
                "",
                "author",
                Utc::now(),
            )
        })
        .unwrap()
        .unwrap()
        .0;
        replacement.unwrap().join().unwrap();
        assert_eq!(artifact.created_by, "author");
        assert_eq!(artifact.on_behalf_of, None);
        let refused = with_client_artifact_author(&rooms, &key, "author", |_| {
            panic!("an Agent author must never reach the client write")
        })
        .unwrap();
        assert_eq!(refused, None::<()>);
    }

    /// Finding B (pro-adversary): `author_id` is caller-supplied and only
    /// roster-checked, so a hostile local caller could author an artifact AS
    /// somebody's agent. An agent's artifact is produced by the daemon's convene
    /// path, never by a client claiming its identity over the wire.
    /// Mutation: delete the forged_artifact_author arm -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_cannot_author_an_artifact_as_an_agent() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "researcher", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        // P2: the human join below is authored as the daemon owner.
        seed_owner(&state, "alice", "Alice");
        let key = RoomKey::new("forge-artifact");
        create_mention_room(&state, &key);
        for (id, name, kind) in [
            ("alice", "Alice", RoomParticipantKind::Human),
            ("researcher", "Researcher", RoomParticipantKind::Agent),
        ] {
            let (status, _) = room_join(
                State(state.clone()),
                Path(key.as_str().to_string()),
                Json(RoomJoinRequest {
                    id: id.into(),
                    display_name: name.into(),
                    kind,
                    owner_id: None,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        let (status, Json(body)) = room_create_artifact(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(CreateArtifactRequest {
                id: "forged".into(),
                kind: RoomArtifactKind::Task,
                title: "I am the agent".into(),
                body: String::new(),
                author_id: "researcher".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], json!("forged_artifact_author"));

        let (status, Json(list)) =
            room_list_artifacts(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            list["artifacts"].as_array().map(|a| a.len()),
            Some(0),
            "a forged artifact must not exist"
        );

        // A human author on the same route still works.
        let (status, _) = room_create_artifact(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(CreateArtifactRequest {
                id: "real".into(),
                kind: RoomArtifactKind::Task,
                title: "Real task".into(),
                body: String::new(),
                author_id: "alice".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, Json(body)) = room_amend_artifact(
            State(state.clone()),
            Path((key.as_str().to_string(), "real".into())),
            Json(AmendArtifactRequest {
                expected_version: 1,
                title: Some("Agent-forged amendment".into()),
                body: None,
                state: None,
                author_id: "researcher".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], json!("forged_artifact_author"));

        let (status, _) = room_amend_artifact(
            State(state.clone()),
            Path((key.as_str().to_string(), "real".into())),
            Json(AmendArtifactRequest {
                expected_version: 1,
                title: Some("   ".into()),
                body: None,
                state: None,
                author_id: "alice".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// The daemon authors every audit row as ("system", System). If a client can
    /// join as System, its ParticipantJoined marker is a System-authored
    /// transcript row indistinguishable from a genuine daemon audit line.
    /// Mutation: delete the System arm in `room_join` -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn system_kind_cannot_join_over_http() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("system-join");
        create_mention_room(&state, &key);

        let (status, Json(body)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "system".into(),
                display_name: "Ocean System".into(),
                kind: RoomParticipantKind::System,
                owner_id: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], json!("forged_participant_kind"));
        let room = with_rooms(&state, |store| store.get(&key))
            .expect("room lookup")
            .expect("room exists");
        assert!(
            room.room.participants.is_empty(),
            "a refused System join must leave no roster row"
        );
        assert!(
            room.transcript.is_empty(),
            "a refused System join must forge no transcript marker"
        );
    }

    /// Join and post must agree on what an id is. `classify_local_author`
    /// refuses an untrimmed id at POST, so accepting one at JOIN strands that
    /// participant forever with no way to discover why.
    /// Mutation: delete the id-normalization arm in `room_join` -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_refuses_the_untrimmed_id_that_post_would_strand() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("untrimmed-join");
        create_mention_room(&state, &key);

        // Client-supplied ids are still validated for non-human kinds (a Bot
        // posts under its roster id). Human ids are owner-derived (P2).
        for bad in [" bot ", "", "   "] {
            let (status, Json(body)) = room_join(
                State(state.clone()),
                Path(key.as_str().to_string()),
                Json(RoomJoinRequest {
                    id: bad.into(),
                    display_name: "Bot".into(),
                    kind: RoomParticipantKind::Bot,
                    owner_id: None,
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "id {bad:?} must be refused"
            );
            assert_eq!(body["code"], json!("invalid_participant_id"));
        }

        // A human join with an untrimmed claimed id joins as the canonical
        // owner id, and can therefore post.
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: " john ".into(),
                display_name: "John".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let room = with_rooms(&state, |store| store.get(&key))
            .expect("room lookup")
            .expect("room exists");
        assert_eq!(room.room.participants.len(), 1);
        assert_eq!(room.room.participants[0].id, "john");
    }

    /// An empty display name produces a " joined" marker with no author to read.
    /// Mutation: delete the display_name arm -> RED.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_refuses_an_empty_display_name() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("blank-name-join");
        create_mention_room(&state, &key);

        let (status, Json(body)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "ghost".into(),
                display_name: "   ".into(),
                kind: RoomParticipantKind::Bot,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], json!("invalid_display_name"));

        // A human join ignores the client display name and takes the owner's.
        let (status, Json(body)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "ghost".into(),
                display_name: "   ".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["room"]["participants"][0]["display_name"],
            json!("John")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missing_agentdef_join_is_rejected() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).expect("agents root");
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("missing-join");
        create_mention_room(&state, &key);

        let (status, Json(body)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "phantom".into(),
                display_name: "Phantom".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], json!(false));
        assert_eq!(body["code"], json!("agent_unresolved"));
        let room = with_rooms(&state, |store| store.get(&key))
            .expect("room lookup")
            .expect("room exists");
        assert!(room.room.participants.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn legacy_phantom_mention_has_no_convene_footprint_or_request() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fake_convene_state(&tmp);
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).expect("agents root");
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        clear_turn_captures();
        let (_replay, mut trigger_rx) = state.agent_events.subscribe_with_replay(None);
        let key = RoomKey::new("legacy-phantom");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        with_rooms(&state, |store| {
            store
                .add_participant(
                    &key,
                    RoomParticipant {
                        id: "phantom".into(),
                        kind: RoomParticipantKind::Agent,
                        display_name: "Phantom".into(),
                    },
                    Utc::now(),
                )
                .expect("legacy roster fixture");
        });

        let (status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "@phantom report".into(),
                thread_parent_seq: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert!(matches!(
            trigger_rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        assert!(state.requests.read().await.is_empty());
        assert!(wait_for_turn_capture("phantom").await.is_none());
        assert!(state.runtime.list_sessions(None).unwrap().is_empty());
        let transcript =
            with_rooms(&state, |store| store.transcript(&key, None)).expect("transcript");
        let refusals: Vec<Value> = transcript
            .iter()
            .filter(|message| {
                message.author_kind == RoomParticipantKind::System
                    && message.kind == RoomMessageKind::System
            })
            .filter_map(|message| serde_json::from_str::<Value>(&message.body).ok())
            .filter(|body| body["type"] == "room.agent.admission")
            .collect();
        assert_eq!(refusals.len(), 1);
        assert_eq!(refusals[0]["room_id"], key.as_str());
        assert_eq!(refusals[0]["agent_member_id"], "phantom");
        assert_eq!(refusals[0]["agent_package_id"], "phantom");
        assert_eq!(refusals[0]["outcome"], "refused");
        assert_eq!(refusals[0]["reason_code"], "agent_package_not_found");
        assert!(!transcript
            .iter()
            .any(|message| message.body.starts_with("auto-convene:")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn valid_folder_applies_instructions_model_allowlist_and_caps() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(
            &agents_root,
            "bound-agent",
            r#"model = "fake-ok"
tools = ["read", "glob"]

[[subprocess_capability]]
name = "fixture-cap"
command = "/definitely/missing/ocean-fixture-cap"
args = ["--stdio"]
env = { FIXTURE = "1" }
"#,
            Some("BOUND_AGENT_INSTRUCTIONS"),
        );
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        clear_turn_captures();
        let key = RoomKey::new("bound-agent-profile");
        create_mention_room(&state, &key);
        join_human(&state, &key);

        let (join_status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "bound-agent".into(),
                display_name: "Bound Agent".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(join_status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "bound-agent",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );

        let (post_status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "@bound-agent report".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(post_status, StatusCode::CREATED);

        let capture = wait_for_turn_capture("bound-agent")
            .await
            .expect("resolved room turn must reach runtime dispatch");
        // TASK-54: the instructions layer is framed with the folder-as-agent
        // sentinels so display projections can strip it; the frame encloses the
        // instructions and terminates before the user's prompt.
        assert!(capture
            .prompt
            .starts_with("[folder-agent instructions]\nBOUND_AGENT_INSTRUCTIONS\n[end folder-agent instructions]\n\n"));
        assert_eq!(
            capture.tool_allowlist,
            Some(vec!["glob".to_string(), "read".to_string()])
        );
        assert_eq!(capture.model.as_deref(), Some("fake-ok"));
        assert!(
            capture.subprocess_caps.is_none(),
            "the Phase 1 node ceiling must remove ambient subprocess access"
        );
        assert!(state.requests.read().await.values().any(|request| {
            request.status.session_id
                == Some(core_sid(authorized_room_agent_session_id(
                    &key,
                    "bound-agent",
                    generation,
                )))
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_data_only_agentdef_is_resolved_and_queued() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "data-only", "", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        clear_turn_captures();
        let (_replay, mut trigger_rx) = state.agent_events.subscribe_with_replay(None);
        let key = RoomKey::new("data-only-profile");
        create_mention_room(&state, &key);
        join_human(&state, &key);

        let (join_status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "data-only".into(),
                display_name: "Data Only".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(join_status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "data-only",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );

        let (post_status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "human".into(),
                author_kind: RoomParticipantKind::Human,
                body: "@data-only report".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(post_status, StatusCode::CREATED);
        let event = loop {
            let event = trigger_rx
                .try_recv()
                .expect("resolved agent emits room_trigger");
            if matches!(event.event, AgentTurnEvent::Extension { ref extension, .. } if extension == "room_trigger")
            {
                break event;
            }
        };
        assert!(matches!(
            event.event,
            AgentTurnEvent::Extension { ref extension, .. } if extension == "room_trigger"
        ));

        let capture = wait_for_turn_capture("data-only")
            .await
            .expect("all-None profile must still dispatch");
        assert!(capture.tool_allowlist.is_none());
        assert!(capture.model.is_none());
        assert!(capture.subprocess_caps.is_none());
        assert!(!capture.prompt.starts_with("\n\n"));
        assert!(state.requests.read().await.values().any(|request| {
            request.status.session_id
                == Some(core_sid(authorized_room_agent_session_id(
                    &key,
                    "data-only",
                    generation,
                )))
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p3_mention_produces_one_agent_run_that_ends_done_with_reply() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "helper", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p3-card");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "helper".into(),
                display_name: "Helper".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "helper",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: String::new(),
                author_kind: RoomParticipantKind::Human,
                body: "@helper fix the thing".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let trigger_seq = body.0["message"]["seq"].as_u64().expect("trigger seq");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let run = loop {
            let runs = with_rooms(&state, |store| store.room_agent_runs(&key, 10)).unwrap();
            if let Some(run) = runs.into_iter().find(|r| r.state.is_terminal()) {
                break run;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "agent run never reached a terminal state"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(run.agent_id, "helper");
        assert_eq!(run.trigger_seq, trigger_seq);
        assert_eq!(run.thread_root_seq, trigger_seq);
        assert_eq!(
            run.session_id,
            authorized_room_agent_session_id(&key, "helper", generation).to_string()
        );
        assert_eq!(run.state, ocean_core::RoomAgentRunState::Done, "{run:?}");
        let reply_seq = run.reply_seq.expect("done run links its reply");
        let reply = with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .into_iter()
            .find(|m| m.seq == reply_seq)
            .expect("reply row exists");
        assert_eq!(reply.author_id, "helper");
        assert_eq!(reply.thread_parent_seq, Some(trigger_seq));
        assert!(run.summary.is_some());
        assert_eq!(
            with_rooms(&state, |store| store.room_agent_runs(&key, 10))
                .unwrap()
                .len(),
            1,
            "one card per convened turn"
        );

        let (status, Json(listed)) =
            room_agent_runs_list(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["runs"][0]["run_id"], run.run_id);
        assert_eq!(listed["runs"][0]["state"], "done");
    }

    #[tokio::test]
    async fn p4_mutations_require_explicit_room_operator_before_lookup() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        for (headers, expected) in [
            (HeaderMap::new(), StatusCode::SERVICE_UNAVAILABLE),
            (
                {
                    let mut h = HeaderMap::new();
                    h.insert(
                        crate::room_operator::OPERATOR_HEADER,
                        "wrong".parse().unwrap(),
                    );
                    h
                },
                StatusCode::FORBIDDEN,
            ),
            (
                {
                    let mut h = HeaderMap::new();
                    h.insert(
                        crate::room_operator::OPERATOR_HEADER,
                        "test-room-operator".parse().unwrap(),
                    );
                    h.insert("cookie", "ambient=1".parse().unwrap());
                    h
                },
                StatusCode::FORBIDDEN,
            ),
        ] {
            let path = || Path(("missing-room".into(), "missing-run".into()));
            let (status, _) = room_agent_run_permission(
                State(state.clone()),
                path(),
                headers.clone(),
                Ok(Json(run_decision(
                    PermissionId::new_v4(),
                    None,
                    PermissionDecisionBody::Allow,
                ))),
            )
            .await;
            assert_eq!(status, expected);
            let (status, _) = room_agent_settings_put(
                State(state.clone()),
                path(),
                headers,
                Ok(Json(RoomAgentSettings::default())),
            )
            .await;
            assert_eq!(status, expected);
        }
    }

    fn run_decision(
        permission_id: PermissionId,
        tool: Option<&str>,
        decision: PermissionDecisionBody,
    ) -> RoomRunPermissionDecisionBody {
        RoomRunPermissionDecisionBody {
            permission_id: permission_id.to_string(),
            tool: tool.map(str::to_string),
            decision,
        }
    }

    /// A pending waiter bound to `token`, as the policy registers it.
    async fn register_room_waiter(
        state: &AppState,
        tool: &str,
        token: &str,
    ) -> (
        PermissionId,
        oneshot::Receiver<ocean_runtime::PermissionDecision>,
    ) {
        let permission_id = PermissionId::new_v4();
        let (tx, rx) = oneshot::channel();
        state.permissions.write().await.insert(
            permission_id,
            crate::request_control::PermissionWaiter {
                status: ocean_core::PermissionStatus {
                    permission_id,
                    request_id: Uuid::new_v4(),
                    session_id: None,
                    tool: tool.into(),
                    reason: format!("permission required for {tool}"),
                    args: json!({}),
                    created_at: Utc::now(),
                },
                sender: Some(tx),
                decision_token: Some(token.into()),
            },
        );
        (permission_id, rx)
    }

    fn project_pending(state: &AppState, run: &mut RoomAgentRun, id: PermissionId, tool: &str) {
        run.state = ocean_core::RoomAgentRunState::AwaitingPermission;
        run.pending_permission = Some(ocean_core::RoomRunPermission {
            permission_id: id.to_string(),
            tool: tool.into(),
            tool_label: tool.into(),
        });
        with_rooms(state, |store| store.put_room_agent_run(run)).unwrap();
    }

    #[tokio::test]
    async fn p4_room_permission_decision_is_bound_to_the_exact_pending_request() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("p4-bound-decision");
        create_mention_room(&state, &key);
        let mut operator = HeaderMap::new();
        operator.insert(
            crate::room_operator::OPERATOR_HEADER,
            "test-room-operator".parse().unwrap(),
        );
        let tracker = crate::room_agent_runs::RunTracker::start(
            state.clone(),
            key.clone(),
            "helper",
            AgentSessionId::new_v4(),
            1,
            1,
            "/repo".into(),
        );
        let token = tracker.mint_decision_token();
        let mut run = with_rooms(&state, |store| store.room_agent_runs(&key, 10))
            .unwrap()
            .pop()
            .unwrap();
        let run_id = run.run_id.clone();
        let decide = |body: RoomRunPermissionDecisionBody| {
            room_agent_run_permission(
                State(state.clone()),
                Path((key.as_str().to_string(), run_id.clone())),
                operator.clone(),
                Ok(Json(body)),
            )
        };

        // The owner sees the first request: a write.
        let (first, mut first_rx) = register_room_waiter(&state, "write", &token).await;
        project_pending(&state, &mut run, first, "write");

        // A decision naming no request does not decode at all.
        assert!(serde_json::from_value::<RoomRunPermissionDecisionBody>(
            json!({ "decision": "allow" })
        )
        .is_err());
        // Another id, or the right id with another tool, is refused untouched.
        for body in [
            run_decision(PermissionId::new_v4(), None, PermissionDecisionBody::Allow),
            run_decision(first, Some("bash"), PermissionDecisionBody::Allow),
        ] {
            let (status, _) = decide(body).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert!(state.permissions.read().await.contains_key(&first));
            assert!(first_rx.try_recv().is_err());
        }

        // That request is cancelled and the turn asks for something else.
        state.permissions.write().await.remove(&first);
        let (second, mut second_rx) = register_room_waiter(&state, "bash", &token).await;
        project_pending(&state, &mut run, second, "bash");

        // A retried Allow (or AllowSession) for the first request must never
        // approve the second.
        for decision in [
            PermissionDecisionBody::Allow,
            PermissionDecisionBody::AllowSession,
        ] {
            let (status, _) = decide(run_decision(first, Some("write"), decision)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert!(state.permissions.read().await.contains_key(&second));
            assert!(second_rx.try_recv().is_err());
        }

        // The decision the owner actually saw resolves exactly that waiter.
        let (status, _) = decide(run_decision(
            second,
            Some("bash"),
            PermissionDecisionBody::AllowSession,
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(matches!(
            second_rx.try_recv(),
            Ok(ocean_runtime::PermissionDecision::AllowSession)
        ));
        // A double click replays nothing.
        let (status, _) = decide(run_decision(
            second,
            Some("bash"),
            PermissionDecisionBody::Allow,
        ))
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Without the operator credential nothing is even looked up.
        let (third, mut third_rx) = register_room_waiter(&state, "bash", &token).await;
        project_pending(&state, &mut run, third, "bash");
        let (status, _) = room_agent_run_permission(
            State(state.clone()),
            Path((key.as_str().to_string(), run.run_id.clone())),
            HeaderMap::new(),
            Ok(Json(run_decision(
                third,
                Some("bash"),
                PermissionDecisionBody::Allow,
            ))),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(third_rx.try_recv().is_err());
        assert!(state.permissions.read().await.contains_key(&third));
    }

    #[tokio::test]
    async fn p4_room_tools_refuse_cancelled_and_stale_generation_writes() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "guarded", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p4-write-guards");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().into()),
            Json(RoomJoinRequest {
                id: "guarded".into(),
                display_name: "Guarded".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "guarded",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );
        let (admission, _permit) = room_agent_authority::admit_room_agent(
            &state,
            &key,
            "guarded",
            "guarded",
            AdmissionTrigger::Mention,
        )
        .await
        .unwrap();
        with_rooms(&state, |store| {
            store.put_room_agent_settings(
                &key,
                "guarded",
                &RoomAgentSettings {
                    model: Some("new-model".into()),
                    instructions: Some("next turn".into()),
                },
            )
        })
        .unwrap();
        let control = room_agent_authority::apply_admission_to_control(
            ocean_agent::PromptControl::yolo(false),
            &admission,
        );
        assert_eq!(control.agent_model.as_deref(), Some("fake-ok"));
        assert!(
            room_agent_authority::append_admission_allow(&state, &admission).is_err(),
            "changed settings cannot register against an older admitted snapshot"
        );
        let session_id = authorized_room_agent_session_id(&key, "guarded", generation);
        let trigger = with_rooms(&state, |store| store.transcript(&key, None))
            .unwrap()
            .last()
            .unwrap()
            .seq;
        let tracker = Arc::new(Mutex::new(crate::room_agent_runs::RunTracker::start(
            state.clone(),
            key.clone(),
            "guarded",
            session_id,
            trigger,
            trigger,
            tmp.path().display().to_string(),
        )));
        let baseline = with_rooms(&state, |store| store.transcript(&key, None)).unwrap();
        for cancelled in [false, true] {
            let cancel = CancellationToken::new();
            let mut scoped = admission.clone();
            if cancelled {
                cancel.cancel();
            } else {
                scoped.generation += 1;
            }
            let tools = crate::room_tools::room_turn_tools(crate::room_tools::RoomTurnBinding {
                state: state.clone(),
                admission: scoped,
                session_id,
                cancel,
                thread_root: trigger,
                tracker: tracker.clone(),
            });
            assert!(tools[0]
                .execute("update", json!({"text": "must not commit"}))
                .await
                .is_err());
            assert!(tools[1]
                .execute("ask", json!({"question": "must not commit"}))
                .await
                .is_err());
            assert_eq!(
                with_rooms(&state, |store| store.transcript(&key, None)).unwrap(),
                baseline
            );
        }
    }

    async fn wait_for_runs(state: &AppState, key: &RoomKey, n: usize) -> Vec<RoomAgentRun> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let runs = with_rooms(state, |store| store.room_agent_runs(key, 10)).unwrap();
            if runs.len() >= n && runs.iter().all(|r| r.state.is_terminal()) {
                return runs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "runs never settled: {runs:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p4_settings_tools_and_thread_reply_resume_a_parked_run() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "asker", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p4-ask");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "asker".into(),
                display_name: "Asker".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        authorize_room_agent_fixture(
            &state,
            &key,
            "asker",
            ActivationPolicy::TaskAndThread,
            ContextPolicy::InvocationOnly,
        );
        let mut operator_headers = HeaderMap::new();
        operator_headers.insert(
            crate::room_operator::OPERATOR_HEADER,
            "test-room-operator".parse().unwrap(),
        );
        let path = |agent: &str| Path((key.as_str().to_string(), agent.to_string()));

        // Per-room settings: normalized on write, scoped to room agents.
        let (status, _) = room_agent_settings_put(
            State(state.clone()),
            path("asker"),
            operator_headers.clone(),
            Ok(Json(RoomAgentSettings {
                instructions: Some("  P4-OVERLAY-MARKER  ".into()),
                model: Some("fake-ok".into()),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, Json(got)) =
            room_agent_settings_get(State(state.clone()), path("asker")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["settings"]["instructions"], "P4-OVERLAY-MARKER");
        let (status, _) = room_agent_settings_get(State(state.clone()), path("nobody")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = room_agent_settings_put(
            State(state.clone()),
            path("asker"),
            operator_headers.clone(),
            Ok(Json(RoomAgentSettings {
                instructions: None,
                model: Some("bad model!".into()),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: String::new(),
                author_kind: RoomParticipantKind::Human,
                body: "@asker paint the shed".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let trigger_seq = body.0["message"]["seq"].as_u64().expect("trigger seq");
        let first = wait_for_runs(&state, &key, 1).await.remove(0);

        // The convened turn carries the overlay, the room model, and the
        // room tools.
        let capture = ROOM_TURN_CAPTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|c| c.agent_id == "asker" && c.prompt.contains("P4-OVERLAY-MARKER"))
            .cloned()
            .expect("settings overlay reaches the prompt");
        assert_eq!(capture.model.as_deref(), Some("fake-ok"));
        assert!(capture.extra_tools.iter().any(|t| t == "room_post_update"));
        assert!(capture.extra_tools.iter().any(|t| t == "room_ask"));

        // Nothing pending: the in-room decision route refuses honestly.
        let (status, _) = room_agent_run_permission(
            State(state.clone()),
            Path((key.as_str().to_string(), first.run_id.clone())),
            operator_headers.clone(),
            Ok(Json(run_decision(
                PermissionId::new_v4(),
                None,
                PermissionDecisionBody::Allow,
            ))),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = room_agent_run_permission(
            State(state.clone()),
            Path((key.as_str().to_string(), "no-such-run".into())),
            operator_headers.clone(),
            Ok(Json(run_decision(
                PermissionId::new_v4(),
                None,
                PermissionDecisionBody::Allow,
            ))),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Park the run exactly as `room_ask` does, then answer in its thread.
        let mut parked = first.clone();
        parked.state = ocean_core::RoomAgentRunState::AwaitingReply;
        with_rooms(&state, |store| store.put_room_agent_run(&parked)).unwrap();
        let (status, body) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: String::new(),
                author_kind: RoomParticipantKind::Human,
                body: "blue".into(),
                thread_parent_seq: Some(trigger_seq),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let answer_seq = body.0["message"]["seq"].as_u64().expect("answer seq");

        let runs = wait_for_runs(&state, &key, 2).await;
        assert_eq!(runs.len(), 2, "the answer convenes exactly one new run");
        let closed = runs.iter().find(|r| r.run_id == first.run_id).unwrap();
        assert_eq!(closed.state, ocean_core::RoomAgentRunState::Done);
        let resumed = runs.iter().find(|r| r.run_id != first.run_id).unwrap();
        assert_eq!(resumed.agent_id, "asker");
        assert_eq!(resumed.trigger_seq, answer_seq);
        assert_eq!(resumed.thread_root_seq, trigger_seq);
        assert_eq!(
            resumed.session_id, first.session_id,
            "same session continues"
        );
        assert_eq!(resumed.state, ocean_core::RoomAgentRunState::Done);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p4_answer_resumption_claims_once_and_survives_refused_admission() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "asker", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p4-answer-claim");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "asker".into(),
                display_name: "Asker".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "asker",
            ActivationPolicy::TaskAndThread,
            ContextPolicy::InvocationOnly,
        );

        // A run parked by `room_ask` on a thread rooted at the human's task.
        let root = append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "paint the shed",
        )
        .unwrap()
        .seq;
        let mut tracker = crate::room_agent_runs::RunTracker::start(
            state.clone(),
            key.clone(),
            "asker",
            authorized_room_agent_session_id(&key, "asker", generation),
            root,
            root,
            tmp.path().display().to_string(),
        );
        tracker.awaiting_reply("Which colour?", None);
        drop(tracker);
        let parked_id = with_rooms(&state, |store| store.parked_room_agent_runs(&key, root))
            .unwrap()
            .pop()
            .expect("parked run")
            .run_id;
        let run = |id: &str| {
            with_rooms(&state, |store| store.room_agent_run(id))
                .unwrap()
                .unwrap()
        };
        let reply = |body: &str| {
            room_post_message(
                State(state.clone()),
                Path(key.as_str().to_string()),
                Json(RoomMessageRequest {
                    author_id: String::new(),
                    author_kind: RoomParticipantKind::Human,
                    body: body.into(),
                    thread_parent_seq: Some(root),
                }),
            )
        };
        let run_count = || {
            with_rooms(&state, |store| store.room_agent_runs(&key, 10))
                .unwrap()
                .len()
        };

        // The successor cannot be admitted: the answer must not be consumed.
        std::fs::remove_dir_all(agents_root.join("asker")).unwrap();
        let (status, _) = reply("blue").await;
        assert_eq!(status, StatusCode::CREATED);
        let after_refusal = run(&parked_id);
        assert!(
            after_refusal.state.is_parked(),
            "a refused admission leaves the run parked: {after_refusal:?}"
        );
        assert_eq!(after_refusal.answer_seq, None, "claim released");
        assert_eq!(run_count(), 1, "no successor");

        // Another answer holds the claim (a concurrent reply in flight): this
        // reply is refused and starts nothing.
        write_agent_fixture(&agents_root, "asker", "model = \"fake-ok\"\n", None);
        assert!(
            with_rooms(&state, |store| store.claim_parked_room_agent_run(
                &parked_id,
                9_999,
                Utc::now()
            ))
            .unwrap()
            .is_some()
        );
        let (status, _) = reply("green").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(run(&parked_id).answer_seq, Some(9_999));
        assert_eq!(run_count(), 1, "the losing reply starts no successor");
        with_rooms(&state, |store| {
            store.settle_room_agent_run_answer(&parked_id, 9_999, false, Utc::now())
        })
        .unwrap()
        .unwrap();

        // The next answer resumes it exactly once and only then closes it.
        let (status, body) = reply("red").await;
        assert_eq!(status, StatusCode::CREATED);
        let answer_seq = body.0["message"]["seq"].as_u64().unwrap();
        let closed = run(&parked_id);
        assert_eq!(closed.state, ocean_core::RoomAgentRunState::Done);
        assert_eq!(closed.answer_seq, Some(answer_seq));
        let runs = wait_for_runs(&state, &key, 2).await;
        assert_eq!(runs.len(), 2);
        let resumed = runs.iter().find(|r| r.run_id != parked_id).unwrap();
        assert_eq!(resumed.trigger_seq, answer_seq);
        assert_eq!(resumed.thread_root_seq, root);
        // A late duplicate reply finds nothing left to claim.
        let (status, _) = reply("red again").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            with_rooms(&state, |store| store.room_agent_runs(&key, 10))
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p4_answer_claim_settles_when_the_posting_request_is_dropped() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "asker", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p4-answer-drop");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "asker".into(),
                display_name: "Asker".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "asker",
            ActivationPolicy::TaskAndThread,
            ContextPolicy::InvocationOnly,
        );
        let root = append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "paint the shed",
        )
        .unwrap()
        .seq;
        let mut tracker = crate::room_agent_runs::RunTracker::start(
            state.clone(),
            key.clone(),
            "asker",
            authorized_room_agent_session_id(&key, "asker", generation),
            root,
            root,
            tmp.path().display().to_string(),
        );
        tracker.awaiting_reply("Which colour?", None);
        drop(tracker);
        let parked_id = with_rooms(&state, |store| store.parked_room_agent_runs(&key, root))
            .unwrap()
            .pop()
            .expect("parked run")
            .run_id;
        let run = |id: &str| {
            with_rooms(&state, |store| store.room_agent_run(id))
                .unwrap()
                .unwrap()
        };
        let reply = |body: &str| {
            room_post_message(
                State(state.clone()),
                Path(key.as_str().to_string()),
                Json(RoomMessageRequest {
                    author_id: String::new(),
                    author_kind: RoomParticipantKind::Human,
                    body: body.into(),
                    thread_parent_seq: Some(root),
                }),
            )
        };

        // Hold the request registry so the successor's registration waits:
        // the handler is then suspended after the claim, before the settle.
        let registry = state.requests.write().await;
        let mut post = Box::pin(reply("blue"));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), post.as_mut())
                .await
                .is_err(),
            "the reply is suspended inside the successor's admission"
        );
        assert!(
            run(&parked_id).answer_seq.is_some(),
            "suspended after the answer claimed the run"
        );
        // The client disconnects: the handler future is dropped mid-way.
        drop(post);
        drop(registry);
        let released = run(&parked_id);
        assert!(released.state.is_parked(), "{released:?}");
        assert_eq!(released.answer_seq, None, "the dropped claim is released");
        assert_eq!(
            with_rooms(&state, |store| store.room_agent_runs(&key, 10))
                .unwrap()
                .len(),
            1,
            "no successor was started"
        );

        // The run is claimable again: the next answer resumes it.
        let (status, body) = reply("blue, again").await;
        assert_eq!(status, StatusCode::CREATED);
        let answer_seq = body.0["message"]["seq"].as_u64().unwrap();
        let closed = run(&parked_id);
        assert_eq!(closed.state, ocean_core::RoomAgentRunState::Done);
        assert_eq!(closed.answer_seq, Some(answer_seq));
        let runs = wait_for_runs(&state, &key, 2).await;
        assert!(runs.iter().any(|r| r.trigger_seq == answer_seq));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn p3_owner_cancel_of_a_room_turn_ends_the_card_cancelled_not_failed() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let agents_root = tmp.path().join("agents");
        write_agent_fixture(&agents_root, "helper", "model = \"fake-ok\"\n", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents_root);
        let key = RoomKey::new("p3-cancel");
        create_mention_room(&state, &key);
        join_human(&state, &key);
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "helper".into(),
                display_name: "Helper".into(),
                kind: RoomParticipantKind::Agent,
                owner_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let generation = authorize_room_agent_fixture(
            &state,
            &key,
            "helper",
            ActivationPolicy::Mention,
            ContextPolicy::InvocationOnly,
        );
        let (status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: String::new(),
                author_kind: RoomParticipantKind::Human,
                body: "@helper fix the thing".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // The post returns once the turn is admitted and registered; the owner
        // cancels it through the ordinary request route before it can finish.
        let session = core_sid(authorized_room_agent_session_id(&key, "helper", generation));
        let request_id = state
            .requests
            .read()
            .await
            .values()
            .find(|control| control.status.session_id == Some(session))
            .map(|control| control.status.request_id)
            .expect("room turn request");
        let Json(cancelled) = crate::cancel_request(State(state.clone()), Path(request_id)).await;
        assert!(cancelled.ok, "{}", cancelled.message);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let run = loop {
            let runs = with_rooms(&state, |store| store.room_agent_runs(&key, 10)).unwrap();
            if let Some(run) = runs.into_iter().find(|r| r.state.is_terminal()) {
                break run;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the cancelled turn never settled"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(
            state
                .requests
                .read()
                .await
                .get(&request_id)
                .map(|control| control.status.state),
            Some(RequestState::Cancelled),
            "the request itself settled cancelled"
        );
        assert_eq!(
            run.state,
            ocean_core::RoomAgentRunState::Cancelled,
            "{run:?}"
        );
        assert_eq!(run.reply_seq, None, "a cancelled turn posts no reply");
    }

    #[tokio::test]
    async fn p3_runtime_sink_persists_tool_progress_and_keeps_finished_card_terminal() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _env = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("p3-runtime-sink");
        create_mention_room(&state, &key);
        let tracker = Arc::new(Mutex::new(crate::room_agent_runs::RunTracker::start(
            state.clone(),
            key.clone(),
            "helper",
            AgentSessionId::new_v4(),
            1,
            1,
            "/repo".into(),
        )));
        let (sink, events) = mpsc::unbounded_channel();
        let watch = tokio::spawn(crate::room_agent_runs::watch_runtime_events(
            tracker.clone(),
            events,
        ));
        sink.send(ocean_runtime::AgentEvent::ToolExecutionStart {
            session_id: None,
            tool_call_id: "call-1".into(),
            tool_name: "write".into(),
            args: json!({"path": "/repo/src/lib.rs"}),
        })
        .unwrap();
        drop(sink);
        tokio::time::timeout(Duration::from_secs(2), watch)
            .await
            .unwrap()
            .unwrap();
        let run = with_rooms(&state, |store| store.room_agent_runs(&key, 10))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            run.state,
            ocean_core::RoomAgentRunState::RunningTool {
                label: "write src/lib.rs".into()
            }
        );
        assert_eq!(run.tool_count, 1);
        assert_eq!(run.files_changed, vec!["src/lib.rs"]);
        tracker.lock().unwrap().finish_done("Finished", Some(2));
        let (sink, events) = mpsc::unbounded_channel();
        sink.send(ocean_runtime::AgentEvent::TextDelta {
            session_id: None,
            delta: "late".into(),
        })
        .unwrap();
        drop(sink);
        crate::room_agent_runs::watch_runtime_events(tracker, events).await;
        let run = with_rooms(&state, |store| store.room_agent_runs(&key, 10))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(run.state, ocean_core::RoomAgentRunState::Done);
        assert_eq!(run.reply_seq, Some(2));
        assert_eq!(run.tool_count, 1);
    }

    // ── S2-P1: snapshot access, merged SSE via router, outbox/retry ──────────

    use super::super::room_routes;
    use std::time::Duration;

    fn seed_access(state: &AppState, key: &RoomKey, access: RoomAccessProjection) {
        with_rooms(state, |store| {
            store
                .create(key.clone(), key.as_str(), None, Utc::now())
                .expect("room fixture");
            store
                .replace_room_access(key, &access)
                .expect("seed access");
        });
    }

    fn local_access() -> RoomAccessProjection {
        RoomAccessProjection {
            local_member_id: None,
            caller_member_id: None,
            state: RoomAccessState::Local,
            last_confirmed_global_sequence: None,
            members: vec![],
            outbox: vec![],
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_open_room_includes_access() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-snap-open");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/snapshot"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["ok"], json!(true));
        assert!(body["room"].is_object());
        assert_eq!(body["aliases"], json!([]));
        // Local access serializes as {"state":"local"} (skip_serializing_if omits defaults).
        assert_eq!(body["access"], json!({"state": "local"}));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_soft_closed_room_returns_200_with_local_access() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-snap-closed");
        with_rooms(&state, |store| {
            store
                .create(key.clone(), "Closed", None, Utc::now())
                .expect("create");
            store.close(&key).expect("close");
        });

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/snapshot"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["ok"], json!(true));
        assert_eq!(body["access"], json!({"state": "local"}));
        assert_eq!(body["aliases"], json!([]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_get_closed_is_404() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-get-closed");
        with_rooms(&state, |store| {
            store
                .create(key.clone(), "ClosedGet", None, Utc::now())
                .expect("create");
            store.close(&key).expect("close");
        });

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_get_open_includes_access() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-get-open");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["access"], json!({"state": "local"}));
        assert_eq!(body["aliases"], json!([]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_alias_reads_cover_inspect_detail_snapshot_and_store_reopen() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let mut state = fake_convene_state(&tmp);
        let path = tmp.path().join("rooms.db");
        state.rooms = Arc::new(Mutex::new(
            ocean_store::SqliteRoomStore::open(&path).expect("file-backed store"),
        ));
        let key = RoomKey::new("alias-readback");
        with_rooms(&state, |store| {
            store
                .create(key.clone(), "Alias Readback", None, Utc::now())
                .expect("create room");
            store
                .add_participant(
                    &key,
                    RoomParticipant {
                        id: "surface-operator".into(),
                        kind: RoomParticipantKind::Human,
                        display_name: "Operator".into(),
                    },
                    Utc::now(),
                )
                .expect("placeholder participant");
            store
                .add_participant(
                    &key,
                    RoomParticipant {
                        id: "smaths".into(),
                        kind: RoomParticipantKind::Human,
                        display_name: "John".into(),
                    },
                    Utc::now(),
                )
                .expect("successor participant");
            store
                .bootstrap_local_room_agent(
                    &key,
                    "surface-operator",
                    RoomParticipant {
                        id: "room-builder".into(),
                        kind: RoomParticipantKind::Agent,
                        display_name: "Builder".into(),
                    },
                    "room-builder",
                    "test-operator",
                    Utc::now(),
                )
                .expect("local owner");
            store
                .retire_participant(
                    &key,
                    ocean_store::RetireParticipantInput {
                        from_id: "surface-operator".into(),
                        successor_id: "smaths".into(),
                        actor: "test-operator".into(),
                        decision_id: "retire-operator".into(),
                        request_digest: "retire-operator-digest".into(),
                    },
                    Utc::now(),
                )
                .expect("retire placeholder");
        });

        // Reopen the durable DB through a new connection before exercising all
        // read handlers, matching the reconnect path the alias serves.
        state.rooms = Arc::new(Mutex::new(
            ocean_store::SqliteRoomStore::open(&path).expect("reopen store"),
        ));
        // Team-platform P2: local human joins and posts are authored as the
        // daemon owner, so the retired-identity refusal is exercised by a
        // daemon whose owner is the retired placeholder.
        seed_owner(&state, "surface-operator", "Operator");

        let (status, Json(join_error)) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "surface-operator".into(),
                display_name: "Operator".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(join_error["error"].as_str().unwrap().contains("retired"));

        let (status, Json(post_error)) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "surface-operator".into(),
                author_kind: RoomParticipantKind::Human,
                body: "cannot post as retired identity".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(post_error["error"], "author_not_in_roster");

        async fn get_json(state: AppState, uri: String) -> (StatusCode, serde_json::Value) {
            use tower::ServiceExt as _;
            let response = room_routes()
                .with_state(state)
                .oneshot(axum::http::Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            (status, body)
        }

        async fn post_json(
            state: AppState,
            uri: String,
            operator: Option<&str>,
            body: serde_json::Value,
        ) -> (StatusCode, serde_json::Value) {
            use tower::ServiceExt as _;
            let mut request = axum::http::Request::post(uri)
                .header(axum::http::header::CONTENT_TYPE, "application/json");
            if let Some(operator) = operator {
                request = request.header(crate::room_operator::OPERATOR_HEADER, operator);
            }
            let response = room_routes()
                .with_state(state)
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            (status, body)
        }

        for uri in [
            format!("/v1/rooms/persistent/{key}/inspect"),
            format!("/v1/rooms/persistent/{key}"),
            format!("/v1/rooms/persistent/{key}/snapshot"),
        ] {
            let (status, body) = get_json(state.clone(), uri.clone()).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["aliases"].as_array().unwrap().len(), 1);
            assert_eq!(body["aliases_truncated"], false);
            assert_eq!(body["aliases"][0]["from"], "surface-operator");
            assert_eq!(body["aliases"][0]["to"], "smaths");
            assert!(body["aliases"][0]["retired_at"].is_string());
            assert_eq!(body["aliases"][0].as_object().unwrap().len(), 3);
            if uri.ends_with("/inspect") {
                assert_eq!(body["owner"]["member_id"], "smaths");
                assert!(body.get("transcript").is_none());
                assert!(body.get("access").is_none());
            }
        }

        let route_placeholder = "web-18c11f5d551e63f8";
        with_rooms(&state, |store| {
            store
                .add_participant(
                    &key,
                    RoomParticipant {
                        id: route_placeholder.into(),
                        kind: RoomParticipantKind::Human,
                        display_name: "Route Placeholder".into(),
                    },
                    Utc::now(),
                )
                .expect("add route placeholder");
        });
        let retire_uri =
            format!("/v1/rooms/persistent/{key}/participants/{route_placeholder}/retire");
        let retire_body = json!({
            "decision_id": "00000000-0000-4000-8000-000000000001",
            "successor_id": "smaths"
        });
        let (status, unauthorized) =
            post_json(state.clone(), retire_uri.clone(), None, retire_body.clone()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unauthorized["error"], "operator_credential_missing");

        let (status, retired) = post_json(
            state.clone(),
            retire_uri.clone(),
            Some("test-room-operator"),
            retire_body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(retired["changed"], true);
        assert_eq!(retired["alias"]["from"], route_placeholder);
        assert_eq!(retired["alias"]["to"], "smaths");

        let (status, replay) = post_json(
            state.clone(),
            retire_uri,
            Some("test-room-operator"),
            retire_body,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replay["changed"], false);
        assert_eq!(replay["alias"]["from"], route_placeholder);
        assert_eq!(replay["alias"]["to"], "smaths");

        let plain = RoomKey::new("no-aliases");
        with_rooms(&state, |store| {
            store
                .create(plain.clone(), "No Aliases", None, Utc::now())
                .expect("create no-alias room");
        });
        let (status, empty) = get_json(
            state.clone(),
            format!("/v1/rooms/persistent/{plain}/inspect"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(empty["aliases"], json!([]));
        assert_eq!(empty["aliases_truncated"], false);

        let crowded = RoomKey::new("many-aliases");
        with_rooms(&state, |store| {
            store
                .create(crowded.clone(), "Many Aliases", None, Utc::now())
                .expect("create alias-heavy room");
            store
                .add_participant(
                    &crowded,
                    RoomParticipant {
                        id: "smaths".into(),
                        kind: RoomParticipantKind::Human,
                        display_name: "John".into(),
                    },
                    Utc::now(),
                )
                .expect("add successor");
            for index in 0..257 {
                let from_id = format!("web-{index:016x}");
                store
                    .add_participant(
                        &crowded,
                        RoomParticipant {
                            id: from_id.clone(),
                            kind: RoomParticipantKind::Human,
                            display_name: from_id.clone(),
                        },
                        Utc::now(),
                    )
                    .expect("add placeholder");
                store
                    .retire_participant(
                        &crowded,
                        ocean_store::RetireParticipantInput {
                            from_id,
                            successor_id: "smaths".into(),
                            actor: "test-operator".into(),
                            decision_id: format!("retire-{index}"),
                            request_digest: format!("digest-{index}"),
                        },
                        Utc::now(),
                    )
                    .expect("retire placeholder");
            }
        });
        let (status, capped) =
            get_json(state, format!("/v1/rooms/persistent/{crowded}/inspect")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(capped["aliases"].as_array().unwrap().len(), 256);
        assert_eq!(capped["aliases_truncated"], true);
    }

    // ── Merged SSE routed tests ──────────────────────────────────────────────

    /// Read the next SSE frame from a streaming body (non-blocking).
    async fn next_sse_frame(body: &mut Body) -> String {
        use http_body_util::BodyExt as _;
        let frame = tokio::time::timeout(Duration::from_millis(500), body.frame())
            .await
            .expect("frame timeout")
            .expect("SSE body ended")
            .expect("SSE body error");
        let bytes = frame.into_data().unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_initial_frame_is_room_access() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-sse-init");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let mut body = resp.into_body();
        let frame = next_sse_frame(&mut body).await;
        assert!(
            frame.contains("event: room_access"),
            "expected room_access, got: {frame}"
        );
        // No id line on the initial access frame.
        assert!(
            !frame.lines().any(|l| l.starts_with("id:")),
            "initial frame must not carry id"
        );
        let data = frame
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .expect("data line");
        let parsed: serde_json::Value = serde_json::from_str(data).expect("valid JSON");
        assert_eq!(
            parsed,
            json!({"state": "local"}),
            "exact access payload mismatch"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_unknown_room_is_404() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get("/v1/rooms/persistent/nonexistent/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_room_isolation_message_not_leaked() {
        use http_body_util::BodyExt as _;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let room_a = RoomKey::new("s2-iso-a");
        let room_b = RoomKey::new("s2-iso-b");
        seed_access(&state, &room_a, local_access());
        seed_access(&state, &room_b, local_access());

        let app_a = room_routes().with_state(state.clone());
        let resp_a = app_a
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{room_a}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp_a.status(), StatusCode::OK);

        let _ = append_room_message(
            &state,
            &room_b,
            "author",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "only in B",
        );

        let mut body = resp_a.into_body();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
        let mut saw_b_message = false;
        while tokio::time::Instant::now() < deadline {
            let frame = tokio::time::timeout(Duration::from_millis(50), body.frame()).await;
            if let Ok(Some(Ok(bytes))) = frame {
                let data = bytes.into_data().unwrap_or_default();
                let text = String::from_utf8_lossy(&data);
                if text.contains("only in B") {
                    saw_b_message = true;
                    break;
                }
            }
        }
        assert!(!saw_b_message, "room_a stream leaked room_b's message");
    }

    // ── S2-P1 outbox/retry routed tests ──────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_returns_202_on_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-ok");
        seed_access(
            &state,
            &key,
            RoomAccessProjection {
                local_member_id: None,
                caller_member_id: None,
                state: RoomAccessState::Live,
                last_confirmed_global_sequence: Some(1),
                members: vec![],
                outbox: vec![RoomOutboxItem {
                    client_event_id: "evt-1".into(),
                    source_id: "src".into(),
                    source_sequence: 10,
                    author_member_id: "auth".into(),
                    event_type: "chat.message".into(),
                    payload: json!({"text": "hi"}),
                    mention_member_ids: vec![],
                    state: OutboxItemState::Failed,
                }],
            },
        );

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["ok"], json!(true));
        let outbox = body["access"]["outbox"].as_array().unwrap();
        let item = outbox
            .iter()
            .find(|i| i["client_event_id"] == "evt-1")
            .unwrap();
        assert_eq!(item["state"], json!("pending"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_local_room_is_409_not_403() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-local");
        with_rooms(&state, |store| {
            store
                .create(key.clone(), "Local", None, Utc::now())
                .expect("create");
        });

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("room_not_federated"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_malformed_json_is_400() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-mal");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("invalid_retry_request"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_empty_id_is_400() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-empty");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("invalid_retry_request"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_unknown_room_is_404() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post("/v1/rooms/persistent/nonexistent/outbox/retry")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("room_not_found"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_unknown_fields_is_400() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-unk");
        seed_access(&state, &key, local_access());

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-1","extra":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("invalid_retry_request"));
    }

    // ── S2-P1 advanced proofs ────────────────────────────────────────────────

    use crate::tests::fake_convene_file_state;

    fn seed_live_with_failed_projection(client_event_id: &str) -> RoomAccessProjection {
        RoomAccessProjection {
            local_member_id: None,
            caller_member_id: None,
            state: RoomAccessState::Live,
            last_confirmed_global_sequence: Some(1),
            members: vec![],
            outbox: vec![RoomOutboxItem {
                client_event_id: client_event_id.into(),
                source_id: "src".into(),
                source_sequence: 10,
                author_member_id: "auth".into(),
                event_type: "chat.message".into(),
                payload: json!({"text": "hi"}),
                mention_member_ids: vec![],
                state: OutboxItemState::Failed,
            }],
        }
    }

    /// Create a room with Live access and one Failed outbox item.
    fn seed_live_with_failed(state: &AppState, key: &RoomKey, client_event_id: &str) {
        seed_access(
            state,
            key,
            seed_live_with_failed_projection(client_event_id),
        );
    }

    /// Read one SSE event frame from a streaming body.
    async fn read_sse_frame(body: &mut Body) -> SseFrame {
        use http_body_util::BodyExt as _;
        let frame = tokio::time::timeout(Duration::from_millis(500), body.frame())
            .await
            .expect("frame timeout")
            .expect("SSE body ended")
            .expect("SSE body error");
        let text = String::from_utf8_lossy(&frame.into_data().unwrap_or_default()).to_string();
        parse_sse_frame(&text)
    }

    #[derive(Debug)]
    struct SseFrame {
        event: String,
        id: Option<String>,
        data: serde_json::Value,
    }

    fn parse_sse_frame(text: &str) -> SseFrame {
        let event = text
            .lines()
            .find_map(|line| line.strip_prefix("event: "))
            .unwrap_or("")
            .to_string();
        let id = text
            .lines()
            .find_map(|line| line.strip_prefix("id: "))
            .map(str::to_string);
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .map(|d| serde_json::from_str(d).expect("valid JSON"))
            .unwrap_or(serde_json::Value::Null);
        SseFrame { event, id, data }
    }

    async fn read_until_access_frame(body: &mut Body, dur: Duration) -> SseFrame {
        use http_body_util::BodyExt as _;
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for room_access SSE frame"
            );
            let frame = tokio::time::timeout(Duration::from_millis(50), body.frame())
                .await
                .expect("frame timeout")
                .expect("SSE body ended")
                .expect("SSE body error");
            let parsed = parse_sse_frame(&String::from_utf8_lossy(
                &frame.into_data().unwrap_or_default(),
            ));
            if parsed.event == "room_access" {
                return parsed;
            }
            assert_eq!(
                parsed.event, "room_read_cursor",
                "unexpected interleaved SSE event before room_access"
            );
            assert!(
                parsed.id.is_none(),
                "room_read_cursor bootstrap must not consume or alter Last-Event-ID"
            );
        }
    }

    /// Drain the body for `dur` and report whether any `room_access` frame arrived.
    async fn saw_access(body: &mut Body, dur: Duration) -> bool {
        use http_body_util::BodyExt as _;
        let deadline = tokio::time::Instant::now() + dur;
        while tokio::time::Instant::now() < deadline {
            let frame = tokio::time::timeout(Duration::from_millis(50), body.frame()).await;
            if let Ok(Some(Ok(bytes))) = frame {
                let data = bytes.into_data().unwrap_or_default();
                let text = String::from_utf8_lossy(&data);
                if text.contains("event: room_access") {
                    return true;
                }
            }
        }
        false
    }

    // ── Access SSE proofs ─────────────────────────────────────────────────────

    /// (4) Two-subscriber: assert both bodies receive the exact full
    /// committed projection after a retry-triggered wake.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_two_subscribers_both_receive_exact_access_update() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-both-sub");
        seed_live_with_failed(&state, &key, "evt-both");

        let expected_proj = RoomAccessProjection {
            local_member_id: None,
            caller_member_id: None,
            state: RoomAccessState::Live,
            last_confirmed_global_sequence: Some(1),
            members: vec![],
            outbox: vec![RoomOutboxItem {
                client_event_id: "evt-both".into(),
                source_id: "src".into(),
                source_sequence: 10,
                author_member_id: "auth".into(),
                event_type: "chat.message".into(),
                payload: json!({"text": "hi"}),
                mention_member_ids: vec![],
                state: OutboxItemState::Pending,
            }],
        };
        let initial_expected =
            serde_json::to_value(seed_live_with_failed_projection("evt-both")).unwrap();
        let expected = serde_json::to_value(&expected_proj).unwrap();

        let app1 = room_routes().with_state(state.clone());
        let resp1 = app1
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let app2 = room_routes().with_state(state.clone());
        let resp2 = app2
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let mut body1 = resp1.into_body();
        let mut body2 = resp2.into_body();

        // Both subscribers must observe the exact initial access projection, but
        // a no-id cursor bootstrap may interleave first.
        let init1 = read_until_access_frame(&mut body1, Duration::from_millis(500)).await;
        assert_eq!(init1.data, initial_expected);
        let init2 = read_until_access_frame(&mut body2, Duration::from_millis(500)).await;
        assert_eq!(init2.data, initial_expected);

        // Retry triggers access change + wake.
        let app_retry = room_routes().with_state(state.clone());
        let resp = app_retry
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-both"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // Both must receive exactly one follow-up room_access frame; any
        // interleaved bootstrap/read-cursor frames must be no-id and not affect
        // message Last-Event-ID semantics.
        let sub1 = read_until_access_frame(&mut body1, Duration::from_secs(1)).await;
        assert_eq!(sub1.data, expected, "subscriber 1 mismatched");
        let sub2 = read_until_access_frame(&mut body2, Duration::from_secs(1)).await;
        assert_eq!(sub2.data, expected, "subscriber 2 mismatched");
    }

    /// (3) Dedup: same-room unchanged hint produces no access frame. Also proves
    /// room isolation (cross-room wake does not leak).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_access_dedup_and_room_isolation() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let room_a = RoomKey::new("s2-acc-iso-a");
        let room_b = RoomKey::new("s2-acc-iso-b");
        seed_live_with_failed(&state, &room_a, "evt-a");
        seed_live_with_failed(&state, &room_b, "evt-b");

        let app_a = room_routes().with_state(state.clone());
        let resp_a = app_a
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{room_a}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp_a.status(), StatusCode::OK);
        let mut body_a = resp_a.into_body();

        // Consume initial access frame.
        let frame = read_sse_frame(&mut body_a).await;
        assert_eq!(frame.event, "room_access");
        assert_eq!(frame.data["outbox"][0]["client_event_id"], json!("evt-a"));

        // 1) Publish same-room unchanged hint → must produce NO access frame.
        publish_room_access_wake(&state, &room_a);
        assert!(
            !saw_access(&mut body_a, Duration::from_millis(300)).await,
            "same-room unchanged hint produced spurious access frame"
        );

        // 2) Retry on room_b → must NOT deliver access frame to room_a.
        let app_retry = room_routes().with_state(state.clone());
        let resp = app_retry
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{room_b}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-b"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        assert!(
            !saw_access(&mut body_a, Duration::from_millis(300)).await,
            "room_a received access frame from room_b retry"
        );
    }

    /// (1) Lag recovery (direct mpsc, deterministic): pre-subscribe cap-1
    /// receiver, capture initial=0, store 42, overflow BEFORE spawning tail,
    /// spawn with last_access=0, assert mpsc = exact 42, no second frame.
    #[tokio::test]
    async fn merged_sse_access_lag_recovers_durable_no_rescue_hint() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let mut state = fake_convene_state(&tmp);
        state.room_access_wakes = RoomAccessWakeBus::new(1);
        let key = RoomKey::new("s2-acc-lag");

        // Seed initial: seq=0.
        let initial = RoomAccessProjection {
            local_member_id: None,
            caller_member_id: None,
            state: RoomAccessState::Live,
            last_confirmed_global_sequence: Some(0),
            members: vec![],
            outbox: vec![],
        };
        seed_access(&state, &key, initial.clone());

        // Pre-subscribe cap-1 receiver BEFORE spawning the tail.
        let hints = state.room_access_wakes.subscribe();

        // Store durable seq=42.
        with_rooms(&state, |store| {
            store
                .replace_room_access(
                    &key,
                    &RoomAccessProjection {
                        local_member_id: None,
                        caller_member_id: None,
                        state: RoomAccessState::Live,
                        last_confirmed_global_sequence: Some(42),
                        members: vec![],
                        outbox: vec![],
                    },
                )
                .expect("replace");
        });

        // Overflow capacity-1 bus BEFORE spawning tail.
        for _ in 0..10 {
            state.room_access_wakes.publish(&key);
        }

        // Spawn tail with last_access=0 + pre-filled receiver.
        let (tx, mut rx) = mpsc::channel::<RoomAccessProjection>(16);
        tokio::spawn(run_room_access_tail(
            state.clone(),
            key.clone(),
            Some(initial),
            hints,
            tx,
        ));

        // Tail must detect Lagged, re-read durable (seq=42), send it.
        let proj = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert_eq!(
            proj.last_confirmed_global_sequence,
            Some(42),
            "lag did not recover durable seq=42"
        );

        // No second hint.
        let second = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(second.is_err(), "spurious second projection after lag");
    }

    // ── Dual receiver cleanup ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_sse_drop_releases_both_message_and_access_receivers() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-cleanup");
        seed_access(&state, &key, local_access());

        assert_eq!(state.room_wakes.receiver_count(), 0);
        assert_eq!(state.room_access_wakes.receiver_count(), 0);

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(state.room_wakes.receiver_count(), 1);
        // Three independent subscriptions: the access-projection tail, one
        // dedicated to the cursor tail (so an access wake alone can make the
        // cursor tail re-check on a federated Connecting/Recovering -> Live
        // transition; see `run_room_read_cursor_tail`), and the agent-run tail
        // (team-platform P3; see `run_room_agent_run_tail`).
        assert_eq!(state.room_access_wakes.receiver_count(), 3);

        drop(resp);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            if state.room_wakes.receiver_count() == 0
                && state.room_access_wakes.receiver_count() == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            state.room_wakes.receiver_count(),
            0,
            "message receiver not released"
        );
        assert_eq!(
            state.room_access_wakes.receiver_count(),
            0,
            "access receiver not released"
        );
    }

    // ── Retry-outbox advanced matrices ────────────────────────────────────────

    fn seed_pending_outbox(state: &AppState, key: &RoomKey) {
        seed_access(
            state,
            key,
            RoomAccessProjection {
                local_member_id: None,
                caller_member_id: None,
                state: RoomAccessState::Live,
                last_confirmed_global_sequence: Some(1),
                members: vec![],
                outbox: vec![RoomOutboxItem {
                    client_event_id: "evt-pending".into(),
                    source_id: "src".into(),
                    source_sequence: 10,
                    author_member_id: "auth".into(),
                    event_type: "chat.message".into(),
                    payload: json!({"text": "p"}),
                    mention_member_ids: vec![],
                    state: OutboxItemState::Pending,
                }],
            },
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_pending_item_is_409() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-pending");
        seed_pending_outbox(&state, &key);
        let app = room_routes().with_state(state);
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-pending"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("outbox_item_not_failed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_unknown_item_is_404() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-noitem");
        seed_live_with_failed(&state, &key, "evt-known");
        let app = room_routes().with_state(state);
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("outbox_item_not_found"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_revoked_is_403() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-revoked");
        seed_access(
            &state,
            &key,
            RoomAccessProjection {
                local_member_id: None,
                caller_member_id: None,
                state: RoomAccessState::Revoked,
                last_confirmed_global_sequence: Some(1),
                members: vec![],
                outbox: vec![RoomOutboxItem {
                    client_event_id: "evt-rev".into(),
                    source_id: "src".into(),
                    source_sequence: 10,
                    author_member_id: "auth".into(),
                    event_type: "chat.message".into(),
                    payload: json!({"text": "rev"}),
                    mention_member_ids: vec![],
                    state: OutboxItemState::Failed,
                }],
            },
        );
        let app = room_routes().with_state(state);
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-rev"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("room_access_revoked"));
    }

    /// (8) Opaque-id preservation: whitespace-trim rejects empty,
    /// but original nonempty spaced id passes through and matches.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_whitespace_trim_rejects_empty_preserves_opaque() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-ws");
        // Empty trimmed id → 400.
        seed_access(&state, &key, local_access());
        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], json!("invalid_retry_request"));

        // Spaced nonempty id must be preserved and match stored item.
        let key2 = RoomKey::new("s2-retry-opaque");
        seed_live_with_failed(&state, &key2, "  evt-with-spaces  ");
        let app2 = room_routes().with_state(state);
        let resp2 = app2
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key2}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"  evt-with-spaces  "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::ACCEPTED);
        let body2: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp2.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        let item = body2["access"]["outbox"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["client_event_id"] == "  evt-with-spaces  ")
            .unwrap();
        assert_eq!(item["state"], json!("pending"));
    }

    /// (6) Non-object body: table-driven null / array / string / number / bool.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_non_object_bodies_are_400() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-retry-nonobj");
        seed_access(&state, &key, local_access());

        let cases: &[(&str, &str)] = &[
            ("null", "null"),
            ("array", r#"["a","b"]"#),
            ("string", r#""not-an-object""#),
            ("number", "42"),
            ("bool", "true"),
        ];
        for (label, body_str) in cases {
            let app = room_routes().with_state(state.clone());
            let resp = app
                .oneshot(
                    axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                        .header("content-type", "application/json")
                        .body(Body::from(*body_str))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "{label}: expected 400, got {}",
                resp.status()
            );
            let body_json: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                body_json["code"],
                json!("invalid_retry_request"),
                "{label}: wrong code"
            );
        }
    }

    /// (5) Exact 202 envelope: compare whole JSON against expected projection.
    /// Also asserts exactly one access wake on success.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_success_has_exact_202_envelope_and_one_wake() {
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = fake_convene_state(&tmp);
        let key = RoomKey::new("s2-202-env");
        seed_live_with_failed(&state, &key, "evt-env");

        let mut pre_rx = state.room_access_wakes.subscribe();

        let app = room_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_event_id":"evt-env"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        // Build expected from typed projection (matches serde serialization exactly).
        let expected_proj = RoomAccessProjection {
            local_member_id: None,
            caller_member_id: None,
            state: RoomAccessState::Live,
            last_confirmed_global_sequence: Some(1),
            members: vec![],
            outbox: vec![RoomOutboxItem {
                client_event_id: "evt-env".into(),
                source_id: "src".into(),
                source_sequence: 10,
                author_member_id: "auth".into(),
                event_type: "chat.message".into(),
                payload: json!({"text": "hi"}),
                mention_member_ids: vec![],
                state: OutboxItemState::Pending,
            }],
        };
        let expected_access = serde_json::to_value(&expected_proj).unwrap();
        let expected = json!({ "ok": true, "access": expected_access });
        assert_eq!(body, expected, "exact 202 envelope mismatch");

        // Exactly one wake, no second.
        let _ = tokio::time::timeout(Duration::from_millis(500), pre_rx.recv())
            .await
            .expect("first wake timeout")
            .expect("first wake not sent");
        let second = tokio::time::timeout(Duration::from_millis(100), pre_rx.recv()).await;
        assert!(second.is_err(), "woke more than once on single success");
    }

    /// (7) No-wake on every error class: prove zero access hints for
    /// 400 / 403 / 404 / 409 / 500 paths.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_outbox_no_access_wake_any_error_class() {
        use tokio::sync::broadcast::error::TryRecvError;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;

        // --- 400: malformed body ---
        {
            let tmp = tempfile::TempDir::new().unwrap();
            let state = fake_convene_state(&tmp);
            let key = RoomKey::new("s2-nw-400");
            seed_access(&state, &key, local_access());
            let mut rx = state.room_access_wakes.subscribe();
            let _keep = state.clone(); // survives router move
            let app = room_routes().with_state(state);
            let resp = app
                .oneshot(
                    axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                        .header("content-type", "application/json")
                        .body(Body::from("not json"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
            // Only Err(Empty) proves no wake was published.
            assert!(
                matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                "wake on 400"
            );
        }

        // --- 403: revoked ---
        {
            let tmp = tempfile::TempDir::new().unwrap();
            let state = fake_convene_state(&tmp);
            let key = RoomKey::new("s2-nw-403");
            seed_access(
                &state,
                &key,
                RoomAccessProjection {
                    local_member_id: None,
                    caller_member_id: None,
                    state: RoomAccessState::Revoked,
                    last_confirmed_global_sequence: Some(1),
                    members: vec![],
                    outbox: vec![RoomOutboxItem {
                        client_event_id: "evt-403".into(),
                        source_id: "s".into(),
                        source_sequence: 1,
                        author_member_id: "a".into(),
                        event_type: "chat.message".into(),
                        payload: json!({}),
                        mention_member_ids: vec![],
                        state: OutboxItemState::Failed,
                    }],
                },
            );
            let mut rx = state.room_access_wakes.subscribe();
            let _keep = state.clone();
            let app = room_routes().with_state(state);
            let resp = app
                .oneshot(
                    axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"client_event_id":"evt-403"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::FORBIDDEN);
            assert!(
                matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                "wake on 403"
            );
        }

        // --- 404: unknown room ---
        {
            let tmp = tempfile::TempDir::new().unwrap();
            let state = fake_convene_state(&tmp);
            let mut rx = state.room_access_wakes.subscribe();
            let _keep = state.clone();
            let app = room_routes().with_state(state);
            let resp = app
                .oneshot(
                    axum::http::Request::post("/v1/rooms/persistent/nonexistent/outbox/retry")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"client_event_id":"x"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
            assert!(
                matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                "wake on 404"
            );
        }

        // --- 409: local room ---
        {
            let tmp = tempfile::TempDir::new().unwrap();
            let state = fake_convene_state(&tmp);
            let key = RoomKey::new("s2-nw-409");
            with_rooms(&state, |store| {
                store
                    .create(key.clone(), "Local", None, Utc::now())
                    .expect("create");
            });
            let mut rx = state.room_access_wakes.subscribe();
            let _keep = state.clone();
            let app = room_routes().with_state(state);
            let resp = app
                .oneshot(
                    axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"client_event_id":"evt-1"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CONFLICT);
            assert!(
                matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                "wake on 409"
            );
        }

        // --- 500: real store error via rusqlite corruption ---
        {
            let tmp = tempfile::TempDir::new().unwrap();
            let (state, db_path) = fake_convene_file_state(&tmp);
            let key = RoomKey::new("s2-nw-500");
            seed_live_with_failed(&state, &key, "evt-500");
            let mut rx = state.room_access_wakes.subscribe();

            // Corrupt: drop the room_access table via a separate connection.
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch("DROP TABLE IF EXISTS room_access")
                .unwrap();
            conn.close().ok();

            let _keep = state.clone();
            let app = room_routes().with_state(state);
            let resp = app
                .oneshot(
                    axum::http::Request::post(format!("/v1/rooms/persistent/{key}/outbox/retry"))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"client_event_id":"evt-500"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "expected 500 from store error, got {}",
                resp.status()
            );
            // Exact sanitized body: Store errors always produce this fixed message.
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let expected = json!({
                "ok": false,
                "code": "internal_error",
                "error": "internal store error"
            });
            assert_eq!(body, expected, "500 body not exact sanitized form");

            // Zero wake on 500.
            assert!(
                matches!(rx.try_recv(), Err(TryRecvError::Empty)),
                "wake on 500"
            );
        }
    }

    #[test]
    fn room_history_projection_omits_audit_and_transport_metadata() {
        let projected = room_history_row(RoomMessage {
            seq: 7,
            author_id: "builder".into(),
            author_kind: RoomParticipantKind::Agent,
            kind: RoomMessageKind::Message,
            body: "durable fact".into(),
            created_at: Utc::now(),
            federated: Some(ocean_core::FederatedMessageMeta {
                ledger_event_id: "private-ledger-correlation".into(),
                global_sequence: 99,
                source_id: "private-source".into(),
                source_sequence: 12,
                client_event_id: "private-client-event".into(),
                origin_principal_id: "private-principal".into(),
                origin_member_id: "private-member".into(),
            }),
            thread_parent_seq: Some(3),
            session_id: Some("private-session".into()),
            attachment_id: Some("private-attachment".into()),
        });
        assert_eq!(projected.seq, 7);
        assert_eq!(projected.author_id, "builder");
        assert_eq!(
            projected.author_kind,
            ocean_agent::RoomHistoryAuthorKind::Agent
        );
        assert_eq!(projected.text, "durable fact");
    }

    #[test]
    fn human_json_that_resembles_an_audit_is_preserved() {
        let body = r#"{"type":"room.agent.bootstrap","question":"@builder review this"}"#;
        assert_eq!(
            room_history_text(
                body.to_string(),
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
            ),
            body
        );
        assert_eq!(
            room_history_text(
                body.to_string(),
                RoomParticipantKind::System,
                RoomMessageKind::System,
            ),
            "[room agent bootstrap audit]"
        );
    }

    #[test]
    fn profile_and_folder_audits_are_readable_without_exposing_decision_metadata() {
        for (kind, label) in [
            ("room.profile.created", "Room profile created"),
            ("room.profile.updated", "Room profile updated"),
            ("room.resource.granted", "Folder shared"),
            ("room.resource.resumed", "Folder access resumed"),
            ("room.resource.suspended", "Folder access suspended"),
            ("room.resource.revoked", "Folder access revoked"),
        ] {
            let body = json!({"type": kind, "decision_id": "private-decision", "actor": "private-operator", "display_name": "[untrusted](https://example.invalid)"}).to_string();
            assert_eq!(
                room_history_text(
                    body.clone(),
                    RoomParticipantKind::System,
                    RoomMessageKind::System
                ),
                label
            );
            assert_eq!(
                room_history_text(
                    body.clone(),
                    RoomParticipantKind::Human,
                    RoomMessageKind::Message
                ),
                body
            );
            assert_eq!(
                room_history_text(
                    body.clone(),
                    RoomParticipantKind::System,
                    RoomMessageKind::Message
                ),
                body
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn room_tail_close_after_final_catch_up_still_sends_the_marker() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("close-after-catch-up");
        create_plain_room(&state, &key);
        join_human(&state, &key);

        // `paused_tail` releases only after the initial catch-up has captured
        // the room as open. Close in that exact gap before the task interprets
        // the captured state and enters its live wait.
        let (mut tail, release) = paused_tail(&state, &key, None).await;
        let (status, _) = room_close(
            State(state),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("human".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        release.send(()).expect("release tail after close");

        assert_eq!(next_message(&mut tail).await.body, "Human joined");
        assert_eq!(next_message(&mut tail).await.body, "human closed the room");
        let ended = tokio::time::timeout(std::time::Duration::from_secs(1), tail.next())
            .await
            .expect("closed tail must end after its marker");
        assert!(ended.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_a_room_freezes_it_and_says_who_did_it() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("close-route-room");
        seed_owner(&state, "alice", "Alice");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Close Route", None, Utc::now())
        })
        .unwrap();
        let (status, _) = room_join(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomJoinRequest {
                id: "alice".into(),
                display_name: "Alice".into(),
                kind: RoomParticipantKind::Human,
                owner_id: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "alice".into(),
                author_kind: RoomParticipantKind::Human,
                body: "before the close".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let mut running = Vec::new();
        for scope in [
            Some(key.clone()),
            Some(key.clone()),
            Some(RoomKey::new("other-room")),
            None,
        ] {
            let session_id = ocean_core::SessionId::new_v4();
            let mut request = PromptRequest {
                prompt: String::new(),
                images: None,
                request_id: None,
                session_id: Some(session_id),
                create_if_missing: true,
                max_turns: None,
                yolo: false,
                cwd: tmp.path().to_string_lossy().into_owned(),
                project_id: None,
                client_type: Some("room".into()),
                decision_token: None,
            };
            let (id, cancel) = crate::request_control::register_running_request(
                &state.requests,
                &mut request,
                "close regression",
                ocean_core::RequestState::Running,
            )
            .await;
            if let Some(room) = scope.clone() {
                state
                    .requests
                    .write()
                    .await
                    .get_mut(&id)
                    .unwrap()
                    .room_agent_authority = Some(RoomAgentRequestAuthority {
                    room,
                    agent_member_id: "agent".into(),
                    generation: 1,
                    admission_id: "admission".into(),
                    decision_id: "decision".into(),
                    approved_definition_digest: "digest".into(),
                    session_id,
                });
            }
            running.push((id, cancel, scope == Some(key.clone())));
        }

        let (status, body) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "close body: {}", body.0);
        assert_eq!(body.0["ok"], json!(true));
        assert_eq!(body.0["closed"], json!(true));
        for (id, cancel, should_cancel) in running {
            assert_eq!(cancel.is_cancelled(), should_cancel);
            assert_eq!(
                state.requests.read().await[&id].status.state,
                if should_cancel {
                    ocean_core::RequestState::Cancelling
                } else {
                    ocean_core::RequestState::Running
                }
            );
        }

        // 1. Detail 404s. This is the contract `room_get` has always had for a
        //    closed room; until now nothing but a call could produce one.
        let (status, _) = room_get(State(state.clone()), Path(key.as_str().to_string())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 2. The snapshot answers `closed: true` — the signal a hydrating
        //    surface actually reads, since it no longer hydrates through detail.
        let (status, snapshot) = room_snapshot(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Query(TranscriptQuery {
                after_seq: None,
                limit: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot.0["closed"], json!(true));
        // The roster survives the close: a frozen room still says who was in it.
        assert_eq!(
            snapshot.0["room"]["participants"]
                .as_array()
                .map(|p| p.len()),
            Some(1)
        );

        // 3. A send is refused. The store's `room_is_open` guard is what does
        //    it, and the answer is the flat 404 every write to a closed room
        //    gives — not a new code this route would have to teach clients.
        let (status, _) = room_post_message(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Json(RoomMessageRequest {
                author_id: "alice".into(),
                author_kind: RoomParticipantKind::Human,
                body: "after the close".into(),
                thread_parent_seq: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 4. The transcript is still readable, and its LAST row is the marker
        //    naming the closer. A close that froze the room without recording
        //    who did it would pass every assertion above.
        let (status, transcript) = room_transcript(
            State(state.clone()),
            Path(key.as_str().to_string()),
            Query(TranscriptQuery {
                after_seq: None,
                limit: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rows = transcript.0["transcript"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|row| row["body"] == json!("before the close")),
            "the pre-close message must survive: closing is soft ({rows:?})"
        );
        let last = rows.last().unwrap();
        assert_eq!(last["kind"], json!("system"));
        assert_eq!(last["body"], json!("alice closed the room"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_close_leaves_the_room_exactly_as_it_was() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("close-refusal-room");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Close Refusals", None, Utc::now())
        })
        .unwrap();
        // The Agent goes on through the store rather than through `room_join`:
        // that route resolves an Agent name against a real folder-agent
        // definition and there is none here. The roster row is all this test
        // needs — the forged-closer gate reads the participant's KIND.
        with_rooms(&state, |store| {
            for participant in [
                RoomParticipant {
                    id: "alice".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "Alice".into(),
                },
                RoomParticipant {
                    id: "researcher".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Researcher".into(),
                },
            ] {
                store.add_participant(&key, participant, Utc::now())?;
            }
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let rows_before = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .expect("open room")
            .transcript
            .len();

        // Nobody named at all.
        let (status, body) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery { actor_id: None }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.0["code"], json!("invalid_request"));

        // An Agent's identity claimed off the wire. An agent does not close a
        // room, and the marker would have said it did.
        let (status, body) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("researcher".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.0["code"], json!("forged_closer"));

        // On the roster of no room. Refused inside the store's transaction.
        let (status, _) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("stranger".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Still open, and not one extra transcript row from three refusals.
        let record = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .expect("room is still open after three refused closes");
        assert_eq!(record.transcript.len(), rows_before);

        // Now close it for real, then close it again: the second is the same
        // 404 an absent room gets, never a silent success.
        let (status, _) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_a_room_ends_the_message_tail_after_the_marker() {
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _env = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("tail-close-room");
        with_rooms(&state, |store| {
            store.create(key.clone(), "Tail Close", None, Utc::now())
        })
        .unwrap();
        join_participant(&state, &key, "alice", RoomParticipantKind::Human, "Alice");

        let hints = state.room_wakes.subscribe();
        let mut stream = room_message_tail(state.clone(), key.clone(), None, hints, None);
        wait_for_wake_receivers(&state, 1).await;

        let (status, _) = room_close(
            State(state.clone()),
            Path(key.as_str().to_string()),
            HeaderMap::new(),
            Query(CloseRoomQuery {
                actor_id: Some("alice".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // The close marker still arrives — after the join marker the initial
        // catch-up already replayed.
        let joined = next_message(&mut stream).await;
        assert_eq!(joined.body, "Alice joined");
        let marker = next_message(&mut stream).await;
        assert_eq!(marker.body, "alice closed the room");
        // ...and then the stream ENDS, rather than idling on a room that can
        // never produce another row.
        let ended = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("the tail must end, not idle, on a closed room");
        assert!(ended.is_none(), "no rows follow the close marker");
    }

    #[test]
    fn unknown_or_untyped_structured_bodies_never_render_raw() {
        let text = |body: &str| {
            room_history_text(
                body.to_string(),
                RoomParticipantKind::System,
                RoomMessageKind::System,
            )
        };
        assert_eq!(
            text(r#"{"type":"room.future.thing","owner_member_id":"[x](https://evil.co)"}"#),
            "[room audit]"
        );
        assert_eq!(
            text(r#"{"owner_member_id":"[x](https://evil.co)"}"#),
            "[room audit]"
        );
        assert_eq!(
            text("operator smaths closed the room"),
            "operator smaths closed the room"
        );
        assert_eq!(
            text(
                r#"{"type":"room.participant.retired","from":"[x](https://evil.co)","to":"smaths"}"#
            ),
            "Participant retired: ? -> smaths"
        );
    }

    #[test]
    fn author_ids_are_bounded_on_render() {
        assert_eq!(rendered_author_id("smaths".into()), "smaths");
        assert_eq!(rendered_author_id("[click](x)".into()), "[filtered]");
        assert_eq!(rendered_author_id("a\nb".into()), "[filtered]");
        assert_eq!(rendered_author_id("x".repeat(129)), "[filtered]");
        assert_eq!(rendered_author_id("x".repeat(128)), "x".repeat(128));
    }

    #[test]
    fn audit_lines_name_the_agent_and_refusals_carry_their_code() {
        let text = |body: &str| {
            room_history_text(
                body.to_string(),
                RoomParticipantKind::System,
                RoomMessageKind::System,
            )
        };
        assert_eq!(
            text(
                r#"{"type":"room.agent.admission","agent_member_id":"helper","outcome":"allowed","operator_principal_id":"op-private"}"#
            ),
            "[room agent admission audit] helper"
        );
        assert_eq!(
            text(
                r#"{"type":"room.agent.admission","agent_member_id":"helper","outcome":"refused","reason_code":"credential_slot_missing"}"#
            ),
            "[room agent admission refused: credential_slot_missing] helper"
        );
        // A refusal without a code, or with a code that is not an identifier, falls back to the plain label.
        assert_eq!(
            text(
                r#"{"type":"room.agent.admission","agent_member_id":"helper","outcome":"refused"}"#
            ),
            "[room agent admission audit] helper"
        );
        assert_eq!(
            text(
                r#"{"type":"room.agent.admission","agent_member_id":"helper","outcome":"refused","reason_code":"[x](y)"}"#
            ),
            "[room agent admission audit] helper"
        );
        // An agent id that could forge a row is dropped, never rendered.
        assert_eq!(
            text(r#"{"type":"room.agent.output","agent_member_id":"[click](x)"}"#),
            "[room agent output audit]"
        );
        assert_eq!(
            text(r#"{"type":"room.agent.output"}"#),
            "[room agent output audit]"
        );
        assert_eq!(
            text(r#"{"type":"room.participant.retired","from":"surface-operator","to":"smaths"}"#),
            "Participant retired: surface-operator -> smaths"
        );
    }

    async fn admitted_execution_fixture(
        state: &AppState,
        tmp: &tempfile::TempDir,
        key: &RoomKey,
    ) -> (RoomAgentAdmission, tokio::sync::OwnedSemaphorePermit) {
        let agents = tmp.path().join("agents");
        write_agent_fixture(&agents, "helper", "", None);
        std::env::set_var("OCEAN_AGENTS_DIR", &agents);
        create_mention_room(state, key);
        join_human(state, key);
        with_rooms(state, |store| {
            store.add_agent_participant_with_owner(
                key,
                RoomParticipant {
                    id: "helper".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Helper".into(),
                },
                "human",
                Utc::now(),
            )
        })
        .unwrap();
        authorize_room_agent_fixture(
            state,
            key,
            "helper",
            ocean_store::ActivationPolicy::Mention,
            ocean_store::ContextPolicy::InvocationOnly,
        );
        room_agent_authority::admit_room_agent(
            state,
            key,
            "helper",
            "helper",
            AdmissionTrigger::Mention,
        )
        .await
        .unwrap()
    }

    async fn wait_for_room_terminal(state: &AppState, request: Uuid, permits: usize) {
        for _ in 0..200 {
            if state
                .requests
                .read()
                .await
                .get(&request)
                .is_some_and(|r| !r.status.state.is_cancellable())
                && state.turn_limiter.available_permits() == permits
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("Room request did not settle and release its permit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_room_success_releases_capacity_without_capability_clone_custody() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("capacity-normal");
        let total = state.turn_limiter.available_permits();
        let (admission, permit) = admitted_execution_fixture(&state, &tmp, &key).await;
        let cached = admission.clone();
        let callback = RoomOperationAuthority::from_admission(
            &state,
            &cached,
            cached.operation_cancel.clone(),
        );
        assert_eq!(state.turn_limiter.available_permits(), total - 1);
        let trigger = append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            "@helper hello",
        )
        .unwrap();
        let queued = spawn_room_agent_turn(
            state.clone(),
            admission,
            permit,
            RoomParticipant {
                id: "helper".into(),
                kind: RoomParticipantKind::Agent,
                display_name: "Helper".into(),
            },
            trigger.seq,
            None,
            Uuid::new_v4(),
            None,
            None,
        )
        .await
        .unwrap();
        wait_for_room_terminal(&state, queued.request_id, total).await;
        assert_eq!(
            state.requests.read().await[&queued.request_id].status.state,
            RequestState::Completed
        );
        assert!(with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .transcript
            .iter()
            .any(|row| row.author_kind == RoomParticipantKind::Agent
                && row.kind == RoomMessageKind::Message));
        assert_eq!(cached.generation, callback.generation);
        assert!(cached.operation_cancel.is_cancelled());
        use ocean_agent::RoomMemoryAuthority as _;
        assert_eq!(
            callback.authorize_operation(),
            Err(ocean_agent::RoomMemoryAuthorityError::AuthorityChanged)
        );
        // A late capability refusal cannot replace the completed registry fact.
        assert_eq!(
            state.requests.read().await[&queued.request_id].status.state,
            RequestState::Completed
        );
        assert_eq!(state.turn_limiter.available_permits(), total);
    }

    #[tokio::test]
    async fn abort_before_first_poll_restores_room_capacity_and_settles_once() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("capacity-abort");
        let total = state.turn_limiter.available_permits();
        let (admission, permit) = admitted_execution_fixture(&state, &tmp, &key).await;
        let session_id = authorized_room_agent_session_id(&key, "helper", admission.generation);
        let request_id = Uuid::new_v4();
        let cancel = admission.operation_cancel.clone();
        let mut prompt = PromptRequest {
            prompt: "fixture".into(),
            images: None,
            request_id: Some(request_id),
            session_id: Some(core_sid(session_id)),
            create_if_missing: true,
            max_turns: None,
            yolo: false,
            cwd: canonical_test_workspace(tmp.path()),
            project_id: None,
            client_type: Some("room".into()),
            decision_token: None,
        };
        register_room_agent_request_checked(
            &state.requests,
            &mut prompt,
            "abort fixture",
            RoomAgentRequestAuthority {
                room: key.clone(),
                agent_member_id: "helper".into(),
                generation: admission.generation,
                admission_id: admission.admission_id.clone(),
                decision_id: admission.decision_id.clone(),
                approved_definition_digest: admission.package.definition_digest.clone(),
                session_id: core_sid(session_id),
            },
            cancel.clone(),
            || room_agent_authority::append_admission_allow(&state, &admission),
        )
        .await
        .unwrap();
        let before_abort = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .transcript;
        let last_before_abort = before_abort.last().unwrap().seq;
        assert!(
            before_abort
                .iter()
                .any(|row| row.author_kind == RoomParticipantKind::Agent
                    && row.kind == RoomMessageKind::ParticipantJoined),
            "the Agent roster marker is pre-existing"
        );
        let finalizer = RoomTurnFinalizer {
            state: state.clone(),
            admission: admission.clone(),
            request_id,
            session_id,
            cancel: cancel.clone(),
            done: CancellationToken::new(),
            watcher: None,
            armed: true,
        };
        // Single-thread runtime: abort before this newly spawned future can poll.
        let handle = tokio::spawn(async move {
            let _permit = permit;
            let _finalizer = finalizer;
            std::future::pending::<()>().await;
        });
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
        wait_for_room_terminal(&state, request_id, total).await;
        assert!(cancel.is_cancelled());
        assert!(!state.requests.read().await[&request_id]
            .status
            .state
            .is_cancellable());
        let after_abort = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .transcript;
        assert_eq!(
            &after_abort[..before_abort.len()],
            before_abort.as_slice(),
            "abort preserves prior roster/admission rows"
        );
        let new_rows: Vec<_> = after_abort
            .iter()
            .filter(|row| row.seq > last_before_abort)
            .collect();
        assert!(
            !new_rows
                .iter()
                .any(|row| row.author_kind == RoomParticipantKind::Agent),
            "aborted work publishes no agent output"
        );
        assert!(
            !new_rows
                .iter()
                .any(|row| row.body.starts_with("auto-convene failed for")
                    || serde_json::from_str::<Value>(&row.body)
                        .ok()
                        .is_some_and(|body| body["type"] == "room.agent.output")),
            "abort cannot publish normal/error output facts"
        );
        let audit_count = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .transcript
            .iter()
            .filter(|row| row.body.contains("turn_interrupted"))
            .count();
        assert_eq!(audit_count, 1);

        // Real registry terminalizer rejects a duplicate terminal response.
        record_prompt_result(
            &state,
            request_id,
            &interrupted_room_result(request_id, session_id),
            None,
            None,
        )
        .await;
        assert_eq!(state.turn_limiter.available_permits(), total);
        assert_eq!(
            with_rooms(&state, |store| store.get(&key))
                .unwrap()
                .unwrap()
                .transcript
                .iter()
                .filter(|row| row.body.contains("turn_interrupted"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn error_terminal_restores_capacity_and_ends_saved_authority_once() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("capacity-error");
        let total = state.turn_limiter.available_permits();
        let (admission, permit) = admitted_execution_fixture(&state, &tmp, &key).await;
        let cancel = admission.operation_cancel.clone();
        let callback = RoomOperationAuthority::from_admission(&state, &admission, cancel.clone());
        let session_id = authorized_room_agent_session_id(&key, "helper", admission.generation);
        let request_id = Uuid::new_v4();
        let mut prompt = PromptRequest {
            prompt: "fixture".into(),
            images: None,
            request_id: Some(request_id),
            session_id: Some(core_sid(session_id)),
            create_if_missing: true,
            max_turns: None,
            yolo: false,
            cwd: canonical_test_workspace(tmp.path()),
            project_id: None,
            client_type: Some("room".into()),
            decision_token: None,
        };
        register_room_agent_request_checked(
            &state.requests,
            &mut prompt,
            "error fixture",
            RoomAgentRequestAuthority {
                room: key.clone(),
                agent_member_id: "helper".into(),
                generation: admission.generation,
                admission_id: admission.admission_id.clone(),
                decision_id: admission.decision_id.clone(),
                approved_definition_digest: admission.package.definition_digest.clone(),
                session_id: core_sid(session_id),
            },
            cancel.clone(),
            || room_agent_authority::append_admission_allow(&state, &admission),
        )
        .await
        .unwrap();
        {
            let _permit = permit;
            let mut finalizer = RoomTurnFinalizer {
                state: state.clone(),
                admission: admission.clone(),
                request_id,
                session_id,
                cancel: cancel.clone(),
                done: CancellationToken::new(),
                watcher: None,
                armed: true,
            };
            let result = interrupted_room_result(request_id, session_id);
            record_prompt_result(&state, request_id, &result, None, None).await;
            append_authorized_room_agent_failure(&state, &admission, session_id, &cancel).unwrap();
            finalizer.disarm();
            assert!(
                !cancel.is_cancelled(),
                "output settles before ending capability lifetime"
            );
        }
        assert!(cancel.is_cancelled());
        assert_eq!(state.turn_limiter.available_permits(), total);
        assert_eq!(
            state.requests.read().await[&request_id].status.state,
            RequestState::Errored
        );
        use ocean_agent::RoomMemoryAuthority as _;
        assert_eq!(
            callback.authorize_operation(),
            Err(ocean_agent::RoomMemoryAuthorityError::AuthorityChanged)
        );
        record_prompt_result(
            &state,
            request_id,
            &interrupted_room_result(request_id, session_id),
            None,
            None,
        )
        .await;
        assert_eq!(
            state.requests.read().await[&request_id].status.state,
            RequestState::Errored
        );
        let rows = with_rooms(&state, |store| store.get(&key))
            .unwrap()
            .unwrap()
            .transcript;
        assert_eq!(
            rows.iter()
                .filter(|row| row.author_kind == RoomParticipantKind::System
                    && row.kind == RoomMessageKind::System
                    && row.body == "auto-convene failed for helper: turn_failed")
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(
                    |row| serde_json::from_str::<Value>(&row.body)
                        .ok()
                        .is_some_and(|body| body["type"] == "room.agent.output"
                            && body["outcome"] == "failed"
                            && body["admission_id"] == admission.admission_id)
                )
                .count(),
            1
        );
        assert!(!rows
            .iter()
            .any(|row| row.author_kind == RoomParticipantKind::Agent
                && row.kind == RoomMessageKind::Message));
    }

    #[tokio::test]
    async fn registration_refusal_cancels_saved_capabilities_and_restores_capacity() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        for cancelled_first in [true, false] {
            let key = RoomKey::new(format!("capacity-registration-refusal-{cancelled_first}"));
            let total = state.turn_limiter.available_permits();
            let (admission, permit) = admitted_execution_fixture(&state, &tmp, &key).await;
            let saved = admission.clone();
            // Cancellation while waiting for the session lease must be checked at
            // actual registry admission even when the persisted binding is active.
            if cancelled_first {
                saved.operation_cancel.cancel();
            } else {
                with_rooms(&state, |store| {
                    store.set_room_agent_binding_status(
                        &key,
                        "helper",
                        ocean_store::SetAgentBindingStatusInput {
                            status: ocean_store::AgentBindingStatus::Suspended,
                            actor: "fixture-operator".into(),
                            decision_id: "suspend-after-admission".into(),
                            request_digest: "suspend-after-admission-digest".into(),
                        },
                        Utc::now(),
                    )
                })
                .unwrap();
            }
            let request_id = Uuid::new_v4();
            let error = spawn_room_agent_turn(
                state.clone(),
                admission,
                permit,
                RoomParticipant {
                    id: "helper".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Helper".into(),
                },
                0,
                None,
                request_id,
                None,
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code(), "authority_changed_before_registration");
            assert!(saved.operation_cancel.is_cancelled());
            assert_eq!(state.turn_limiter.available_permits(), total);
            assert!(!state.requests.read().await.contains_key(&request_id));
            assert!(!with_rooms(&state, |store| store.get(&key))
                .unwrap()
                .unwrap()
                .transcript
                .iter()
                .any(|row| row.body.contains("\"outcome\":\"admitted\"")));
            use ocean_agent::RoomMemoryAuthority as _;
            let callback = RoomOperationAuthority::from_admission(
                &state,
                &saved,
                saved.operation_cancel.clone(),
            );
            assert_eq!(
                callback.authorize_operation(),
                Err(ocean_agent::RoomMemoryAuthorityError::AuthorityChanged)
            );
        }
    }

    #[tokio::test]
    async fn unresolved_required_slot_refuses_before_registration_optional_slot_does_not() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_FIXTURE_REQUIRED_SLOT",
        ]);
        std::env::remove_var("OCEAN_FIXTURE_REQUIRED_SLOT");
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let total = state.turn_limiter.available_permits();
        for required in [true, false] {
            let key = RoomKey::new(format!("required-slot-{required}"));
            let (baseline, permit) = admitted_execution_fixture(&state, &tmp, &key).await;
            drop(baseline);
            drop(permit);
            with_rooms(&state, |store| {
                store.put_room_profile(
                    &key,
                    ocean_store::PutRoomProfileInput {
                        repos: vec![],
                        tools: vec![],
                        credential_slots: vec![ocean_store::CredentialSlot {
                            name: "MODEL_KEY".into(),
                            purpose: "fixture".into(),
                            required,
                            resolvers: vec!["env:OCEAN_FIXTURE_REQUIRED_SLOT".into()],
                        }],
                        default_resource_id: None,
                        agent_defaults: Default::default(),
                        updated_by: "fixture-operator".into(),
                        decision_id: "profile-fixture".into(),
                        request_digest: "profile-fixture-digest".into(),
                    },
                    Utc::now(),
                )
            })
            .unwrap();
            let result = room_agent_authority::admit_room_agent(
                &state,
                &key,
                "helper",
                "helper",
                AdmissionTrigger::Mention,
            )
            .await;
            if required {
                assert_eq!(result.unwrap_err().code(), "credential_slot_missing");
                assert!(with_rooms(&state, |store| store.get(&key))
                    .unwrap()
                    .unwrap()
                    .transcript
                    .iter()
                    .any(|row| row.body.contains("credential_slot_missing")));
            } else {
                drop(result.unwrap());
            }
            assert!(state.requests.read().await.is_empty());
            assert_eq!(state.turn_limiter.available_permits(), total);
        }
    }

    #[tokio::test]
    async fn create_route_canonicalizes_valid_roots_and_refuses_invalid_without_rows() {
        let _yolo = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let file = tmp.path().join("file");
        std::fs::write(&file, b"fixture").unwrap();
        let app = super::super::room_routes().with_state(state.clone());
        for (index, root) in [
            "relative".to_string(),
            tmp.path().join("missing").to_str().unwrap().to_string(),
            file.to_str().unwrap().to_string(),
        ]
        .into_iter()
        .enumerate()
        {
            let key = format!("invalid-root-{index}");
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::post("/v1/rooms/persistent")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"key":key,"name":"Invalid","workspace_root":root}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 4096)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body, json!({"ok":false,"error":"invalid_workspace_root"}));
            assert!(with_rooms(&state, |store| store.get(&RoomKey::new(&key)))
                .unwrap()
                .is_none());
        }
        for (key, root) in [
            (
                "canonical-root",
                Some(tmp.path().join(".").to_str().unwrap().to_string()),
            ),
            ("null-root", None),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::post("/v1/rooms/persistent")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"key":key,"name":"Valid","workspace_root":root}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let stored = with_rooms(&state, |store| store.get(&RoomKey::new(key)))
                .unwrap()
                .unwrap();
            assert_eq!(
                stored.room.workspace_root,
                root.map(|_| canonical_test_workspace(tmp.path()))
            );
        }
    }

    #[test]
    fn every_store_audit_writer_has_a_render_rule() {
        // Every production source that writes a room audit row. A new
        // ocean-store module is caught by the file-count check below.
        let sources = [
            include_str!("../../ocean-store/src/lib.rs"),
            include_str!("../../ocean-store/src/room_profile.rs"),
            include_str!("../../ocean-store/src/room_resources.rs"),
            include_str!("../../ocean-store/src/room_retirement.rs"),
        ];
        let store_modules =
            std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../ocean-store/src"))
                .expect("ocean-store sources")
                .filter(|entry| {
                    entry
                        .as_ref()
                        .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "rs"))
                })
                .count();
        assert_eq!(
            store_modules,
            sources.len(),
            "ocean-store gained a source file; add it to this scan"
        );
        let mut found = std::collections::BTreeSet::new();
        for source in sources {
            let production = source.split("#[cfg(test)]").next().unwrap_or(source);
            let mut rest = production;
            while let Some(start) = rest.find("\"room.") {
                let tail = &rest[start + 1..];
                let end = tail.find('"').unwrap_or(tail.len());
                let literal = &tail[..end];
                // Every `"room.` literal counts; one the renderer's rules could
                // not name (a hyphen, a digit, uppercase) fails here rather
                // than slipping past the scan.
                found.insert(literal.to_string());
                rest = &tail[end.min(tail.len())..];
            }
        }
        assert!(
            found.len() >= 11,
            "the scan stopped matching the writers: {found:?}"
        );
        for audit_type in &found {
            assert!(
                RENDERED_AUDIT_TYPES.contains(&audit_type.as_str()),
                "ocean-store writes audit type {audit_type} with no render rule in room_history_text"
            );
        }
        for audit_type in RENDERED_AUDIT_TYPES {
            let rendered = room_history_text(
                format!(r#"{{"type":"{audit_type}"}}"#),
                RoomParticipantKind::System,
                RoomMessageKind::System,
            );
            assert_ne!(
                rendered, "[room audit]",
                "{audit_type} has no rule of its own"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audit_rows_reach_every_human_route_projected() {
        use http_body_util::BodyExt as _;

        const POISON_OWNER: &str = "[click here](https://evil.co)";
        const PACKAGE: &str = "pkg-interpolated-only-into-the-audit";
        const PLAIN: &str = "they reverted the map change";

        fn audit_and_plain(
            rows: &[serde_json::Value],
            route: &str,
        ) -> (serde_json::Value, serde_json::Value) {
            let audit = rows
                .iter()
                .find(|row| row["kind"] == "system")
                .unwrap_or_else(|| panic!("{route} dropped the audit row"))
                .clone();
            let plain = rows
                .iter()
                .find(|row| row["kind"] == "message")
                .unwrap_or_else(|| panic!("{route} dropped the human row"))
                .clone();
            (audit, plain)
        }

        fn assert_projected(rows: &[serde_json::Value], route: &str) {
            let (audit, plain) = audit_and_plain(rows, route);
            let body = audit["body"]
                .as_str()
                .unwrap_or_else(|| panic!("{route} audit body is not a string"));
            assert!(
                body.starts_with("[room agent bootstrap audit]"),
                "{route} served it raw: {body}"
            );
            assert!(!body.contains("]("), "{route} kept link syntax: {body}");
            assert!(!body.contains("evil.co"), "{route}: {body}");
            assert!(!body.contains(POISON_OWNER), "{route}: {body}");
            assert!(!body.contains(PACKAGE), "{route}: {body}");
            assert_eq!(plain["body"], PLAIN, "{route} rewrote an ordinary body");
        }

        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
            "OCEAN_AGENTS_DIR",
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
        ]);
        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::tests::isolated_room_fixture_state(&tmp);
        let key = RoomKey::new("audit-projection");
        create_plain_room(&state, &key);
        join_participant(
            &state,
            &key,
            POISON_OWNER,
            RoomParticipantKind::Human,
            "Owner",
        );
        with_rooms(&state, |store| {
            store
                .bootstrap_local_room_agent(
                    &key,
                    POISON_OWNER,
                    RoomParticipant {
                        id: "builder".into(),
                        kind: RoomParticipantKind::Agent,
                        display_name: "Builder".into(),
                    },
                    PACKAGE,
                    "operator:test",
                    Utc::now(),
                )
                .expect("bootstrap writes the audit row");
        });
        append_room_message(
            &state,
            &key,
            "human",
            RoomParticipantKind::Human,
            RoomMessageKind::Message,
            PLAIN,
        )
        .expect("plain message");

        // The ledger is untouched: this slice fixes the read, not the record, and
        // `crates/ocean-store/AGENTS.md` rules the audit rows records-not-prose.
        let stored = with_rooms(&state, |store| store.transcript(&key, None)).expect("transcript");
        assert!(
            stored.iter().any(|message| {
                message.kind == RoomMessageKind::System && message.body.contains(POISON_OWNER)
            }),
            "the store must still hold the audit exactly as it arrived"
        );

        for (route, path) in [
            ("room_get", format!("/v1/rooms/persistent/{key}")),
            (
                "room_transcript",
                format!("/v1/rooms/persistent/{key}/transcript"),
            ),
            (
                "room_snapshot",
                format!("/v1/rooms/persistent/{key}/snapshot"),
            ),
        ] {
            let app = room_routes().with_state(state.clone());
            let response = app
                .oneshot(axum::http::Request::get(&path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{route}");
            let value: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let rows: Vec<serde_json::Value> = value["transcript"]
                .as_array()
                .unwrap_or_else(|| panic!("{route} returned no transcript array"))
                .clone();
            assert_projected(&rows, route);
        }

        // The live tail carries the same projection: replay it off the wire.
        let app = room_routes().with_state(state.clone());
        let response = app
            .oneshot(
                axum::http::Request::get(format!("/v1/rooms/persistent/{key}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let mut wire = String::new();
        for _ in 0..8 {
            if sse_room_messages(&wire)
                .iter()
                .any(|row| row["kind"] == "message")
            {
                break;
            }
            let frame = tokio::time::timeout(std::time::Duration::from_millis(500), body.frame())
                .await
                .expect("SSE frame exceeded 500ms")
                .expect("SSE body ended before the transcript replayed")
                .expect("SSE body error");
            wire.push_str(std::str::from_utf8(frame.data_ref().expect("SSE data frame")).unwrap());
        }
        assert_projected(&sse_room_messages(&wire), "room_events");
    }

    /// Every `RoomMessage` decoded out of a raw SSE wire, in arrival order. The
    /// whole accumulated wire is re-scanned each pass because frames may batch —
    /// one HTTP body frame is not guaranteed to hold exactly one event.
    fn sse_room_messages(wire: &str) -> Vec<serde_json::Value> {
        wire.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok())
            .filter(|value| value.get("seq").is_some())
            .collect()
    }

    #[test]
    fn authorized_room_session_is_generation_bound_and_collision_safe() {
        let room = RoomKey::new("ab");
        let first = authorized_room_agent_session_id(&room, "c", 1);
        assert_eq!(first, authorized_room_agent_session_id(&room, "c", 1));
        assert_ne!(first, authorized_room_agent_session_id(&room, "c", 2));
        assert_ne!(
            first,
            authorized_room_agent_session_id(&RoomKey::new("a"), "bc", 1),
            "length-prefixing must distinguish concatenation-equivalent pairs"
        );
        assert_ne!(
            first,
            room_agent_session_id(&room, "c"),
            "Phase 1 must never resume a legacy room session"
        );
    }

    #[test]
    fn room_history_redacts_structured_audit_without_breaking_backward_cursor() {
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("history-redaction");
        store
            .create(room.clone(), "History Redaction", None, Utc::now())
            .unwrap();
        store
            .authorize_room_agent(
                &room,
                ocean_store::AuthorizeAgentInput {
                    agent_member_id: "builder".into(),
                    agent_package_id: "builder".into(),
                    agent_definition_digest: "sha256:def".into(),
                    agent_definition_revision: None,
                    display_name: "Builder".into(),
                    owner_member_id: "human-1".into(),
                    authorized_by: "operator:test".into(),
                    activation_policy: ocean_store::ActivationPolicy::ExplicitOnly,
                    context_policy: ContextPolicy::RoomHistory,
                    memory_scope: ocean_store::MemoryScope::None,
                    requested_capabilities: Vec::new(),
                    room_capability_grants: Vec::new(),
                    decision_id: "decision-secret".into(),
                    request_digest: "request-secret".into(),
                },
                Utc::now(),
            )
            .unwrap();
        let older = store
            .append_message(
                &room,
                "human-1",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "older user fact",
                Utc::now(),
            )
            .unwrap();
        let audit = store
            .append_message(
                &room,
                "system",
                RoomParticipantKind::System,
                RoomMessageKind::System,
                r#"{"type":"room.agent.bootstrap","operator_principal_id":"operator-private","decision_id":"decision-private","agent_member_id":"builder-private"}"#,
                Utc::now(),
            )
            .unwrap();
        let newer = store
            .append_message(
                &room,
                "human-1",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "newer user fact",
                Utc::now(),
            )
            .unwrap();

        let first = store
            .authorized_room_history_page(&room, "builder", 1, None, 2)
            .unwrap();
        assert!(first.has_more);
        let projected = first
            .messages
            .into_iter()
            .map(room_history_row)
            .collect::<Vec<_>>();
        assert_eq!(projected[0].seq, newer.seq);
        assert_eq!(projected[1].seq, audit.seq);
        assert!(
            projected[1]
                .text
                .starts_with("[room agent bootstrap audit]"),
            "{}",
            projected[1].text
        );
        assert!(!projected[1].text.contains("operator-private"));
        assert!(!projected[1].text.contains("decision-private"));
        // S0: the agent's roster id is public and rides after the label.
        assert_eq!(
            projected[1].text,
            "[room agent bootstrap audit] builder-private"
        );

        let second = store
            .authorized_room_history_page(&room, "builder", 1, Some(projected[1].seq), 2)
            .unwrap();
        assert!(second
            .messages
            .iter()
            .any(|message| message.seq == older.seq && message.body == "older user fact"));
    }
}
