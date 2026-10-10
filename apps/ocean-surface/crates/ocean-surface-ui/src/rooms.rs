//! Persistent Rooms panel — list, create, join/leave, transcript + composer.
//!
//! The web counterpart to the daemon's persistent-rooms surface (OCEAN-65,
//! routes under `/v1/rooms/persistent/*`). A room is a durable, named
//! collaboration space with a participant roster and an append-only transcript.
//! This module owns:
//!
//!   GET    /v1/rooms/persistent                       → list rooms
//!   POST   /v1/rooms/persistent                       → create a room
//!   GET    /v1/rooms/persistent/{key}                 → room + transcript
//!   POST   /v1/rooms/persistent/{key}/participants    → join
//!   DELETE /v1/rooms/persistent/{key}/participants/{id}→ leave
//!   POST   /v1/rooms/persistent/{key}/messages        → post a message
//!   GET    /v1/rooms/persistent/{key}/events           → live SSE tail (TASK-10)
//!
//! Live updates: the daemon's room-scoped SSE (TASK-10, `GET
//! /v1/rooms/persistent/{key}/events`) streams every transcript row as a
//! `room_message` frame with `id:=seq`. The surface hydrates once, then tails
//! live with sequence resume (`?after_seq=` on each newly constructed browser
//! connection) — no poll, no global-stream workaround (TASK-11).
//!
//! The whole module is self-contained — it carries its own request layer rather
//! than threading rooms state through the `Daemon` handle — so it never touches
//! the live agent loop / session SSE code.

use std::collections::HashMap;

use futures_util::future::Either;
use futures_util::StreamExt;
use gloo_net::eventsource::futures::EventSource;
use gloo_net::http::Request;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

/// SSE tail connection state for the live indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailState {
    /// Initial catch-up replay in progress.
    Replaying,
    /// Live stream connected, receiving frames in real time.
    Live,
    /// Connection dropped, attempting reconnect.
    Reconnecting,
}

// ---- Wire types (mirror ocean-core Room / RoomMessage / RoomParticipant) ----

/// What kind of actor a participant / message author is. Mirrors
/// `ocean_core::RoomParticipantKind` (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomParticipantKind {
    Human,
    Agent,
    Bot,
    Tool,
    System,
}

impl RoomParticipantKind {
    /// The author/roster chip mark — hand-drawn SVGs from `icons.rs`, same
    /// stroke family as the rest of the surface (emoji glyphs are forbidden
    /// in product UI; the 07-08 purge missed this path — QA-006).
    #[allow(dead_code)]
    fn icon(self) -> AnyView {
        match self {
            RoomParticipantKind::Human => view! { <crate::icons::Person /> }.into_any(),
            RoomParticipantKind::Agent => view! { <crate::icons::Robot /> }.into_any(),
            RoomParticipantKind::Bot => view! { <crate::icons::Cog /> }.into_any(),
            RoomParticipantKind::Tool => view! { <crate::icons::Wrench /> }.into_any(),
            RoomParticipantKind::System => view! { <crate::icons::Spark /> }.into_any(),
        }
    }

    /// A lowercase word for the kind — shown next to the icon so the roster makes
    /// it explicit who's an agent (i.e. auto-convene-able) vs. a human.
    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            RoomParticipantKind::Human => "human",
            RoomParticipantKind::Agent => "agent",
            RoomParticipantKind::Bot => "bot",
            RoomParticipantKind::Tool => "tool",
            RoomParticipantKind::System => "system",
        }
    }
}

/// One participant in a room's roster. Mirrors `ocean_core::RoomParticipant`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomParticipant {
    pub id: String,
    pub kind: RoomParticipantKind,
    pub display_name: String,
}

/// What kind of transcript entry a message is. Mirrors
/// `ocean_core::RoomMessageKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMessageKind {
    Message,
    ParticipantJoined,
    ParticipantLeft,
    System,
}

/// One transcript entry. Mirrors `ocean_core::RoomMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomMessage {
    pub seq: u64,
    pub author_id: String,
    pub author_kind: RoomParticipantKind,
    pub kind: RoomMessageKind,
    pub body: String,
    #[serde(default)]
    pub created_at: String,
    /// Confirmed-federation metadata. `None` for local-only rooms and G1
    /// messages. Present only after Bedrock confirms.
    #[serde(default)]
    pub federated: Option<FederatedMessageMeta>,
    /// Root message sequence for a one-level thread reply. `None` for roots.
    #[serde(default)]
    pub thread_parent_seq: Option<u64>,
}

// ---- Federated wire types (exact mirror of ocean-core 786c6ba4) -------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedMessageMeta {
    pub ledger_event_id: String,
    pub global_sequence: u64,
    pub source_id: String,
    pub source_sequence: u64,
    pub client_event_id: String,
    pub origin_principal_id: String,
    pub origin_member_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedRoomMemberProjection {
    pub member_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_member_id: Option<String>,
    pub actor_type: FederatedActorType,
    pub role_in_room: FederatedRoomRole,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_agent_descriptor: Option<PublicAgentDescriptor>,
    pub joined_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_presence: Option<MemberPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_binding_available: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederatedActorType {
    User,
    Agent,
}

impl FederatedActorType {
    #[allow(dead_code)]
    fn icon(self) -> AnyView {
        match self {
            Self::User => view! { <crate::icons::Person /> }.into_any(),
            Self::Agent => view! { <crate::icons::Robot /> }.into_any(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederatedRoomRole {
    Owner,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberPresence {
    Live,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicAgentDescriptor {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_alias: Option<String>,
    #[serde(default)]
    pub skills_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomReadCursorProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirrored_upstream_read_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomReadSummary {
    pub latest_seq: Option<u64>,
    pub read_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomOutboxItem {
    pub client_event_id: String,
    pub source_id: String,
    pub source_sequence: u64,
    pub author_member_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mention_member_ids: Vec<String>,
    pub state: OutboxItemState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxItemState {
    Pending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAccessProjection {
    pub state: RoomAccessState,
    /// Daemon credential's caller identity; never inferred from browser storage.
    #[serde(
        default,
        alias = "self_member_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub caller_member_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_confirmed_global_sequence: Option<u64>,
    /// The Bedrock member id this daemon's owner speaks as in a federated
    /// room. `None` for Local rooms and before a credential exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_member_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<FederatedRoomMemberProjection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outbox: Vec<RoomOutboxItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomAccessState {
    Local,
    Connecting,
    Live,
    Recovering,
    Revoked,
}
/// Owner-minted, single-use room invitation. The code is intentionally exposed
/// only by the explicit invite response so the surface can build a share link.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomInvite {
    pub code: String,
    pub expires_at: String,
    pub room_key: String,
    pub room_name: String,
}
/// Unredeemed native invite held only in local UI memory until the operator
/// explicitly joins or dismisses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRoomInvite {
    pub room_key: String,
    pub code: String,
}

#[derive(Clone, PartialEq, Eq)]
struct PendingInviteRedemption {
    invite: PendingRoomInvite,
    intent_revision: u64,
    room_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadCursorRequest {
    ticket: u64,
    read_seq: u64,
}

/// How a room's agents are auto-woken. Mirrors `ocean_core::RoomTriggerPolicy`.
/// All flags default off; the daemon reads this on `room_create` and evaluates
/// it on every non-agent-authored message (OCEAN-65 / OCEAN-111).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoomTriggerPolicy {
    /// Wake an agent when it is @-mentioned in the transcript (the common case).
    #[serde(default)]
    pub on_mention: bool,
    /// Wake an agent when someone replies in a thread it participates in.
    #[serde(default)]
    pub on_thread_reply: bool,
    /// Wake an agent when a rendered component emits an interaction event.
    #[serde(default)]
    pub on_component_event: bool,
    /// Optional cron expression for scheduled wake-ups. `None`/empty = no schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_schedule: Option<String>,
}

/// A persistent room. Mirrors `ocean_core::Room` (we read only the fields the
/// panel renders).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Room {
    /// The room key. `ocean_core::RoomKey` serializes as a bare string
    /// (`pub struct RoomKey(pub String)`), so this deserializes directly.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub participants: Vec<RoomParticipant>,
    #[serde(default)]
    pub created_at: String,
    /// Last change to roster/metadata/transcript — shown as "last activity".
    #[serde(default)]
    pub updated_at: String,
    /// Optional auto-convene trigger policy. `None` = no automatic triggers.
    #[serde(default)]
    pub trigger_policy: Option<RoomTriggerPolicy>,
    /// This Ocean's per-room mute pref (team-platform P6; owner-local).
    #[serde(default)]
    pub muted: bool,
}

// ---- Response envelopes (the daemon's `json!({ "ok": .., .. })` shapes) ------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RoomsListResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    rooms: Vec<Room>,
    #[serde(default)]
    read_states: Vec<RoomReadStateWire>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RoomReadStateWire {
    room_id: String,
    #[serde(default)]
    latest_seq: Option<String>,
    #[serde(default)]
    read_seq: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ReadCursorPatchBody {
    read_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ReadCursorPatchEnvelope {
    #[serde(default)]
    ok: bool,
    cursor: RoomReadCursorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RoomReadCursorBody {
    room_id: String,
    #[serde(default)]
    read_seq: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadCursorProjectionTarget {
    Local,
    MirroredUpstream,
}

#[derive(Debug, Clone, Deserialize)]
struct RoomGetResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    room: Option<Room>,
    #[serde(default)]
    transcript: Vec<RoomMessage>,
    /// Required on every successful room open, including local rooms.
    access: RoomAccessProjection,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoomErrorResponse {
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoomMutateResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    room: Option<Room>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TranscriptResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    transcript: Vec<RoomMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct RedeemInviteResponse {
    room_key: String,
}

// ---- Request bodies (match the daemon's serde::Deserialize structs) ----------

#[derive(Debug, Clone, Serialize)]
struct CreateRoomBody<'a> {
    key: &'a str,
    name: &'a str,
    /// Optional trigger policy. Skipped when `None` so the daemon's `#[serde(default)]`
    /// (no triggers) applies; otherwise the daemon stores it verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_policy: Option<RoomTriggerPolicy>,
}

/// A human join carries no identity: the daemon joins its owner.
#[derive(Debug, Clone, Serialize)]
struct JoinBody {
    kind: RoomParticipantKind,
}

/// An agent join names the daemon-owned folder agent to add.
#[derive(Debug, Clone, Serialize)]
struct AgentJoinBody<'a> {
    id: &'a str,
    display_name: &'a str,
    kind: RoomParticipantKind,
}

/// A human post carries no author id: the daemon authors it as its owner.
#[derive(Debug, Clone, Serialize)]
struct PostMessageBody<'a> {
    author_kind: RoomParticipantKind,
    body: &'a str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_parent_seq: Option<u64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
struct RetryOutboxBody<'a> {
    client_event_id: &'a str,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct RetryOutboxSuccess {
    ok: bool,
    access: RoomAccessProjection,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct RetryOutboxErrorResponse {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// The daemon owner this surface speaks for (`GET /v1/me`). Every surface
/// talking to one daemon is the same person, so identity is daemon-owned and
/// never minted in the browser (team-platform P2).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OwnerIdentity {
    pub participant_id: String,
    pub display_name: String,
}

/// Outcome of a typed create-room operation. Each outcome carries the
/// request it resolves, so surfaces can gate only the matching attempt
/// and leave concurrent submits untouched.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateOutcome {
    /// Room created successfully (key).
    Success { key: String },
    /// Daemon rejected as a duplicate.
    Duplicate,
    /// Daemon rejected for another reason, or client-side reject
    /// (empty name, encode error, network failure).
    Failed { error: String },
}

/// Resolved action for a create-room dispatch — what the surface
/// Effect should do after inspecting the op-id slot.
#[derive(Debug, PartialEq)]
pub enum CreateResolution {
    /// Creation succeeded — clear the draft.
    Success,
    /// Creation failed or was rejected — keep the draft for retry.
    KeepDraft,
    /// No outcome yet (in flight) or op_id belongs to another attempt.
    Pending,
}

/// CAS admission guard for the create-room result channel.
/// Simple op-id comparison — only the dispatch that currently owns
/// the slot may publish status and select its room.
pub struct CasAdmission;

impl CasAdmission {
    /// True when `slot_op` matches `my_op` — the completion is current.
    pub fn admit(slot_op: u64, my_op: u64) -> bool {
        slot_op == my_op
    }
}

/// Reactive handle for the rooms panel. Holds the room list, the open room +
/// its transcript, status text, and the SSE-tail generation counter. Cloned
/// freely (all fields are `Copy` signal handles), like [`crate::daemon::Daemon`].
#[derive(Clone, Copy)]
pub struct Rooms {
    /// Daemon base URL signal, shared with `Daemon::url` so requests follow the
    /// origin learned at bootstrap (phone-via-tunnel resolves it asynchronously,
    /// so we must read it live at request time, not snapshot it at construction).
    pub url: RwSignal<String>,
    /// All persistent rooms (from `GET /v1/rooms/persistent`).
    pub list: RwSignal<Vec<Room>>,
    /// Whether the first `fetch_rooms` has resolved (success or failure). Starts
    /// false so the panel shows a loading placeholder instead of falsely
    /// asserting "No rooms yet" during the initial in-flight fetch.
    pub rooms_loaded: RwSignal<bool>,
    /// Whether the latest room-list request is still in flight.
    pub rooms_loading: RwSignal<bool>,
    /// Error from the latest room-list request, if that request failed.
    pub rooms_error: RwSignal<Option<String>>,
    /// Monotonic ticket ensuring only the latest overlapping list request may
    /// publish list/loading/error state.
    list_request_ticket: RwSignal<u64>,
    /// The currently selected room key, if any.
    pub open_key: RwSignal<Option<String>>,
    /// The open room's full record (roster + metadata).
    pub open_room: RwSignal<Option<Room>>,
    /// The open room's transcript, ascending by `seq`.
    pub transcript: RwSignal<Vec<RoomMessage>>,
    /// The open room's agent work cards (team-platform P3), start-ordered.
    pub runs: RwSignal<Vec<RoomAgentRun>>,
    /// Free-form status line (errors, in-flight notices).
    pub status: RwSignal<String>,
    /// Monotonic generation: bumped when the open room changes so a stale
    /// poll/SSE loop retires instead of writing into the wrong room.
    generation: RwSignal<u64>,
    /// The daemon owner's participant id (`GET /v1/me`); empty until loaded.
    /// Local rooms render "me" by it; joins and posts never send it as
    /// authority — the daemon derives authorship itself.
    pub identity_id: RwSignal<&'static str>,
    /// The daemon owner's display name; empty until loaded.
    pub identity_name: RwSignal<&'static str>,
    /// Latest owner read owns publication; a daemon-origin change retires it.
    owner_request_ticket: RwSignal<u64>,
    owner_origin: RwSignal<Option<String>>,
    /// Tail state for the live connection indicator. Starts as Replaying during
    /// initial catch-up, switches to Live once connected, and to Reconnecting on
    /// drop/retry. The view reads this to render the status bar indicator.
    tail_state: RwSignal<TailState>,
    /// Available agent summaries fetched from GET /v1/agents (TASK-9/TASK-11).
    pub available_agents: RwSignal<Vec<AgentSummary>>,
    /// The daemon's agents root as reported by `/v1/agents`, so the picker's
    /// empty state can tell the operator exactly where to drop an agent folder.
    pub agents_root: RwSignal<Option<String>>,
    /// Agent name currently being added to the open room (`None` = idle).
    /// Gates the picker so a double-click can't double-register.
    pub add_agent_in_flight: RwSignal<Option<String>>,
    /// Whether the first `fetch_agents` has resolved. Starts false so the
    /// add-agent picker shows nothing rather than a premature "No agents" while
    /// the initial `/v1/agents` fetch is still in flight (same flash class as
    /// `rooms_loaded`).
    pub agents_loaded: RwSignal<bool>,
    /// Required access projection for the open room. `None` means loading or
    /// no room is open; local rooms carry `Some(state = Local)`.
    pub access: RwSignal<Option<RoomAccessProjection>>,
    /// Per-room durable unread summary from the daemon room list.
    pub read_summaries: RwSignal<HashMap<String, RoomReadSummary>>,
    /// Durable read cursor for the currently open room.
    pub open_read_cursor: RwSignal<Option<RoomReadCursorProjection>>,
    /// The latest PATCH /read-cursor, including ownership of its completion.
    read_cursor_in_flight: RwSignal<Option<ReadCursorRequest>>,
    read_cursor_request_ticket: RwSignal<u64>,
    /// Surfaces snapshot the op_id before dispatching and gate only on the
    /// outcome carrying a matching id — concurrent submits never cross-resolve.
    pub create_op: RwSignal<(u64, Option<CreateOutcome>)>,
    /// Most recently minted invite for the currently open room.
    pub invite: RwSignal<Option<RoomInvite>>,
    /// Invite creation lifecycle for the room header affordance.
    pub invite_loading: RwSignal<bool>,
    /// Sanitized invite creation failure; never contains the invite code.
    pub invite_error: RwSignal<Option<String>>,
    /// Native invite awaiting explicit operator confirmation.
    pub pending_invite: RwSignal<Option<PendingRoomInvite>>,
    /// Thread root to open once the target room's transcript has loaded
    /// (team-platform P6: picking an inbox item).
    pub focus_thread: RwSignal<Option<(String, String, u64)>>,
    pending_invite_revision: RwSignal<u64>,
    pending_invite_redemption: RwSignal<Option<PendingInviteRedemption>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomsFetchMode {
    Interactive,
    Silent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoomsListSuccess {
    rooms: Vec<Room>,
    read_summaries: HashMap<String, RoomReadSummary>,
}

impl Rooms {
    /// Construct a rooms handle that shares the live `Daemon::url` signal, so it
    /// always targets the origin resolved by bootstrap. Room collaboration is
    /// daemon-native text; LiveKit state is intentionally outside this type.
    pub fn new(daemon: &crate::daemon::Daemon) -> Self {
        Self::with_url(daemon.url)
    }

    fn with_url(url: RwSignal<String>) -> Self {
        Self {
            url,
            list: RwSignal::new(Vec::new()),
            rooms_loaded: RwSignal::new(false),
            rooms_loading: RwSignal::new(false),
            rooms_error: RwSignal::new(None),
            list_request_ticket: RwSignal::new(0),
            open_key: RwSignal::new(None),
            open_room: RwSignal::new(None),
            transcript: RwSignal::new(Vec::new()),
            runs: RwSignal::new(Vec::new()),
            status: RwSignal::new(String::new()),
            generation: RwSignal::new(0),
            identity_id: RwSignal::new(""),
            identity_name: RwSignal::new(""),
            owner_request_ticket: RwSignal::new(0),
            owner_origin: RwSignal::new(None),
            tail_state: RwSignal::new(TailState::Replaying),
            available_agents: RwSignal::new(Vec::new()),
            agents_root: RwSignal::new(None),
            add_agent_in_flight: RwSignal::new(None),
            agents_loaded: RwSignal::new(false),
            access: RwSignal::new(None),
            read_summaries: RwSignal::new(HashMap::new()),
            open_read_cursor: RwSignal::new(None),
            read_cursor_in_flight: RwSignal::new(None),
            read_cursor_request_ticket: RwSignal::new(0),
            create_op: RwSignal::new((0, None)),
            invite: RwSignal::new(None),
            invite_loading: RwSignal::new(false),
            invite_error: RwSignal::new(None),
            pending_invite: RwSignal::new(None),
            focus_thread: RwSignal::new(None),
            pending_invite_revision: RwSignal::new(0),
            pending_invite_redemption: RwSignal::new(None),
        }
    }

    fn base(&self) -> String {
        self.url.get_untracked().trim_end_matches('/').to_string()
    }
    /// Reactive projection used by the workspace to suppress its empty state
    /// until snapshot replay has reached the live room tail.
    pub fn transcript_tail_is_live(&self) -> bool {
        self.tail_state.get() == TailState::Live
    }

    /// Current-room predicate over the live `generation` and `open_key`
    /// signals. Async tail work checks it before any non-frame state write;
    /// decoded frames pass through `accept_room_tail_frame` below.
    ///
    /// `pub(crate)` so sibling modules (`rooms_workspace/`) holding a
    /// previously-captured `(generation, key)` pair — e.g. a pending
    /// read-advance request built while a room was open — can re-validate it
    /// before dispatching a mutating request. A same-key close/reopen bumps
    /// `generation`, so a stale pair is rejected even though the key still
    /// matches the newly-reopened room.
    pub(crate) fn room_is_current(&self, generation_id: u64, key: &str) -> bool {
        room_request_is_current(
            generation_id,
            self.generation.get_untracked(),
            key,
            self.open_key.get_untracked().as_deref(),
        )
    }

    /// `pub(crate)` snapshot of the live room-identity generation counter —
    /// bumped by every `open_room`/`close_room`. Exposed so a caller building
    /// state that outlives one render (e.g. `ReadAdvanceRequest`) can stamp it
    /// with "as of which room admission" it was computed, then re-validate
    /// via `room_is_current` before acting on it later.
    pub(crate) fn generation_snapshot(&self) -> u64 {
        self.generation.get_untracked()
    }

    pub(crate) fn take_pending_thread_focus(&self) -> Option<u64> {
        let (origin, key, root) = self.focus_thread.get()?;
        if self.url.get() != origin {
            self.focus_thread.set(None);
            return None;
        }
        if self.open_key.get().as_deref() != Some(key.as_str())
            || self.open_room.get().as_ref().map(|room| room.id.as_str()) != Some(key.as_str())
            || !self
                .transcript
                .with(|rows| rows.iter().any(|row| row.seq == root))
        {
            return None;
        }
        self.focus_thread.set(None);
        Some(root)
    }

    /// Synchronously clear the open-room signals and pin `tail_state` to
    /// `Replaying` so no prior room state leaks into the next open. Shared
    /// by `open_room` (pre-hydrate) and `close_room`.
    fn reset_room_state(&self) {
        self.focus_thread.set(None);
        self.open_room.set(None);
        self.transcript.set(Vec::new());
        self.runs.set(Vec::new());
        self.access.set(None);
        self.open_read_cursor.set(None);
        self.read_cursor_in_flight.set(None);
        self.tail_state.set(TailState::Replaying);
        self.invite.set(None);
        self.invite_loading.set(false);
        self.invite_error.set(None);
        self.pending_invite_redemption.set(None);
    }

    /// Whether the current identity is joined according to the room's explicit
    /// access authority. Local rooms use the daemon-native roster; every
    /// non-local state uses only the safe access-member projection.
    pub fn joined_open(&self) -> bool {
        joined_open_for(
            self.access.get().as_ref(),
            self.open_room.get().as_ref(),
            self.identity_id.get(),
        )
    }

    /// Fetch the room list (`GET /v1/rooms/persistent`). Overlapping requests
    /// are latest-wins: an older completion cannot publish any list lifecycle
    /// state after a newer request has started.
    pub fn fetch_rooms(&self) {
        self.fetch_rooms_with_mode(RoomsFetchMode::Interactive);
    }

    pub fn fetch_rooms_silent(&self) {
        self.fetch_rooms_with_mode(RoomsFetchMode::Silent);
    }

    fn fetch_rooms_with_mode(&self, mode: RoomsFetchMode) {
        if should_skip_rooms_fetch(mode, self.rooms_loading.get_untracked()) {
            return;
        }
        let base = self.base();
        let me = *self;
        let ticket = self.list_request_ticket.get_untracked().wrapping_add(1);
        self.list_request_ticket.set(ticket);
        if matches!(mode, RoomsFetchMode::Interactive) {
            self.rooms_loading.set(true);
            self.rooms_error.set(None);
        }
        spawn_local(async move {
            let get_url = format!("{base}/v1/rooms/persistent");
            let result = match Request::get(&get_url).send().await {
                Ok(resp) => match resp.json::<RoomsListResponse>().await {
                    Ok(r) if r.ok => match read_summaries_from_wire(&r.read_states) {
                        Ok(read_summaries) => Ok(RoomsListSuccess {
                            rooms: r.rooms,
                            read_summaries,
                        }),
                        Err(error) => Err(format!("rooms decode error: {error}")),
                    },
                    Ok(r) => Err(format!(
                        "rooms list failed: {}",
                        r.error.unwrap_or_else(|| "unknown error".into())
                    )),
                    Err(err) => Err(format!("rooms decode error: {err}")),
                },
                Err(err) => Err(format!("rooms fetch error: {err}")),
            };
            let is_current =
                list_request_is_current(ticket, me.list_request_ticket.get_untracked());
            finish_rooms_fetch(&me.rooms_loaded, &me.rooms_loading, mode, is_current);
            if !is_current {
                return;
            }
            match result {
                Ok(success) => {
                    // The daemon is reachable at this origin; load the owner
                    // identity if the mount-time fetch raced URL bootstrap.
                    if me.identity_id.get_untracked().is_empty() {
                        me.fetch_me();
                    }
                    me.list.set(success.rooms.clone());
                    me.read_summaries.update(|current| {
                        *current = merge_room_read_summaries(
                            current,
                            &success.rooms,
                            &success.read_summaries,
                        );
                    });
                    me.rooms_error.set(None);
                }
                Err(error) => {
                    if matches!(mode, RoomsFetchMode::Interactive) {
                        me.status.set(error.clone());
                        me.rooms_error.set(Some(error));
                    }
                }
            }
        });
    }

    fn begin_owner_request(&self) -> (String, u64) {
        let base = self.base();
        if self.owner_origin.get_untracked().as_deref() != Some(base.as_str()) {
            self.identity_id.set("");
            self.identity_name.set("");
            self.owner_origin.set(Some(base.clone()));
        }
        let ticket = self.owner_request_ticket.get_untracked().wrapping_add(1);
        self.owner_request_ticket.set(ticket);
        (base, ticket)
    }

    fn apply_owner_response(&self, base: &str, ticket: u64, owner: OwnerIdentity) -> bool {
        if self.base() != base || self.owner_request_ticket.get_untracked() != ticket {
            return false;
        }
        if self.identity_id.get_untracked() != owner.participant_id {
            self.identity_id
                .set(Box::leak(owner.participant_id.into_boxed_str()));
        }
        if self.identity_name.get_untracked() != owner.display_name {
            self.identity_name
                .set(Box::leak(owner.display_name.into_boxed_str()));
        }
        true
    }

    /// Load the daemon owner identity (`GET /v1/me`). The strings are leaked
    /// once per distinct identity so request closures keep `&'static str`
    /// signals; an unchanged identity is not re-leaked.
    pub fn fetch_me(&self) {
        let (base, ticket) = self.begin_owner_request();
        let me = *self;
        spawn_local(async move {
            let url = format!("{base}/v1/me");
            let Ok(resp) = Request::get(&url).send().await else {
                return;
            };
            let Ok(owner) = resp.json::<OwnerIdentity>().await else {
                return;
            };
            me.apply_owner_response(&base, ticket, owner);
        });
    }

    /// Fetch available agents from GET /v1/agents (TASK-9/TASK-11).
    /// Unresolved catalog entries stay visible with their resolution error.
    pub fn fetch_agents(&self) {
        let base = self.base();
        let agents_sig = self.available_agents;
        let root_sig = self.agents_root;
        let loaded = self.agents_loaded;
        spawn_local(async move {
            let url = format!("{base}/v1/agents");
            match Request::get(&url).send().await {
                Ok(resp) => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        root_sig.set(
                            json.get("root")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        );
                        agents_sig.set(parse_agent_summaries(&json));
                    }
                }
                Err(_) => {
                    agents_sig.set(Vec::new());
                }
            }
            // Resolved (success, empty, or error) — the picker may now show
            // "No agents" honestly instead of during the in-flight window.
            loaded.set(true);
        });
    }

    /// Create a room (`POST /v1/rooms/persistent`) with an optional auto-convene
    /// `trigger_policy`, then select it. The daemon keys rooms by `key`; we
    /// derive a url-safe key from the name but keep the human name intact.
    /// Atomically dispatch a create-room request, returning the op_id the
    /// caller should snapshot. When the request resolves, `create_op` carries
    /// a typed outcome tagged with that id — surfaces gate only on their own
    /// id and leave concurrent submits untouched.
    ///
    /// Every side effect is gated on CAS admission: a stale completion
    /// superseded by a later dispatch must never select the wrong room or
    /// overwrite the current attempt's status. Stale successes still refresh
    /// the room list so a server-created room is discoverable.
    ///
    /// Callers should gate dispatch on `pending_create` to prevent concurrent
    /// attempts — the closure in `rooms_workspace/` does this.
    pub fn create_room(&self, name: String, policy: Option<RoomTriggerPolicy>) -> u64 {
        let name = name.trim().to_string();
        if name.is_empty() {
            return 0;
        }
        let key = slugify(&name);
        if key.is_empty() {
            self.status
                .set("room name needs at least one letter/number".into());
            return 0;
        }
        let base = self.base();
        let me = *self;
        let status = self.status;
        let signal = self.create_op;
        let op_id = {
            let (n, _) = signal.get_untracked();
            n.wrapping_add(1)
        };
        // Set "in flight" immediately so the caller doesn't immediately
        // observe a stale-success resolution from a prior request.
        signal.set((op_id, None));
        spawn_local(async move {
            let body = CreateRoomBody {
                key: &key,
                name: &name,
                trigger_policy: policy,
            };
            let post_url = format!("{base}/v1/rooms/persistent");
            let res = Request::post(&post_url)
                .header("content-type", "application/json")
                .json(&body);
            let res = match res {
                Ok(req) => req.send().await,
                Err(err) => {
                    // Encode error: CAS-gate the status + outcome together.
                    // Stale encode errors are fully suppressed — they carry
                    // no server-side state.
                    signal.update(|(cur, out)| {
                        if Self::cas_admit_create(*cur, op_id) {
                            status.set(format!("create encode error: {err}"));
                            *out = Some(CreateOutcome::Failed {
                                error: format!("encode: {err}"),
                            });
                        }
                    });
                    return;
                }
            };
            let outcome = match res {
                Ok(resp) => match resp.json::<RoomMutateResponse>().await {
                    Ok(r) if r.ok => CreateOutcome::Success { key: key.clone() },
                    Ok(r) => {
                        let msg = r.error.unwrap_or_else(|| "unknown error".into());
                        if msg.to_lowercase().contains("duplicate") {
                            CreateOutcome::Duplicate
                        } else {
                            CreateOutcome::Failed { error: msg }
                        }
                    }
                    Err(err) => CreateOutcome::Failed {
                        error: format!("decode: {err}"),
                    },
                },
                Err(err) => CreateOutcome::Failed {
                    error: format!("post: {err}"),
                },
            };
            // CAS-gated publish. Admitted: full status + select + list refresh.
            // Stale success: list refresh only (server-created room must be
            // discoverable). Stale failure/duplicate: fully suppressed.
            signal.update(|(cur, out)| {
                if Self::cas_admit_create(*cur, op_id) {
                    *out = Some(outcome.clone());
                    match &outcome {
                        CreateOutcome::Success { key } => {
                            status.set(format!("room '{name}' created"));
                            me.fetch_rooms();
                            me.open_room(key.clone());
                        }
                        CreateOutcome::Duplicate => {
                            status.set(format!("room '{name}' already exists"));
                        }
                        CreateOutcome::Failed { error } => {
                            status.set(format!("create failed: {error}"));
                        }
                    }
                } else if matches!(&outcome, CreateOutcome::Success { .. }) {
                    // Stale success: room exists server-side — refresh the
                    // list so it's discoverable, but never select or report.
                    me.fetch_rooms();
                }
                // Stale failure/duplicate: no side effects.
            });
        });
        op_id
    }

    /// Whether a create-room completion is still current (not superseded
    /// by a later dispatch). Admitted = the slot op matches ours; stale
    /// = the slot was claimed by a newer dispatch.
    pub fn cas_admit_create(slot_op: u64, my_op: u64) -> bool {
        CasAdmission::admit(slot_op, my_op)
    }

    /// Resolve a pending create-room dispatch from the op-id slot.
    /// Called by the surface Effect — only the matching op_id's outcome
    /// determines whether to clear the draft.
    pub fn resolve_create_op(
        current_op: u64,
        my_op: u64,
        outcome: Option<&CreateOutcome>,
    ) -> CreateResolution {
        if current_op != my_op {
            return CreateResolution::Pending;
        }
        match outcome {
            Some(CreateOutcome::Success { .. }) => CreateResolution::Success,
            Some(CreateOutcome::Duplicate) | Some(CreateOutcome::Failed { .. }) => {
                CreateResolution::KeepDraft
            }
            None => CreateResolution::Pending,
        }
    }

    /// Mint a scoped, expiring, single-use invite for the open room. The
    /// daemon owns Bedrock registration and credential custody; this client
    /// receives only the intentionally shareable invite response.
    pub fn create_invite(&self) {
        if self.invite_loading.get_untracked() {
            return;
        }
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        self.invite.set(None);
        self.invite_error.set(None);
        self.invite_loading.set(true);
        spawn_local(async move {
            let url = format!("{base}/v1/rooms/persistent/{}/invites", encode(&key));
            let result = match Request::post(&url)
                .header("content-type", "application/json")
                .body("{}")
                .expect("static invite body is valid")
                .send()
                .await
            {
                Ok(response) if response.ok() => response
                    .json::<RoomInvite>()
                    .await
                    .map_err(|_| "The invitation response was invalid.".to_string()),
                Ok(response) if response.status() == 503 => {
                    Err("Room sharing is not configured on this Ocean.".to_string())
                }
                Ok(_) => Err("Ocean could not create an invitation.".to_string()),
                Err(_) => Err("Ocean could not reach the room service.".to_string()),
            };
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            me.invite_loading.set(false);
            match result {
                Ok(invite) if invite.room_key == key => me.invite.set(Some(invite)),
                Ok(_) => me
                    .invite_error
                    .set(Some("The invitation did not match this room.".into())),
                Err(error) => me.invite_error.set(Some(error)),
            }
        });
    }
    /// Stage an attacker-triggerable native invite for explicit confirmation.
    /// Opening a link never consumes a single-use invite by itself.
    pub fn stage_invite(&self, room_key: String, code: String) {
        self.status.set(String::new());
        self.pending_invite_revision
            .update(|revision| *revision = revision.wrapping_add(1));
        self.pending_invite_redemption.set(None);
        self.pending_invite
            .set(Some(PendingRoomInvite { room_key, code }));
    }

    pub fn dismiss_pending_invite(&self) {
        self.pending_invite_revision
            .update(|revision| *revision = revision.wrapping_add(1));
        self.pending_invite.set(None);
        self.pending_invite_redemption.set(None);
        self.status.set(String::new());
    }

    pub fn pending_invite_joining(&self) -> bool {
        self.pending_invite_redemption
            .get()
            .as_ref()
            .is_some_and(|request| {
                pending_invite_request_is_current(
                    request,
                    self.pending_invite_revision.get(),
                    self.generation.get(),
                    self.pending_invite.get().as_ref(),
                )
            })
    }

    /// Redeem a deep-linked invite through the local daemon, then open the
    /// daemon-authoritative room returned by redemption. The room key embedded
    /// in the attacker-triggerable link is routing decoration, not authority.
    /// The invite code stays in the POST body and never enters status text.
    pub fn redeem_invite(&self, link_room_key: String, code: String) {
        let Some(request) = begin_pending_invite_redemption(
            PendingRoomInvite {
                room_key: link_room_key,
                code,
            },
            self.pending_invite_revision.get_untracked(),
            self.generation.get_untracked(),
            self.pending_invite.get_untracked().as_ref(),
            self.pending_invite_redemption.get_untracked().as_ref(),
        ) else {
            return;
        };
        let base = self.base();
        let me = *self;
        self.pending_invite_redemption.set(Some(request.clone()));
        self.status.set("Joining shared room…".into());
        spawn_local(async move {
            let body = serde_json::json!({"code": &request.invite.code});
            let result = match Request::post(&format!("{base}/v1/rooms/persistent/invites/redeem"))
                .header("content-type", "application/json")
                .json(&body)
                .expect("invite request serializes")
                .send()
                .await
            {
                Ok(response) if response.ok() => response
                    .json::<RedeemInviteResponse>()
                    .await
                    .map(|redeemed| redeemed.room_key)
                    .map_err(|_| "The shared room response was invalid."),
                Ok(response) if response.status() == 403 => {
                    Err("This invitation is invalid or has expired.")
                }
                Ok(response) if response.status() == 409 => {
                    Err("This room is already linked differently.")
                }
                Ok(response) if response.status() == 503 => {
                    Err("Room sharing is not configured on this Ocean.")
                }
                Ok(_) => Err("Ocean could not join the shared room."),
                Err(_) => Err("Ocean could not reach the room service."),
            };
            if !pending_invite_completion_is_current(
                &request,
                me.pending_invite_revision.get_untracked(),
                me.generation.get_untracked(),
                me.pending_invite.get_untracked().as_ref(),
                me.pending_invite_redemption.get_untracked().as_ref(),
            ) {
                return;
            }
            me.pending_invite_redemption.set(None);
            match result {
                Ok(room_key) => {
                    me.pending_invite.set(None);
                    me.status.set("Shared room joined.".into());
                    me.fetch_rooms();
                    me.open_room(room_key);
                }
                Err(error) => me.status.set(error.into()),
            }
        });
    }

    /// Open a room: load its record + full transcript, bump the generation, and
    /// start the room-scoped SSE live tail (TASK-10/TASK-11).
    pub fn open_room(&self, key: String) {
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked().wrapping_add(1);
        self.generation.set(generation_id);
        self.open_key.set(Some(key.clone()));
        self.reset_room_state();
        self.status.set("loading room…".into());

        spawn_local(async move {
            let get_url = format!("{base}/v1/rooms/persistent/{}", encode(&key));
            let result = match Request::get(&get_url).send().await {
                Ok(resp) if resp.ok() => match resp.json::<RoomGetResponse>().await {
                    Ok(r) if r.ok => Ok((r.room, r.transcript, r.access)),
                    Ok(r) => Err(format!(
                        "room load failed: {}",
                        r.error.unwrap_or_else(|| "unknown error".into())
                    )),
                    Err(err) => Err(format!("room decode error: {err}")),
                },
                Ok(resp) => {
                    let http_status = resp.status();
                    match resp.json::<RoomErrorResponse>().await {
                        Ok(r) => Err(format!(
                            "room load failed: {}",
                            r.error.unwrap_or_else(|| format!("HTTP {http_status}"))
                        )),
                        Err(err) => Err(format!("room load failed: HTTP {http_status} ({err})")),
                    }
                }
                Err(err) => Err(format!("room fetch error: {err}")),
            };
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok((room, transcript, access)) => {
                    me.open_room.set(room);
                    me.transcript.set(transcript.clone());
                    me.access.set(Some(access.clone()));
                    update_open_summary_from_open_room(
                        &me.read_summaries,
                        me.open_key.get_untracked().as_deref(),
                        &transcript,
                        Some(&access),
                        me.open_read_cursor.get_untracked().as_ref(),
                    );
                    me.status.set(String::new());
                    me.fetch_agents();
                    me.start_live_tail(key, generation_id);
                }
                Err(error) => me.status.set(error),
            }
        });
    }
    /// Close the open room and stop its live loops.
    pub fn close_room(&self) {
        self.generation.update(|g| *g = g.wrapping_add(1));
        self.open_key.set(None);
        self.reset_room_state();
    }

    /// Join the open room as the current identity
    /// (`POST .../participants`).
    pub fn join_open(&self) {
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        spawn_local(async move {
            let body = JoinBody {
                kind: RoomParticipantKind::Human,
            };
            let post_url = format!("{base}/v1/rooms/persistent/{}/participants", encode(&key));
            let result = match Request::post(&post_url)
                .header("content-type", "application/json")
                .json(&body)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) => match resp.json::<RoomMutateResponse>().await {
                        Ok(r) if r.ok => Ok(r.room),
                        Ok(r) => Err(format!(
                            "join failed: {}",
                            r.error.unwrap_or_else(|| "unknown error".into())
                        )),
                        Err(err) => Err(format!("join decode error: {err}")),
                    },
                    Err(err) => Err(format!("join post error: {err}")),
                },
                Err(err) => Err(format!("join encode error: {err}")),
            };
            if result.is_ok() {
                me.fetch_rooms();
            }
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(room) => {
                    me.open_room.set(room);
                    me.status.set("joined".into());
                    me.refresh_open_transcript(&key, generation_id);
                }
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Add a daemon-owned agent to the open room.
    ///
    /// Routes on the room's access state: a `Local` room takes the
    /// participant path (`POST .../participants`, kind = agent); a
    /// credentialed room registers the agent through the daemon's federation
    /// bridge (`POST .../members/agents`) so the coworkers' rosters see it
    /// with its public descriptor. A revoked room is refused up front.
    pub fn add_agent(&self, agent_id: String) {
        let agent_id = agent_id.trim().to_string();
        if agent_id.is_empty() {
            self.status.set("agent id required".into());
            return;
        }
        if self.add_agent_in_flight.get_untracked().is_some() {
            return;
        }
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let access_state = self.access.get_untracked().map(|access| access.state);
        match agent_add_route(access_state) {
            AgentAddRoute::Participant => self.add_agent_local(key, agent_id),
            AgentAddRoute::Register => self.add_agent_federated(key, agent_id),
            AgentAddRoute::Refused => {
                self.status
                    .set("Agents can't be added to a revoked room.".into());
            }
        }
    }

    fn add_agent_local(&self, key: String, agent_id: String) {
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        self.add_agent_in_flight.set(Some(agent_id.clone()));
        spawn_local(async move {
            let body = AgentJoinBody {
                id: &agent_id,
                display_name: &agent_id,
                kind: RoomParticipantKind::Agent,
            };
            let post_url = format!("{base}/v1/rooms/persistent/{}/participants", encode(&key));
            let result = match Request::post(&post_url)
                .header("content-type", "application/json")
                .json(&body)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) => match resp.json::<RoomMutateResponse>().await {
                        Ok(r) if r.ok => Ok(r.room),
                        Ok(r) => Err(format!(
                            "add agent failed: {}",
                            r.error.unwrap_or_else(|| "unknown error".into())
                        )),
                        Err(err) => Err(format!("add agent decode error: {err}")),
                    },
                    Err(err) => Err(format!("add agent post error: {err}")),
                },
                Err(err) => Err(format!("add agent encode error: {err}")),
            };
            me.add_agent_in_flight.set(None);
            if result.is_ok() {
                me.fetch_rooms();
            }
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(room) => {
                    me.open_room.set(room);
                    me.status
                        .set(format!("agent '{agent_id}' added — mention @{agent_id}"));
                    me.refresh_open_transcript(&key, generation_id);
                }
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Register a local folder-as-agent into a credentialed (shared) room.
    /// The daemon resolves the agent, builds its public descriptor, and
    /// registers it with Bedrock; the response is the refreshed access
    /// projection (roster included), which replaces `access` directly.
    fn add_agent_federated(&self, key: String, agent_id: String) {
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        self.add_agent_in_flight.set(Some(agent_id.clone()));
        spawn_local(async move {
            let body = serde_json::json!({ "agent_names": [agent_id] });
            let url = format!("{base}/v1/rooms/persistent/{}/members/agents", encode(&key));
            let result = match Request::post(&url)
                .header("content-type", "application/json")
                .json(&body)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) if resp.ok() => resp
                        .json::<RoomAccessProjection>()
                        .await
                        .map_err(|_| "The room roster response was invalid.".to_string()),
                    Ok(resp) => Err(register_agent_error_text(resp.status())),
                    Err(_) => Err("Ocean could not reach the room service.".to_string()),
                },
                Err(err) => Err(format!("add agent encode error: {err}")),
            };
            me.add_agent_in_flight.set(None);
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(access) => {
                    me.access.set(Some(access));
                    me.status.set(format!(
                        "agent '{agent_id}' registered — mention @{agent_id}"
                    ));
                }
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Ids of the open room's **agent** participants — the actors a human can
    /// `@mention` to auto-convene. Used to render the composer's discoverability
    /// hint.
    #[allow(dead_code)]
    pub fn agent_ids(&self) -> Vec<String> {
        let access = self.access.get();
        let room = self.open_room.get();
        agent_ids_for(access.as_ref(), room.as_ref())
    }

    /// Leave the open room (`DELETE .../participants/{id}`).
    pub fn leave_open(&self) {
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        let id = self.identity_id.get_untracked();
        spawn_local(async move {
            let del_url = format!(
                "{base}/v1/rooms/persistent/{}/participants/{}",
                encode(&key),
                encode(id)
            );
            let result = match Request::delete(&del_url).send().await {
                Ok(resp) => match resp.json::<RoomMutateResponse>().await {
                    Ok(r) if r.ok => Ok(r.room),
                    Ok(r) => Err(format!(
                        "leave failed: {}",
                        r.error.unwrap_or_else(|| "unknown error".into())
                    )),
                    Err(err) => Err(format!("leave decode error: {err}")),
                },
                Err(err) => Err(format!("leave error: {err}")),
            };
            if result.is_ok() {
                me.fetch_rooms();
            }
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(room) => {
                    me.open_room.set(room);
                    me.status.set("left".into());
                    me.refresh_open_transcript(&key, generation_id);
                }
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Post a message to the open room (`POST .../messages`). `@id` mentions in
    /// the body drive the daemon's trigger-policy auto-convene.
    pub fn post_message(&self, body: String, thread_parent_seq: Option<u64>) {
        if !access_allows_writes(self.access.get_untracked().as_ref()) {
            return;
        }
        let body = body.trim().to_string();
        if body.is_empty() {
            return;
        }
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        spawn_local(async move {
            let payload = PostMessageBody {
                author_kind: RoomParticipantKind::Human,
                body: &body,
                thread_parent_seq,
            };
            let post_url = format!("{base}/v1/rooms/persistent/{}/messages", encode(&key));
            let result = match Request::post(&post_url)
                .header("content-type", "application/json")
                .json(&payload)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) if resp.ok() => Ok(()),
                    Ok(resp) => Err(format!(
                        "message failed: {}",
                        resp.text().await.unwrap_or_default()
                    )),
                    Err(err) => Err(format!("message post error: {err}")),
                },
                Err(err) => Err(format!("message encode error: {err}")),
            };
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(()) => me.refresh_open_transcript(&key, generation_id),
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Retry a failed outbox item (`POST …/outbox/retry`).
    #[allow(dead_code)]
    pub fn retry_outbox(&self, client_event_id: String) {
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        spawn_local(async move {
            let payload = RetryOutboxBody {
                client_event_id: &client_event_id,
            };
            let post_url = format!("{base}/v1/rooms/persistent/{}/outbox/retry", encode(&key));
            let result = match Request::post(&post_url)
                .header("content-type", "application/json")
                .json(&payload)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) if resp.status() == 202 => {
                        match resp.json::<RetryOutboxSuccess>().await {
                            Ok(r) if r.ok => Ok(r.access),
                            Ok(_) => Err("retry response invalid".into()),
                            Err(err) => Err(format!("retry decode error: {err}")),
                        }
                    }
                    Ok(resp) => {
                        let http_status = resp.status();
                        match resp.json::<RetryOutboxErrorResponse>().await {
                            Ok(r) => {
                                let detail = match (r.code, r.error) {
                                    (Some(code), Some(error)) => format!("{code}: {error}"),
                                    (Some(code), None) => code,
                                    (None, Some(error)) => error,
                                    (None, None) => format!("HTTP {http_status}"),
                                };
                                Err(format!("retry failed: {detail}"))
                            }
                            Err(err) => Err(format!("retry failed: HTTP {http_status} ({err})")),
                        }
                    }
                    Err(err) => Err(format!("retry post error: {err}")),
                },
                Err(err) => Err(format!("retry encode error: {err}")),
            };
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            match result {
                Ok(access) => {
                    apply_access_projection(&me.access, access);
                    me.status.set("retry queued".into());
                }
                Err(error) => me.status.set(error),
            }
        });
    }

    /// Re-fetch the open room's transcript tail and append only new entries.
    fn refresh_open_transcript(&self, key: &str, generation_id: u64) {
        let base = self.base();
        let me = *self;
        let key = key.to_string();
        spawn_local(async move {
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            let after = last_transcript_seq(&me.transcript.get_untracked());
            let get_url = format!(
                "{base}/v1/rooms/persistent/{}/transcript?after_seq={after}",
                encode(&key)
            );
            if let Ok(resp) = Request::get(&get_url).send().await {
                if let Ok(r) = resp.json::<TranscriptResponse>().await {
                    if r.ok && !r.transcript.is_empty() && me.room_is_current(generation_id, &key) {
                        me.transcript.update(|transcript| {
                            for message in r.transcript {
                                if transcript.last().map(|last| last.seq).unwrap_or(0) < message.seq
                                {
                                    transcript.push(message);
                                }
                            }
                        });
                    }
                }
            }
        });
    }

    /// Start the live tail for `key` at `generation_id`: room-scoped SSE
    /// (`GET /v1/rooms/persistent/{key}/events`) with `?after_seq=` resume for
    /// newly constructed browser connections (TASK-10/TASK-11). Replaces the
    /// 2.5s poll workaround.
    fn start_live_tail(&self, key: String, generation_id: u64) {
        let me = *self;
        let base = self.base();
        let last_seq = RwSignal::new(last_transcript_seq(&self.transcript.get_untracked()));
        let tail_state = self.tail_state;

        spawn_local(async move {
            let events_url = format!("{base}/v1/rooms/persistent/{}/events", encode(&key));
            let mut resume_seq = last_seq.get_untracked();
            let mut reconnecting = false;

            loop {
                if !me.room_is_current(generation_id, &key) {
                    break;
                }
                tail_state.set(if reconnecting {
                    TailState::Reconnecting
                } else {
                    TailState::Replaying
                });

                let url = format!("{events_url}?after_seq={resume_seq}");
                let mut es = match EventSource::new(&url) {
                    Ok(es) => es,
                    Err(_) => {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        continue;
                    }
                };
                let message_sub = match es.subscribe("room_message") {
                    Ok(s) => s
                        .map(|event| event.map(|msg| ("room_message", msg)))
                        .boxed_local(),
                    Err(_) => {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        continue;
                    }
                };
                let access_sub = match es.subscribe("room_access") {
                    Ok(s) => s
                        .map(|event| event.map(|msg| ("room_access", msg)))
                        .boxed_local(),
                    Err(_) => {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        continue;
                    }
                };
                let read_cursor_sub = match es.subscribe("room_read_cursor") {
                    Ok(s) => s
                        .map(|event| event.map(|msg| ("room_read_cursor", msg)))
                        .boxed_local(),
                    Err(_) => {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        continue;
                    }
                };
                // Only write connection state if this room+generation is still current.
                if me.room_is_current(generation_id, &key) {
                    tail_state.set(match es.state() {
                        gloo_net::eventsource::State::Open => TailState::Live,
                        gloo_net::eventsource::State::Connecting if reconnecting => {
                            TailState::Reconnecting
                        }
                        gloo_net::eventsource::State::Connecting => TailState::Replaying,
                        gloo_net::eventsource::State::Closed => TailState::Reconnecting,
                    });
                }
                let run_sub = match es.subscribe("room_agent_run") {
                    Ok(s) => s
                        .map(|event| event.map(|msg| ("room_agent_run", msg)))
                        .boxed_local(),
                    Err(_) => {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        continue;
                    }
                };
                let mut stream = futures_util::stream::select(
                    futures_util::stream::select(message_sub, access_sub),
                    futures_util::stream::select(read_cursor_sub, run_sub),
                );
                // Race stream.next() against a 2 s timeout so room close/switch
                // can cancel a stalled connection (blame: gloo EventSource errors
                // are suppressed during CONNECTING, so Reconnecting never fires
                // without an explicit timeout-pump — codex TASK-11 review).
                loop {
                    if !me.room_is_current(generation_id, &key) {
                        break;
                    }
                    let next = stream.next();
                    let timeout = gloo_timers::future::TimeoutFuture::new(2_000);
                    let msg = match futures_util::future::select(Box::pin(next), Box::pin(timeout))
                        .await
                    {
                        Either::Left((Some(msg), _)) => msg,
                        Either::Left((None, _)) => break, // stream ended
                        Either::Right(_) => {
                            // Timeout fired — poll the native connection to
                            // distinguish a quiet stream from a dead one.
                            // gloo wraps web_sys::EventSource; State mirrors
                            // the underlying readyState constants.
                            // Only write tail_state if this room+gen is still current.
                            if me.room_is_current(generation_id, &key) {
                                match es.state() {
                                    gloo_net::eventsource::State::Open => {
                                        tail_state.set(TailState::Live);
                                    }
                                    gloo_net::eventsource::State::Connecting => {
                                        tail_state.set(TailState::Reconnecting);
                                    }
                                    gloo_net::eventsource::State::Closed => {
                                        tail_state.set(TailState::Reconnecting);
                                        break; // fall through to outer reconnect loop
                                    }
                                }
                            } else {
                                break;
                            }
                            continue;
                        }
                    };
                    let Ok((name, msg)) = msg else { continue };
                    let Some(data) = msg.1.data().as_string() else {
                        continue;
                    };
                    let Some(frame) = decode_room_tail_frame(name, &data, &key) else {
                        continue;
                    };
                    let Some(frame) = accept_room_tail_frame(
                        frame,
                        generation_id,
                        me.generation.get_untracked(),
                        &key,
                        me.open_key.get_untracked().as_deref(),
                    ) else {
                        break;
                    };
                    tail_state.set(TailState::Live);
                    match frame {
                        RoomTailFrame::Run(run) => {
                            me.runs.update(|runs| upsert_run(runs, run));
                        }
                        RoomTailFrame::Access(access) => {
                            apply_access_projection(&me.access, access.clone());
                            update_open_summary_from_open_room(
                                &me.read_summaries,
                                Some(&key),
                                &me.transcript.get_untracked(),
                                Some(&access),
                                me.open_read_cursor.get_untracked().as_ref(),
                            );
                        }
                        RoomTailFrame::ReadCursor(cursor) => {
                            // Mirrored SSE cursors merge monotonically with the
                            // cursor already held for this room+generation, so a
                            // lagging frame cannot lower the durable read.
                            let merged = merge_read_cursor_projection(
                                me.open_read_cursor.get_untracked().as_ref(),
                                cursor,
                            );
                            me.open_read_cursor.set(Some(merged.clone()));
                            update_open_summary_from_open_room(
                                &me.read_summaries,
                                Some(&key),
                                &me.transcript.get_untracked(),
                                me.access.get_untracked().as_ref(),
                                Some(&merged),
                            );
                        }
                        RoomTailFrame::Message(entry) => {
                            if entry.seq > last_seq.get_untracked() {
                                last_seq.set(entry.seq);
                            }
                            let is_roster_change = matches!(
                                entry.kind,
                                RoomMessageKind::ParticipantJoined
                                    | RoomMessageKind::ParticipantLeft
                            );
                            me.transcript.update(|t| {
                                if t.iter().any(|m| m.seq == entry.seq) {
                                    return;
                                }
                                t.push(entry);
                            });
                            update_open_summary_from_open_room(
                                &me.read_summaries,
                                Some(&key),
                                &me.transcript.get_untracked(),
                                me.access.get_untracked().as_ref(),
                                me.open_read_cursor.get_untracked().as_ref(),
                            );
                            // Refresh the room record (roster) on join/leave frames
                            // so other clients see an accurate participant list.
                            if is_roster_change {
                                let base = base.clone();
                                let key = key.clone();
                                let open_room = me.open_room;
                                spawn_local(async move {
                                    if !me.room_is_current(generation_id, &key) {
                                        return;
                                    }
                                    if let Ok(resp) = Request::get(&format!(
                                        "{base}/v1/rooms/persistent/{}",
                                        encode(&key)
                                    ))
                                    .send()
                                    .await
                                    {
                                        if let Ok(r) = resp.json::<RoomMutateResponse>().await {
                                            if r.ok {
                                                if !me.room_is_current(generation_id, &key) {
                                                    return;
                                                }
                                                if let Some(room) = r.room {
                                                    open_room.set(Some(room));
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                resume_seq = last_seq.get_untracked();
                reconnecting = true;
                if !me.room_is_current(generation_id, &key) {
                    break;
                }
                gloo_timers::future::TimeoutFuture::new(1_000).await;
            }
        });
    }

    pub fn mark_open_read_if_current(&self, candidate_read_seq: u64) {
        let Some(key) = self.open_key.get_untracked() else {
            return;
        };
        let current_summary = self
            .read_summaries
            .get_untracked()
            .get(&key)
            .copied()
            .unwrap_or(RoomReadSummary {
                latest_seq: None,
                read_seq: None,
            });
        let durable_read_seq = self
            .open_read_cursor
            .get_untracked()
            .as_ref()
            .and_then(current_durable_read_seq);
        if !should_send_read_cursor(
            candidate_read_seq,
            current_summary.read_seq,
            durable_read_seq,
            self.read_cursor_in_flight.get_untracked(),
        ) {
            return;
        }

        let base = self.base();
        let me = *self;
        let generation_id = self.generation.get_untracked();
        let ticket = self
            .read_cursor_request_ticket
            .get_untracked()
            .wrapping_add(1);
        let request = ReadCursorRequest {
            ticket,
            read_seq: candidate_read_seq,
        };
        self.read_cursor_request_ticket.set(ticket);
        self.read_cursor_in_flight.set(Some(request));
        spawn_local(async move {
            let patch_url = format!("{base}/v1/rooms/persistent/{}/read-cursor", encode(&key));
            let body = ReadCursorPatchBody {
                read_seq: candidate_read_seq,
            };
            let result = match Request::patch(&patch_url)
                .header("content-type", "application/json")
                .json(&body)
            {
                Ok(req) => match req.send().await {
                    Ok(resp) if resp.ok() => match resp.json::<ReadCursorPatchEnvelope>().await {
                        Ok(envelope) if envelope.ok => {
                            parse_patch_read_cursor_response(&key, envelope.cursor)
                        }
                        Ok(_) => Err("read cursor failed: unknown error".into()),
                        Err(err) => Err(format!("read cursor decode error: {err}")),
                    },
                    Ok(resp) => match resp.json::<RoomErrorResponse>().await {
                        Ok(error) => Err(format!(
                            "read cursor failed: {}",
                            error
                                .error
                                .unwrap_or_else(|| format!("HTTP {}", resp.status()))
                        )),
                        Err(err) => Err(format!(
                            "read cursor failed: HTTP {} ({err})",
                            resp.status()
                        )),
                    },
                    Err(err) => Err(format!("read cursor patch error: {err}")),
                },
                Err(err) => Err(format!("read cursor encode error: {err}")),
            };
            if !me.room_is_current(generation_id, &key) {
                return;
            }
            if let Ok(cursor) = result {
                let merged = merge_read_cursor_projection(
                    me.open_read_cursor.get_untracked().as_ref(),
                    cursor,
                );
                me.open_read_cursor.set(Some(merged));
                update_open_summary_from_open_room(
                    &me.read_summaries,
                    Some(&key),
                    &me.transcript.get_untracked(),
                    me.access.get_untracked().as_ref(),
                    me.open_read_cursor.get_untracked().as_ref(),
                );
            }
            me.read_cursor_in_flight.update(|in_flight| {
                *in_flight = finish_read_cursor_request(*in_flight, request);
            });
        });
    }
}

// ---- Helpers ----------------------------------------------------------------

fn begin_pending_invite_redemption(
    invite: PendingRoomInvite,
    intent_revision: u64,
    room_generation: u64,
    pending: Option<&PendingRoomInvite>,
    in_flight: Option<&PendingInviteRedemption>,
) -> Option<PendingInviteRedemption> {
    if pending != Some(&invite) {
        return None;
    }
    let request = PendingInviteRedemption {
        invite,
        intent_revision,
        room_generation,
    };
    (in_flight != Some(&request)).then_some(request)
}

fn pending_invite_request_is_current(
    request: &PendingInviteRedemption,
    intent_revision: u64,
    room_generation: u64,
    pending: Option<&PendingRoomInvite>,
) -> bool {
    request.intent_revision == intent_revision
        && request.room_generation == room_generation
        && pending == Some(&request.invite)
}

fn pending_invite_completion_is_current(
    request: &PendingInviteRedemption,
    intent_revision: u64,
    room_generation: u64,
    pending: Option<&PendingRoomInvite>,
    in_flight: Option<&PendingInviteRedemption>,
) -> bool {
    in_flight == Some(request)
        && pending_invite_request_is_current(request, intent_revision, room_generation, pending)
}

/// Only confirmed positions dedupe completed writes. A clamped acknowledgement
/// releases the request, allowing the visible candidate to be attempted again.
fn should_send_read_cursor(
    candidate_read_seq: u64,
    summary_read_seq: Option<u64>,
    durable_read_seq: Option<u64>,
    in_flight: Option<ReadCursorRequest>,
) -> bool {
    candidate_read_seq > applied_open_read_seq(summary_read_seq, durable_read_seq)
        && !in_flight.is_some_and(|request| request.read_seq >= candidate_read_seq)
}

fn finish_read_cursor_request(
    in_flight: Option<ReadCursorRequest>,
    completed: ReadCursorRequest,
) -> Option<ReadCursorRequest> {
    in_flight.filter(|request| *request != completed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoomTailFrame {
    Message(RoomMessage),
    Access(RoomAccessProjection),
    ReadCursor(RoomReadCursorProjection),
    Run(RoomAgentRun),
}

/// Live lifecycle of one room-convened agent turn (team-platform P3), mirror
/// of `ocean_core::RoomAgentRunState`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RoomAgentRunState {
    Queued,
    Thinking,
    RunningTool {
        label: String,
    },
    AwaitingPermission,
    AwaitingReply,
    Done,
    Failed {
        reason: String,
    },
    Cancelled,
    /// A state this build does not know yet renders as idle work.
    #[serde(other)]
    Unknown,
}

impl RoomAgentRunState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. } | Self::Cancelled)
    }
}

/// One agent turn in a room: the work card's daemon-owned projection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomAgentRun {
    pub run_id: String,
    pub room_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub trigger_seq: u64,
    pub thread_root_seq: u64,
    #[serde(flatten)]
    pub state: RoomAgentRunState,
    pub started_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub tool_count: u32,
    #[serde(default)]
    pub reply_seq: Option<u64>,
    /// The tool approval this run is blocked on (owner-local).
    #[serde(default)]
    pub pending_permission: Option<RoomRunPermission>,
}

/// A pending tool approval on a run. Mirrors `ocean_core::RoomRunPermission`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RoomRunPermission {
    pub permission_id: String,
    /// Tool name of the pending call; the decision names it back.
    #[serde(default)]
    pub tool: String,
    pub tool_label: String,
}

/// Per-room agent overrides. Mirrors `ocean_core::RoomAgentSettings`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAgentSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Deserialize)]
struct RoomAgentSettingsEnvelope {
    settings: RoomAgentSettings,
}

#[derive(Deserialize)]
struct RoomErrorEnvelope {
    #[serde(default)]
    error: String,
}

const UNREACHABLE: &str = "Ocean could not reach the room service.";

/// The decision body for one pending permission. It names the exact request
/// the owner saw (`permission_id` + `tool`), so the daemon refuses it with 409
/// once that request is stale instead of applying it to a later one.
pub fn run_permission_body(permission: &RoomRunPermission, allow: bool) -> String {
    let mut body = serde_json::json!({
        "permission_id": permission.permission_id,
        "decision": if allow { "allow" } else { "deny" },
    });
    if !permission.tool.is_empty() {
        body["tool"] = serde_json::Value::String(permission.tool.clone());
    }
    body.to_string()
}

/// A Room owner mutation that needs daemon Room operator authority. Exactly
/// the routes a first-party surface may make as the operator (decision
/// 2026-10-08): the web proxy attaches the key for its logged-in session, the
/// Tauri shell attaches it natively (`host::room_owner_mutation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerRoute<'a> {
    RunPermission { key: &'a str, run_id: &'a str },
    AgentSettings { key: &'a str, agent_id: &'a str },
}

impl OwnerRoute<'_> {
    fn method(self) -> &'static str {
        match self {
            Self::RunPermission { .. } => "POST",
            Self::AgentSettings { .. } => "PUT",
        }
    }

    fn url(self, base: &str) -> String {
        match self {
            Self::RunPermission { key, run_id } => format!(
                "{base}/v1/rooms/persistent/{}/runs/{}/permission",
                encode(key),
                encode(run_id)
            ),
            Self::AgentSettings { key, agent_id } => agent_settings_url(base, key, agent_id),
        }
    }

    /// The native shell's invoke arguments: one of its fixed kinds plus the
    /// raw ids (the shell encodes them into its own fixed path).
    fn ids(self) -> (&'static str, String, String) {
        match self {
            Self::RunPermission { key, run_id } => {
                ("run_permission", key.to_string(), run_id.to_string())
            }
            Self::AgentSettings { key, agent_id } => {
                ("agent_settings", key.to_string(), agent_id.to_string())
            }
        }
    }
}

/// Status and body of an owner mutation, from either transport.
struct OwnerReply {
    status: u16,
    body: String,
}

/// Send an owner mutation: through the native shell on Tauri (it attaches the
/// operator key), else as an ordinary request to `base` (the web proxy
/// attaches the key for its logged-in session; a direct daemon refuses).
async fn send_owner_mutation(
    base: &str,
    route: OwnerRoute<'_>,
    body: String,
) -> Result<OwnerReply, ()> {
    let (kind, room, target) = route.ids();
    if let Some(reply) =
        crate::host::room_owner_mutation(kind, Some(&room), Some(&target), &body).await
    {
        return reply
            .map(|r| OwnerReply {
                status: r.status,
                body: r.body,
            })
            .map_err(|_| ());
    }
    let url = route.url(base);
    let builder = match route.method() {
        "POST" => Request::post(&url),
        _ => Request::put(&url),
    };
    let request = builder
        .header("content-type", "application/json")
        .body(body)
        .map_err(|_| ())?;
    let response = request.send().await.map_err(|_| ())?;
    Ok(OwnerReply {
        status: response.status(),
        body: response.text().await.unwrap_or_default(),
    })
}

/// Owner approve/deny for a run's pending tool permission. The card updates
/// from the next `room_agent_run` frame; nothing is applied optimistically.
pub async fn decide_run_permission(
    base: &str,
    key: &str,
    run_id: &str,
    permission: &RoomRunPermission,
    allow: bool,
) -> Result<(), String> {
    let route = OwnerRoute::RunPermission { key, run_id };
    match send_owner_mutation(base, route, run_permission_body(permission, allow)).await {
        Ok(r) if (200..300).contains(&r.status) => Ok(()),
        Ok(r) if r.status == 409 || r.status == 404 => Err("Already decided.".into()),
        Ok(_) => Err("Ocean could not record the decision.".into()),
        Err(()) => Err(UNREACHABLE.into()),
    }
}

fn agent_settings_url(base: &str, key: &str, agent_id: &str) -> String {
    format!(
        "{base}/v1/rooms/persistent/{}/agents/{}/settings",
        encode(key),
        encode(agent_id)
    )
}

pub async fn fetch_agent_settings(
    base: &str,
    key: &str,
    agent_id: &str,
) -> Result<RoomAgentSettings, String> {
    match Request::get(&agent_settings_url(base, key, agent_id))
        .send()
        .await
    {
        Ok(r) if r.ok() => r
            .json::<RoomAgentSettingsEnvelope>()
            .await
            .map(|e| e.settings)
            .map_err(|_| "Ocean returned invalid agent settings.".into()),
        Ok(_) => Err("Ocean could not load agent settings.".into()),
        Err(_) => Err(UNREACHABLE.into()),
    }
}

pub async fn save_agent_settings(
    base: &str,
    key: &str,
    agent_id: &str,
    settings: &RoomAgentSettings,
) -> Result<RoomAgentSettings, String> {
    let body = serde_json::to_string(settings).map_err(|_| UNREACHABLE.to_string())?;
    let route = OwnerRoute::AgentSettings { key, agent_id };
    match send_owner_mutation(base, route, body).await {
        Ok(r) if (200..300).contains(&r.status) => {
            serde_json::from_str::<RoomAgentSettingsEnvelope>(&r.body)
                .map(|e| e.settings)
                .map_err(|_| "Ocean returned invalid agent settings.".into())
        }
        Ok(r) if r.status == 400 => {
            let code = serde_json::from_str::<RoomErrorEnvelope>(&r.body)
                .map(|e| e.error)
                .unwrap_or_default();
            Err(settings_error_message(&code).into())
        }
        Ok(_) => Err("Ocean could not save agent settings.".into()),
        Err(()) => Err(UNREACHABLE.into()),
    }
}

/// Human text for a settings validation code.
pub(crate) fn settings_error_message(code: &str) -> &'static str {
    match code {
        "instructions_too_long" => "Instructions are too long.",
        "invalid_model" => "That model name is not valid.",
        _ => "Ocean rejected these settings.",
    }
}

/// Ids of the runs whose card attaches under thread root `root_seq`.
pub(crate) fn run_ids_for_root(runs: &[RoomAgentRun], root_seq: u64) -> Vec<String> {
    runs.iter()
        .filter(|r| r.thread_root_seq == root_seq)
        .map(|r| r.run_id.clone())
        .collect()
}

/// Match the daemon's recent work-card projection on reconnect and live updates.
pub(crate) const ROOM_RUNS_LIMIT: usize = 50;

/// Insert or replace a run by id, retaining the most recent starts.
pub(crate) fn upsert_run(runs: &mut Vec<RoomAgentRun>, run: RoomAgentRun) {
    match runs.iter_mut().find(|r| r.run_id == run.run_id) {
        Some(existing) => *existing = run,
        None => runs.push(run),
    }
    runs.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then(a.run_id.cmp(&b.run_id))
    });
    let excess = runs.len().saturating_sub(ROOM_RUNS_LIMIT);
    runs.drain(..excess);
}

fn decode_room_tail_frame(
    name: &str,
    data: &str,
    expected_room_key: &str,
) -> Option<RoomTailFrame> {
    match name {
        "room_message" => serde_json::from_str(data).ok().map(RoomTailFrame::Message),
        "room_access" => serde_json::from_str(data).ok().map(RoomTailFrame::Access),
        // Cursor frames carry a durable read position, so room identity is
        // validated by construction here: the expected key is required and the
        // frame is dropped unless the wire `room_id` matches it exactly.
        "room_read_cursor" => serde_json::from_str::<RoomReadCursorBody>(data)
            .ok()
            .and_then(|body| {
                parse_room_read_cursor_projection(
                    expected_room_key,
                    ReadCursorProjectionTarget::MirroredUpstream,
                    body,
                )
                .ok()
            })
            .map(RoomTailFrame::ReadCursor),
        // Run frames carry their room identity; drop any that name another.
        "room_agent_run" => serde_json::from_str::<RoomAgentRun>(data)
            .ok()
            .filter(|run| run.room_id == expected_room_key)
            .map(RoomTailFrame::Run),
        _ => None,
    }
}

/// Admit a decoded SSE frame only while its captured room generation and key
/// still own the open room. This is the single production boundary before any
/// frame-driven tail state, access, cursor, or transcript mutation.
fn accept_room_tail_frame(
    frame: RoomTailFrame,
    expected_generation: u64,
    current_generation: u64,
    expected_key: &str,
    current_key: Option<&str>,
) -> Option<RoomTailFrame> {
    room_request_is_current(
        expected_generation,
        current_generation,
        expected_key,
        current_key,
    )
    .then_some(frame)
}

fn read_summaries_from_wire(
    read_states: &[RoomReadStateWire],
) -> Result<HashMap<String, RoomReadSummary>, String> {
    let mut summaries = HashMap::with_capacity(read_states.len());
    for state in read_states {
        let latest_seq = parse_optional_decimal_u64(state.latest_seq.as_deref())?;
        let read_seq = parse_optional_decimal_u64(state.read_seq.as_deref())?;
        if summaries
            .insert(
                state.room_id.clone(),
                RoomReadSummary {
                    latest_seq,
                    read_seq,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate read state for room '{}'", state.room_id));
        }
    }
    Ok(summaries)
}

fn parse_optional_decimal_u64(raw: Option<&str>) -> Result<Option<u64>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty decimal read state".into());
    }
    trimmed
        .parse::<u64>()
        .map(Some)
        .map_err(|_| format!("invalid decimal read state '{trimmed}'"))
}

fn parse_room_read_cursor_projection(
    expected_room_key: &str,
    target: ReadCursorProjectionTarget,
    body: RoomReadCursorBody,
) -> Result<RoomReadCursorProjection, String> {
    let room_id = body.room_id.trim();
    if room_id.is_empty() {
        return Err("read cursor decode error: empty room_id".into());
    }
    if room_id != expected_room_key {
        return Err(format!(
            "read cursor decode error: wrong room_id '{room_id}' for '{expected_room_key}'"
        ));
    }
    let read_seq = parse_optional_decimal_u64(body.read_seq.as_deref())?;
    Ok(match target {
        ReadCursorProjectionTarget::Local => RoomReadCursorProjection {
            read_seq,
            mirrored_upstream_read_seq: None,
        },
        ReadCursorProjectionTarget::MirroredUpstream => RoomReadCursorProjection {
            read_seq: None,
            mirrored_upstream_read_seq: read_seq,
        },
    })
}

fn parse_patch_read_cursor_response(
    expected_room_key: &str,
    response: RoomReadCursorBody,
) -> Result<RoomReadCursorProjection, String> {
    parse_room_read_cursor_projection(
        expected_room_key,
        ReadCursorProjectionTarget::Local,
        response,
    )
}

/// Fold a newly observed cursor projection into the one already held for the
/// open room. Both the local (PATCH-confirmed) and mirrored (SSE) positions
/// advance monotonically, so a lagging mirrored frame can neither lower the
/// durable read, drop a locally confirmed read, nor resurrect unread — while a
/// later, higher mirrored frame still corrects the durable read upward.
fn merge_read_cursor_projection(
    current: Option<&RoomReadCursorProjection>,
    incoming: RoomReadCursorProjection,
) -> RoomReadCursorProjection {
    let Some(current) = current else {
        return incoming;
    };
    RoomReadCursorProjection {
        read_seq: max_optional_u64(current.read_seq, incoming.read_seq),
        mirrored_upstream_read_seq: max_optional_u64(
            current.mirrored_upstream_read_seq,
            incoming.mirrored_upstream_read_seq,
        ),
    }
}

/// The durable read position is the furthest confirmed read across both the
/// local PATCH projection and the mirrored upstream projection.
fn current_durable_read_seq(cursor: &RoomReadCursorProjection) -> Option<u64> {
    max_optional_u64(cursor.read_seq, cursor.mirrored_upstream_read_seq)
}

/// The read position already applied for the open room: the furthest of the
/// summary's confirmed read and the durable cursor projection. Folding with a
/// monotonic max (rather than preferring the summary when present) keeps a
/// lagging summary from re-sending a PATCH the durable cursor already covers.
fn applied_open_read_seq(summary_read_seq: Option<u64>, durable_read_seq: Option<u64>) -> u64 {
    max_optional_u64(summary_read_seq, durable_read_seq).unwrap_or(0)
}

fn latest_summary_seq_for_open_room(
    transcript: &[RoomMessage],
    access: Option<&RoomAccessProjection>,
) -> Option<u64> {
    match access.map(|projection| projection.state) {
        Some(RoomAccessState::Live) => {
            access.and_then(|projection| projection.last_confirmed_global_sequence)
        }
        _ => transcript.last().map(|message| message.seq),
    }
}

fn merged_room_read_summary(
    current: Option<&RoomReadSummary>,
    incoming: Option<&RoomReadSummary>,
) -> Option<RoomReadSummary> {
    match (current, incoming) {
        (None, None) => None,
        (Some(current), None) => Some(*current),
        (None, Some(incoming)) => Some(*incoming),
        (Some(current), Some(incoming)) => Some(RoomReadSummary {
            latest_seq: max_optional_u64(current.latest_seq, incoming.latest_seq),
            read_seq: max_optional_u64(current.read_seq, incoming.read_seq),
        }),
    }
}

fn max_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn merge_room_read_summaries(
    current: &HashMap<String, RoomReadSummary>,
    rooms: &[Room],
    incoming: &HashMap<String, RoomReadSummary>,
) -> HashMap<String, RoomReadSummary> {
    let mut merged = HashMap::with_capacity(rooms.len());
    for room in rooms {
        let room_id = room.id.clone();
        if let Some(summary) =
            merged_room_read_summary(current.get(&room_id), incoming.get(&room_id))
        {
            merged.insert(room_id, summary);
        }
    }
    merged
}

fn update_open_summary_from_open_room(
    summaries: &RwSignal<HashMap<String, RoomReadSummary>>,
    open_key: Option<&str>,
    transcript: &[RoomMessage],
    access: Option<&RoomAccessProjection>,
    cursor: Option<&RoomReadCursorProjection>,
) {
    let Some(open_key) = open_key else {
        return;
    };
    let latest_seq = latest_summary_seq_for_open_room(transcript, access);
    let existing = summaries.get_untracked().get(open_key).copied();
    let read_seq = max_optional_u64(
        cursor.and_then(current_durable_read_seq),
        existing.and_then(|summary| summary.read_seq),
    );
    summaries.update(|map| {
        map.insert(
            open_key.to_string(),
            RoomReadSummary {
                latest_seq: max_optional_u64(
                    existing.and_then(|summary| summary.latest_seq),
                    latest_seq,
                ),
                read_seq,
            },
        );
    });
}

pub(crate) fn room_has_durable_unread(summary: Option<&RoomReadSummary>) -> bool {
    let Some(summary) = summary else {
        return false;
    };
    match (summary.latest_seq, summary.read_seq) {
        (Some(latest), Some(read)) => latest > read,
        (Some(_), None) => true,
        _ => false,
    }
}

fn apply_access_projection(
    signal: &RwSignal<Option<RoomAccessProjection>>,
    next: RoomAccessProjection,
) -> bool {
    let mut current = signal.get_untracked();
    let changed = replace_access_projection(&mut current, next);
    if changed {
        signal.set(current);
    }
    changed
}

fn replace_access_projection(
    current: &mut Option<RoomAccessProjection>,
    next: RoomAccessProjection,
) -> bool {
    if current.as_ref() == Some(&next) {
        return false;
    }
    *current = Some(next);
    true
}

fn last_transcript_seq(transcript: &[RoomMessage]) -> u64 {
    transcript.last().map(|message| message.seq).unwrap_or(0)
}

fn list_request_is_current(expected_ticket: u64, current_ticket: u64) -> bool {
    expected_ticket == current_ticket
}

fn should_skip_rooms_fetch(mode: RoomsFetchMode, rooms_loading: bool) -> bool {
    matches!(mode, RoomsFetchMode::Silent) && rooms_loading
}

fn finish_rooms_fetch(
    rooms_loaded: &RwSignal<bool>,
    rooms_loading: &RwSignal<bool>,
    mode: RoomsFetchMode,
    is_current: bool,
) {
    if !is_current {
        return;
    }
    rooms_loaded.set(true);
    if matches!(mode, RoomsFetchMode::Interactive) {
        rooms_loading.set(false);
    }
}

fn joined_open_for(
    access: Option<&RoomAccessProjection>,
    room: Option<&Room>,
    identity_id: &str,
) -> bool {
    let Some(access) = access else {
        return false;
    };
    if access.state == RoomAccessState::Local {
        return room.is_some_and(|room| {
            room.participants
                .iter()
                .any(|participant| participant.id == identity_id)
        });
    }
    // A federated room knows "me" only by the daemon-projected member id.
    let Some(me) = access.local_member_id.as_deref() else {
        return false;
    };
    access.members.iter().any(|member| member.member_id == me)
}

/// Human-readable name for a message author or member id: the room roster's
/// display name first (Local), then the federated member projection, falling
/// back to the raw id only when neither authority names it.
pub fn author_display_name(
    room: Option<&Room>,
    access: Option<&RoomAccessProjection>,
    author_id: &str,
) -> String {
    room.and_then(|room| {
        room.participants
            .iter()
            .find(|p| p.id == author_id)
            .map(|p| p.display_name.clone())
    })
    .or_else(|| {
        access.and_then(|access| {
            access
                .members
                .iter()
                .find(|m| m.member_id == author_id)
                .map(|m| m.display_name.clone())
        })
    })
    .filter(|name| !name.trim().is_empty())
    .unwrap_or_else(|| author_id.to_string())
}

/// Up to two uppercase initials from a display name ("Ada King" -> "AK",
/// "researcher" -> "RE", "" -> "?").
pub fn name_initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    let initials: String = match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    initials.to_uppercase()
}

/// The id this surface's owner speaks as in the open room: the Local roster
/// id from `/v1/me`, or the daemon-projected Bedrock member id when federated.
pub fn local_speaker_id(
    access: Option<&RoomAccessProjection>,
    owner_participant_id: &str,
) -> Option<String> {
    match access {
        Some(a) if a.state == RoomAccessState::Local => {
            (!owner_participant_id.is_empty()).then(|| owner_participant_id.to_string())
        }
        Some(a) => a.local_member_id.clone(),
        None => None,
    }
}

/// Which placeholder the rooms list should render, given whether the first
/// fetch has resolved and how many rooms came back. Splitting `Loading` from
/// `Empty` stops the panel from flashing "No rooms yet" while the initial
/// request is still in flight — an empty list only *means* empty once loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum RoomsListState {
    Loading,
    Empty,
    Populated,
}

#[allow(dead_code)]
pub(crate) fn rooms_list_state(loaded: bool, room_count: usize) -> RoomsListState {
    if room_count > 0 {
        // Rooms are present — always render them, even if a refetch is in
        // flight. Only the *empty* list is ambiguous between loading and empty.
        RoomsListState::Populated
    } else if loaded {
        RoomsListState::Empty
    } else {
        RoomsListState::Loading
    }
}

/// Whether the open room's transcript should show its "No messages yet" empty
/// state. Only once the live tail is actually connected (`Live`) AND the
/// transcript is empty: during the initial `Replaying` catch-up (or a
/// `Reconnecting` gap) an empty transcript means "still loading", not
/// "genuinely empty", so the stage must not flash the empty copy on room open
/// before history arrives (same bug class as [`rooms_list_state`]).
#[allow(dead_code)]
fn show_transcript_empty(tail: TailState, transcript_empty: bool) -> bool {
    transcript_empty && matches!(tail, TailState::Live)
}

/// One discoverable folder-as-agent, as summarized by `GET /v1/agents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummary {
    pub name: String,
    pub description: Option<String>,
    pub model: Option<String>,
    pub skills: u32,
    pub subagents: Vec<String>,
    /// Set when the daemon could discover the folder but not resolve it
    /// (malformed `agent.toml`, …). Such an agent is shown but not addable.
    pub error: Option<String>,
}

/// Parse the `/v1/agents` payload into summaries. Unknown/missing fields
/// degrade to `None`/empty; an entry without a `name` is dropped.
fn parse_agent_summaries(json: &serde_json::Value) -> Vec<AgentSummary> {
    let Some(agents) = json.get("agents").and_then(|a| a.as_array()) else {
        return Vec::new();
    };
    agents
        .iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let str_field = |k: &str| {
                a.get(k)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            Some(AgentSummary {
                name: name.to_string(),
                description: str_field("description"),
                model: str_field("model"),
                skills: a
                    .get("skills")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                subagents: a
                    .get("subagents")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                error: str_field("error"),
            })
        })
        .collect()
}

/// Which daemon route adds an agent to a room in a given access state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentAddRoute {
    /// Local room: `POST .../participants` with `kind: agent`.
    Participant,
    /// Credentialed room: `POST .../members/agents` (Bedrock registration).
    Register,
    /// Revoked room: refuse without a request.
    Refused,
}

pub(crate) fn agent_add_route(state: Option<RoomAccessState>) -> AgentAddRoute {
    match state {
        None | Some(RoomAccessState::Local) => AgentAddRoute::Participant,
        Some(RoomAccessState::Revoked) => AgentAddRoute::Refused,
        Some(RoomAccessState::Connecting | RoomAccessState::Live | RoomAccessState::Recovering) => {
            AgentAddRoute::Register
        }
    }
}

/// Operator-facing text for a failed `/members/agents` registration. Mirrors
/// the daemon's `intent_error_response` status mapping.
fn register_agent_error_text(status: u16) -> String {
    match status {
        400 => "Ocean could not resolve that agent folder.".into(),
        403 => "This room's access has been revoked.".into(),
        404 => "That room no longer exists.".into(),
        409 => "This room is not linked to Bedrock yet.".into(),
        502 => "Bedrock returned an unexpected response.".into(),
        503 => "Room sharing is not configured on this Ocean.".into(),
        _ => "Ocean could not register the agent.".into(),
    }
}

/// Whether the add-agent picker should show its "No agents" hint: only once
/// `/v1/agents` has resolved AND the list is empty. During the initial fetch an
/// empty list means "still loading", not "no agents" (same flash class as the
/// rooms-list and transcript empties).
#[allow(dead_code)]
fn show_no_agents(agents_loaded: bool, agent_count: usize) -> bool {
    agents_loaded && agent_count == 0
}

/// Pure predicate: is `expected_generation`/`expected_key` still the current
/// room admission? `pub(crate)` so sibling modules (`rooms_workspace/`) can
/// unit-test the exact rejection logic behind [`Rooms::room_is_current`]
/// without needing a live `Rooms` handle (which requires a browser runtime).
pub(crate) fn room_request_is_current(
    expected_generation: u64,
    current_generation: u64,
    expected_key: &str,
    current_key: Option<&str>,
) -> bool {
    expected_generation == current_generation && current_key == Some(expected_key)
}

fn access_allows_writes(access: Option<&RoomAccessProjection>) -> bool {
    matches!(
        access.map(|projection| projection.state),
        Some(RoomAccessState::Local | RoomAccessState::Live)
    )
}

#[allow(dead_code)]
fn access_banner(access: Option<&RoomAccessProjection>) -> Option<&'static str> {
    match access.map(|projection| projection.state) {
        Some(RoomAccessState::Connecting) => Some("Connecting"),
        Some(RoomAccessState::Recovering) => Some("Recovering"),
        Some(RoomAccessState::Revoked) => Some("Access revoked"),
        None | Some(RoomAccessState::Local | RoomAccessState::Live) => None,
    }
}

#[allow(dead_code)]
/// Agent *names* already present in the open room, for de-duplicating the
/// add-agent picker. Local rooms key agents by participant id (== folder
/// name); federated rosters key them by Bedrock member id, so there the
/// public descriptor's `display_name` (the canonical folder name the daemon
/// registered) is the comparable value.
pub(crate) fn present_agent_names(
    access: Option<&RoomAccessProjection>,
    room: Option<&Room>,
) -> Vec<String> {
    match access {
        Some(access) if access.state != RoomAccessState::Local => access
            .members
            .iter()
            .filter(|member| member.actor_type == FederatedActorType::Agent)
            .map(|member| {
                member
                    .public_agent_descriptor
                    .as_ref()
                    .map(|d| d.display_name.clone())
                    .unwrap_or_else(|| member.display_name.clone())
            })
            .collect(),
        _ => room
            .map(|room| {
                room.participants
                    .iter()
                    .filter(|p| p.kind == RoomParticipantKind::Agent)
                    .map(|p| p.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn agent_ids_for(access: Option<&RoomAccessProjection>, room: Option<&Room>) -> Vec<String> {
    let Some(access) = access else {
        return Vec::new();
    };
    if access.state != RoomAccessState::Local {
        return access
            .members
            .iter()
            .filter(|member| member.actor_type == FederatedActorType::Agent)
            .map(|member| member.member_id.clone())
            .collect();
    }
    room.map(|room| {
        room.participants
            .iter()
            .filter(|participant| participant.kind == RoomParticipantKind::Agent)
            .map(|participant| participant.id.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// Derive a url/key-safe slug from a room name (lowercase alnum + `-`).
fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Percent-encode a path segment (room keys can contain `-`/`_`/alnum already,
/// but a defensive encode keeps an unexpected char from breaking the URL).
/// Pure Rust so tests run on native targets.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// A compact "last activity" label from an ISO-8601 timestamp — just the
/// date+time portion, trimmed. Empty input → empty string.
#[allow(dead_code)]
fn short_time(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    // "2026-06-05T12:34:56.789Z" → "2026-06-05 12:34"
    let trimmed = ts.split('.').next().unwrap_or(ts).replace('T', " ");
    trimmed.chars().take(16).collect()
}

/// Build the per-room LiveKit token path, percent-encoding the room key.
/// Pure utility used by `daemon.rs` bootstrap; rooms G1 does not call it.
pub(crate) fn livekit_token_path_for_room(key: &str) -> String {
    format!("/v1/rooms/{}/livekit-token", encode(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_routes_map_to_the_shell_kinds_and_the_daemon_paths() {
        let base = "http://127.0.0.1:4780";
        let decision = OwnerRoute::RunPermission {
            key: "team room",
            run_id: "run-1",
        };
        assert_eq!(
            decision.ids(),
            (
                "run_permission",
                "team room".to_string(),
                "run-1".to_string()
            )
        );
        assert_eq!(decision.method(), "POST");
        assert_eq!(
            decision.url(base),
            format!(
                "{base}/v1/rooms/persistent/{}/runs/run-1/permission",
                encode("team room")
            )
        );
        let settings = OwnerRoute::AgentSettings {
            key: "k",
            agent_id: "helper",
        };
        assert_eq!(
            settings.ids(),
            ("agent_settings", "k".to_string(), "helper".to_string())
        );
        assert_eq!(settings.method(), "PUT");
        assert_eq!(settings.url(base), agent_settings_url(base, "k", "helper"));
    }

    #[test]
    fn run_permission_body_names_the_request_the_owner_saw() {
        let permission = RoomRunPermission {
            permission_id: "7c1d2f9e-1111-4222-8333-944445555666".into(),
            tool: "bash".into(),
            tool_label: "bash cargo test".into(),
        };
        let allow: serde_json::Value =
            serde_json::from_str(&run_permission_body(&permission, true)).unwrap();
        assert_eq!(
            allow,
            serde_json::json!({
                "permission_id": "7c1d2f9e-1111-4222-8333-944445555666",
                "tool": "bash",
                "decision": "allow",
            })
        );
        let deny: serde_json::Value =
            serde_json::from_str(&run_permission_body(&permission, false)).unwrap();
        assert_eq!(deny["decision"], "deny");
        assert_eq!(deny["permission_id"], permission.permission_id.as_str());
        // A card from an older daemon carries no tool name; the id still binds.
        let legacy: RoomRunPermission = serde_json::from_value(serde_json::json!({
            "permission_id": "p1",
            "tool_label": "write a.rs",
        }))
        .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&run_permission_body(&legacy, true)).unwrap();
        assert_eq!(body["permission_id"], "p1");
        assert!(body.get("tool").is_none());
    }

    #[test]
    fn redeem_response_uses_daemon_authoritative_room_key() {
        let response: RedeemInviteResponse = serde_json::from_value(serde_json::json!({
            "room_key": "canonical-room",
            "state": "connecting",
            "members": [],
            "outbox": []
        }))
        .unwrap();
        assert_eq!(response.room_key, "canonical-room");
    }

    #[test]
    fn no_agents_hint_waits_for_agents_fetch() {
        // Empty list before the fetch resolves = still loading, hide the hint.
        assert!(!show_no_agents(false, 0));
        // Once resolved and still empty = genuinely no agents.
        assert!(show_no_agents(true, 0));
        // Any agents present never shows the empty hint.
        assert!(!show_no_agents(true, 3));
        assert!(!show_no_agents(false, 3));
    }

    #[test]
    fn transcript_empty_state_waits_for_live_tail() {
        // During the initial catch-up the tail is Replaying and the transcript
        // is empty — that is "still loading", NOT "no messages", so the empty
        // copy must stay hidden.
        assert!(!show_transcript_empty(TailState::Replaying, true));
        // A reconnect gap with an empty transcript is likewise ambiguous.
        assert!(!show_transcript_empty(TailState::Reconnecting, true));
        // Only once connected (Live) does an empty transcript genuinely mean
        // "no messages yet".
        assert!(show_transcript_empty(TailState::Live, true));
        // A non-empty transcript never shows the empty state, in any tail state.
        assert!(!show_transcript_empty(TailState::Live, false));
        assert!(!show_transcript_empty(TailState::Replaying, false));
    }

    #[test]
    fn rooms_list_state_distinguishes_loading_from_genuinely_empty() {
        // Before the first fetch resolves, an empty list means "still loading",
        // NOT "no rooms" — the panel must not assert emptiness prematurely.
        assert_eq!(rooms_list_state(false, 0), RoomsListState::Loading);
        // A non-empty list mid-load still renders rooms (they arrived).
        assert_eq!(rooms_list_state(false, 3), RoomsListState::Populated);
        // Once loaded, an empty list is genuinely empty.
        assert_eq!(rooms_list_state(true, 0), RoomsListState::Empty);
        // Once loaded with rooms, populated.
        assert_eq!(rooms_list_state(true, 2), RoomsListState::Populated);
    }

    fn access_projection(state: RoomAccessState) -> RoomAccessProjection {
        RoomAccessProjection {
            state,
            caller_member_id: None,
            last_confirmed_global_sequence: None,
            local_member_id: None,
            members: Vec::new(),
            outbox: Vec::new(),
        }
    }

    fn message(seq: u64) -> RoomMessage {
        RoomMessage {
            seq,
            author_id: "member-1".into(),
            author_kind: RoomParticipantKind::Human,
            kind: RoomMessageKind::Message,
            body: format!("message {seq}"),
            created_at: "2026-07-16T22:00:00Z".into(),
            federated: None,
            thread_parent_seq: None,
        }
    }

    #[test]
    fn post_message_wire_omits_none_thread_parent_and_includes_some() {
        // P2: the wire carries no author id — the daemon authors as its owner.
        let root = serde_json::to_value(PostMessageBody {
            author_kind: RoomParticipantKind::Human,
            body: "root body",
            thread_parent_seq: None,
        })
        .expect("root post message body should serialize");
        assert_eq!(
            root,
            serde_json::json!({
                "author_kind": "human",
                "body": "root body"
            })
        );

        let reply = serde_json::to_value(PostMessageBody {
            author_kind: RoomParticipantKind::Human,
            body: "reply body",
            thread_parent_seq: Some(7),
        })
        .expect("reply post message body should serialize");
        assert_eq!(
            reply,
            serde_json::json!({
                "author_kind": "human",
                "body": "reply body",
                "thread_parent_seq": 7
            })
        );
    }

    fn local_room() -> Room {
        Room {
            id: "room-1".into(),
            name: "Room One".into(),
            participants: vec![
                RoomParticipant {
                    id: "local-agent".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Local Agent".into(),
                },
                RoomParticipant {
                    id: "local-human".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "Local Human".into(),
                },
            ],
            created_at: String::new(),
            updated_at: String::new(),
            trigger_policy: None,
            muted: false,
        }
    }

    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("Map Fix"), "map-fix");
        assert_eq!(slugify("  Ocean   Surface!! "), "ocean-surface");
        assert_eq!(slugify("already-ok_123"), "already-ok-123");
    }

    #[test]
    fn slugify_strips_leading_trailing_separators() {
        assert_eq!(slugify("!!!hi!!!"), "hi");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn short_time_trims_iso_to_minute() {
        assert_eq!(short_time("2026-06-05T12:34:56.789Z"), "2026-06-05 12:34");
        assert_eq!(short_time(""), "");
    }

    #[test]
    fn livekit_token_path_percent_encodes_room_key_as_one_path_segment() {
        assert_eq!(
            livekit_token_path_for_room("project/surface demo"),
            "/v1/rooms/project%2Fsurface%20demo/livekit-token"
        );
    }

    #[test]
    fn g1_room_message_without_federation_metadata_decodes_as_none() {
        let message: RoomMessage = serde_json::from_value(serde_json::json!({
            "seq": 1,
            "author_id": "local-human",
            "author_kind": "human",
            "kind": "message",
            "body": "hello",
            "created_at": "2026-07-16T22:00:00Z"
        }))
        .expect("G1 message should decode");

        assert_eq!(message.federated, None);
    }

    #[test]
    fn room_message_thread_parent_seq_decodes_and_defaults_to_none() {
        let reply: RoomMessage = serde_json::from_value(serde_json::json!({
            "seq": 2,
            "author_id": "local-human",
            "author_kind": "human",
            "kind": "message",
            "body": "reply",
            "created_at": "2026-07-16T22:01:00Z",
            "thread_parent_seq": 1
        }))
        .expect("reply should decode");
        assert_eq!(reply.thread_parent_seq, Some(1));

        let root: RoomMessage = serde_json::from_value(serde_json::json!({
            "seq": 1,
            "author_id": "local-human",
            "author_kind": "human",
            "kind": "message",
            "body": "root",
            "created_at": "2026-07-16T22:00:00Z"
        }))
        .expect("root should decode");
        assert_eq!(root.thread_parent_seq, None);
    }

    #[test]
    fn room_get_requires_access_and_local_projection_is_exact() {
        let response: RoomGetResponse = serde_json::from_value(serde_json::json!({
            "ok": true,
            "room": null,
            "transcript": [],
            "access": { "state": "local" }
        }))
        .expect("P1 room envelope should decode");
        assert_eq!(response.access, access_projection(RoomAccessState::Local));

        let missing = serde_json::from_value::<RoomGetResponse>(serde_json::json!({
            "ok": true,
            "room": null,
            "transcript": []
        }));
        assert!(missing.is_err(), "access must remain required");

        let error: RoomErrorResponse = serde_json::from_value(serde_json::json!({
            "ok": false,
            "error": "no room with key 'missing'"
        }))
        .expect("non-success responses use the separate error envelope");
        assert_eq!(error.error.as_deref(), Some("no room with key 'missing'"));

        assert_eq!(
            serde_json::to_value(access_projection(RoomAccessState::Local)).unwrap(),
            serde_json::json!({ "state": "local" })
        );
    }

    #[test]
    fn federated_access_projection_uses_only_safe_exact_wire_fields() {
        let projection: RoomAccessProjection = serde_json::from_value(serde_json::json!({
            "state": "live",
            "last_confirmed_global_sequence": 44,
            "members": [{
                "member_id": "member-agent",
                "owner_member_id": "member-owner",
                "actor_type": "agent",
                "role_in_room": "member",
                "display_name": "Fable",
                "public_agent_descriptor": {
                    "display_name": "Fable",
                    "description": "reviewer",
                    "model_alias": "fable",
                    "skills_count": 2,
                    "subagent_names": ["research", "review"]
                },
                "joined_at": "2026-07-16T22:00:00Z",
                "derived_presence": "live",
                "local_binding_available": false
            }],
            "outbox": [{
                "client_event_id": "client-1",
                "source_id": "surface-web",
                "source_sequence": 7,
                "author_member_id": "member-owner",
                "event_type": "room_message",
                "payload": { "body": "hello" },
                "mention_member_ids": ["member-agent"],
                "state": "failed"
            }]
        }))
        .expect("full safe projection should decode");

        let wire = serde_json::to_value(&projection).unwrap();
        assert_eq!(
            wire["members"][0],
            serde_json::json!({
                "member_id": "member-agent",
                "owner_member_id": "member-owner",
                "actor_type": "agent",
                "role_in_room": "member",
                "display_name": "Fable",
                "public_agent_descriptor": {
                    "display_name": "Fable",
                    "description": "reviewer",
                    "model_alias": "fable",
                    "skills_count": 2,
                    "subagent_names": ["research", "review"]
                },
                "joined_at": "2026-07-16T22:00:00Z",
                "derived_presence": "live",
                "local_binding_available": false
            })
        );
        let member = wire["members"][0].as_object().unwrap();
        for secret in [
            "owner_principal_token_id",
            "registration_key",
            "bearer_token",
            "access_token",
        ] {
            assert!(!member.contains_key(secret));
        }
        assert_eq!(
            wire["outbox"][0],
            serde_json::json!({
                "client_event_id": "client-1",
                "source_id": "surface-web",
                "source_sequence": 7,
                "author_member_id": "member-owner",
                "event_type": "room_message",
                "payload": { "body": "hello" },
                "mention_member_ids": ["member-agent"],
                "state": "failed"
            })
        );
    }

    #[test]
    fn all_access_states_pin_write_and_banner_policy() {
        let cases = [
            (RoomAccessState::Local, true, None),
            (RoomAccessState::Connecting, false, Some("Connecting")),
            (RoomAccessState::Live, true, None),
            (RoomAccessState::Recovering, false, Some("Recovering")),
            (RoomAccessState::Revoked, false, Some("Access revoked")),
        ];

        assert!(!access_allows_writes(None));
        assert_eq!(access_banner(None), None);
        for (state, writes, banner) in cases {
            let access = access_projection(state);
            assert_eq!(access_allows_writes(Some(&access)), writes);
            assert_eq!(access_banner(Some(&access)), banner);
        }
    }

    #[test]
    fn tail_frame_decoder_tags_access_and_messages_without_cursor_blending() {
        let access =
            serde_json::to_string(&access_projection(RoomAccessState::Recovering)).unwrap();
        let frame = decode_room_tail_frame("room_access", &access, "room-1").unwrap();
        assert_eq!(
            frame,
            RoomTailFrame::Access(access_projection(RoomAccessState::Recovering))
        );

        let frame = decode_room_tail_frame(
            "room_message",
            r#"{"seq":8,"author_id":"member-1","author_kind":"human","kind":"message","body":"hello","created_at":"2026-07-16T22:00:00Z"}"#,
            "room-1",
        )
        .unwrap();
        match frame {
            RoomTailFrame::Message(message) => assert_eq!(message.seq, 8),
            other => panic!("message frame decoded as {other:?}"),
        }
        assert!(decode_room_tail_frame("unknown", "{}", "room-1").is_none());
    }

    fn run_wire(run_id: &str, room: &str, state: &str, started: &str) -> String {
        format!(
            r#"{{"run_id":"{run_id}","room_id":"{room}","agent_id":"helper","session_id":"s","trigger_seq":4,"thread_root_seq":4,"state":"{state}","label":"bash cargo test","started_at":"{started}","updated_at":"{started}","tool_count":2}}"#
        )
    }

    #[test]
    fn run_frames_decode_only_for_the_open_room() {
        let wire = run_wire("r1", "room-1", "running_tool", "2026-10-03T10:00:00Z");
        let Some(RoomTailFrame::Run(run)) =
            decode_room_tail_frame("room_agent_run", &wire, "room-1")
        else {
            panic!("run frame must decode");
        };
        assert_eq!(
            run.state,
            RoomAgentRunState::RunningTool {
                label: "bash cargo test".into()
            }
        );
        assert_eq!(run.tool_count, 2);
        assert!(run.files_changed.is_empty());
        assert!(
            decode_room_tail_frame("room_agent_run", &wire, "room-2").is_none(),
            "a run naming another room is dropped"
        );
        let future = run_wire("r2", "room-1", "some_future_state", "2026-10-03T10:00:00Z");
        let Some(RoomTailFrame::Run(run)) =
            decode_room_tail_frame("room_agent_run", &future, "room-1")
        else {
            panic!("unknown states still decode");
        };
        assert_eq!(run.state, RoomAgentRunState::Unknown);
    }

    #[test]
    fn upsert_run_replaces_by_id_and_keeps_start_order() {
        let decode = |w: String| serde_json::from_str::<RoomAgentRun>(&w).unwrap();
        let mut runs = Vec::new();
        upsert_run(
            &mut runs,
            decode(run_wire("b", "r", "queued", "2026-10-03T10:00:05Z")),
        );
        upsert_run(
            &mut runs,
            decode(run_wire("a", "r", "queued", "2026-10-03T10:00:00Z")),
        );
        upsert_run(
            &mut runs,
            decode(run_wire("b", "r", "done", "2026-10-03T10:00:05Z")),
        );
        assert_eq!(
            runs.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(runs[1].state, RoomAgentRunState::Done);
        assert_eq!(run_ids_for_root(&runs, 4), vec!["a", "b"]);
        assert!(run_ids_for_root(&runs, 5).is_empty());
    }

    #[test]
    fn live_run_projection_keeps_only_latest_fifty_across_replay_and_updates() {
        let make = |index: usize, state: &str| {
            serde_json::from_str::<RoomAgentRun>(&run_wire(
                &format!("run-{index:03}"),
                "r",
                state,
                &format!("2026-10-03T10:{:02}:{:02}Z", index / 60, index % 60),
            ))
            .unwrap()
        };
        let mut runs = Vec::new();
        // Replay may arrive in reverse order; retain newest, not last received.
        for index in (0..75).rev() {
            upsert_run(&mut runs, make(index, "thinking"));
            assert!(runs.len() <= ROOM_RUNS_LIMIT);
        }
        assert_eq!(runs.first().unwrap().run_id, "run-025");
        assert_eq!(runs.last().unwrap().run_id, "run-074");
        upsert_run(&mut runs, make(74, "done"));
        assert_eq!(runs.len(), ROOM_RUNS_LIMIT);
        assert_eq!(runs.last().unwrap().state, RoomAgentRunState::Done);
        // A late update to an evicted run must not displace a recent card.
        upsert_run(&mut runs, make(2, "done"));
        assert_eq!(runs.first().unwrap().run_id, "run-025");
        upsert_run(&mut runs, make(75, "queued"));
        assert_eq!(runs.first().unwrap().run_id, "run-026");
        assert_eq!(runs.last().unwrap().run_id, "run-075");
    }

    #[test]
    fn access_projection_replacement_is_idempotent() {
        let live = access_projection(RoomAccessState::Live);
        let mut current = Some(live.clone());
        assert!(!replace_access_projection(&mut current, live));

        let recovering = access_projection(RoomAccessState::Recovering);
        assert!(replace_access_projection(&mut current, recovering.clone()));
        assert_eq!(current, Some(recovering));
    }

    #[test]
    fn transcript_cursor_is_seeded_from_last_hydrated_sequence() {
        assert_eq!(last_transcript_seq(&[]), 0);
        assert_eq!(last_transcript_seq(&[message(3), message(9)]), 9);
    }

    #[test]
    fn retry_wire_requires_exact_body_and_success_access_envelope() {
        assert_eq!(
            serde_json::to_value(RetryOutboxBody {
                client_event_id: "client-1"
            })
            .unwrap(),
            serde_json::json!({ "client_event_id": "client-1" })
        );

        let success: RetryOutboxSuccess = serde_json::from_value(serde_json::json!({
            "ok": true,
            "access": { "state": "live" }
        }))
        .expect("202 envelope should decode");
        assert!(success.ok);
        assert_eq!(success.access.state, RoomAccessState::Live);
        assert!(
            serde_json::from_value::<RetryOutboxSuccess>(serde_json::json!({ "ok": true }))
                .is_err()
        );
    }

    #[test]
    fn retry_projection_guard_requires_generation_and_room_match() {
        assert!(room_request_is_current(4, 4, "room-1", Some("room-1")));
        assert!(!room_request_is_current(4, 5, "room-1", Some("room-1")));
        assert!(!room_request_is_current(4, 4, "room-1", Some("room-2")));
        assert!(!room_request_is_current(4, 4, "room-1", None));
    }

    /// Regression: a request scheduled while room "A" is open at generation N
    /// must be rejected once "A" is closed and reopened under the SAME key —
    /// which bumps the generation to N+1 without changing `open_key`. Key
    /// equality alone (the pre-fix guard) would wrongly admit this stale
    /// request; `room_request_is_current` — the exact predicate backing the
    /// pub(crate) `Rooms::room_is_current` exposed for `rooms_workspace/` —
    /// must reject it.
    #[test]
    fn room_request_is_current_rejects_stale_generation_across_same_key_close_reopen() {
        let key = "room-a";
        let scheduled_generation = 3; // captured "gen N" while room-a was open

        // Sanity: the schedule-time snapshot is admitted against itself.
        assert!(room_request_is_current(
            scheduled_generation,
            scheduled_generation,
            key,
            Some(key),
        ));

        // Close + reopen the SAME key: generation advances to N+1, `open_key`
        // is still "room-a" — the pre-fix key-only guard would wrongly admit
        // the stale request here.
        let generation_after_close_reopen = scheduled_generation + 1;
        assert!(!room_request_is_current(
            scheduled_generation,
            generation_after_close_reopen,
            key,
            Some(key),
        ));
        // A freshly-stamped request for the new admission is admitted.
        assert!(room_request_is_current(
            generation_after_close_reopen,
            generation_after_close_reopen,
            key,
            Some(key),
        ));
    }

    #[test]
    fn agent_ids_switch_strictly_between_local_and_federated_rosters() {
        let room = local_room();
        let local = access_projection(RoomAccessState::Local);
        assert_eq!(
            agent_ids_for(Some(&local), Some(&room)),
            vec!["local-agent"]
        );
        assert!(agent_ids_for(None, Some(&room)).is_empty());

        let mut federated = access_projection(RoomAccessState::Live);
        federated.members = vec![
            FederatedRoomMemberProjection {
                member_id: "opaque-agent".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::Agent,
                role_in_room: FederatedRoomRole::Member,
                display_name: "Remote Agent".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: None,
                local_binding_available: Some(false),
            },
            FederatedRoomMemberProjection {
                member_id: "opaque-user".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::User,
                role_in_room: FederatedRoomRole::Owner,
                display_name: "User".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: None,
                local_binding_available: Some(true),
            },
        ];
        assert_eq!(
            agent_ids_for(Some(&federated), Some(&room)),
            vec!["opaque-agent"]
        );
    }

    #[test]
    fn joined_open_uses_only_the_authoritative_roster_for_access_mode() {
        let room = local_room();
        let local = access_projection(RoomAccessState::Local);
        assert!(joined_open_for(Some(&local), Some(&room), "local-human"));
        assert!(!joined_open_for(Some(&local), Some(&room), "remote-owner"));
        assert!(!joined_open_for(None, Some(&room), "local-human"));

        let mut federated = access_projection(RoomAccessState::Live);
        federated.members = vec![FederatedRoomMemberProjection {
            member_id: "federated-user".into(),
            owner_member_id: Some("local-human".into()),
            actor_type: FederatedActorType::User,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Federated User".into(),
            public_agent_descriptor: None,
            joined_at: String::new(),
            derived_presence: None,
            local_binding_available: Some(true),
        }];
        // Federated "me" is the daemon-projected member id, never the owner's
        // Local participant id.
        assert!(!joined_open_for(Some(&federated), None, "local-human"));
        federated.local_member_id = Some("federated-user".into());
        assert!(joined_open_for(Some(&federated), None, "local-human"));
        federated.local_member_id = Some("someone-else".into());
        assert!(!joined_open_for(
            Some(&federated),
            Some(&room),
            "local-human"
        ));
    }

    #[test]
    fn author_display_name_prefers_roster_then_member_projection() {
        let room = local_room();
        let local_id = room.participants[0].id.clone();
        let local_name = room.participants[0].display_name.clone();
        assert_eq!(
            author_display_name(Some(&room), None, &local_id),
            local_name
        );
        let mut live = access_projection(RoomAccessState::Live);
        live.members = vec![FederatedRoomMemberProjection {
            member_id: "m-7".into(),
            owner_member_id: None,
            actor_type: FederatedActorType::User,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Grace Hopper".into(),
            public_agent_descriptor: None,
            joined_at: String::new(),
            derived_presence: None,
            local_binding_available: None,
        }];
        assert_eq!(
            author_display_name(None, Some(&live), "m-7"),
            "Grace Hopper"
        );
        assert_eq!(
            author_display_name(Some(&room), Some(&live), "ghost"),
            "ghost"
        );
    }

    #[test]
    fn name_initials_take_first_and_last_words() {
        assert_eq!(name_initials("Ada King"), "AK");
        assert_eq!(name_initials("Grace Brewster Hopper"), "GH");
        assert_eq!(name_initials("researcher"), "RE");
        assert_eq!(name_initials("john-smathers"), "JS");
        assert_eq!(name_initials("   "), "?");
    }

    #[test]
    fn local_speaker_id_follows_access_authority() {
        let local = access_projection(RoomAccessState::Local);
        assert_eq!(local_speaker_id(Some(&local), "ada"), Some("ada".into()));
        assert_eq!(local_speaker_id(Some(&local), ""), None);
        let mut live = access_projection(RoomAccessState::Live);
        assert_eq!(local_speaker_id(Some(&live), "ada"), None);
        live.local_member_id = Some("m-1".into());
        assert_eq!(local_speaker_id(Some(&live), "ada"), Some("m-1".into()));
        assert_eq!(local_speaker_id(None, "ada"), None);
    }

    #[test]
    fn room_list_ticket_is_strictly_latest_request_wins() {
        assert!(list_request_is_current(8, 8));
        assert!(!list_request_is_current(7, 8));
        assert!(!list_request_is_current(8, 9));
    }

    #[test]
    fn outbox_states_keep_pending_and_failed_distinct() {
        assert_eq!(
            serde_json::from_str::<OutboxItemState>(r#""pending""#).unwrap(),
            OutboxItemState::Pending
        );
        assert_eq!(
            serde_json::from_str::<OutboxItemState>(r#""failed""#).unwrap(),
            OutboxItemState::Failed
        );
        assert_eq!(
            serde_json::to_string(&OutboxItemState::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&OutboxItemState::Failed).unwrap(),
            r#""failed""#
        );
    }

    // ── TASK-21 tail guard: stale SSE frames after close/switch ────────
    #[test]
    fn production_frame_boundary_accepts_only_the_current_room_for_both_variants() {
        let frames = [
            RoomTailFrame::Message(message(8)),
            RoomTailFrame::Access(access_projection(RoomAccessState::Recovering)),
        ];

        for frame in frames {
            assert_eq!(
                accept_room_tail_frame(frame.clone(), 4, 4, "room-a", Some("room-a")),
                Some(frame.clone()),
                "current frame must be admitted"
            );
            assert_eq!(
                accept_room_tail_frame(frame.clone(), 4, 5, "room-a", Some("room-a")),
                None,
                "generation-stale frame must be a total no-op"
            );
            assert_eq!(
                accept_room_tail_frame(frame.clone(), 4, 4, "room-a", Some("room-b")),
                None,
                "wrong-room frame must be a total no-op"
            );
            assert_eq!(
                accept_room_tail_frame(frame, 4, 4, "room-a", None),
                None,
                "closed-room frame must be a total no-op"
            );
        }
    }

    #[test]
    fn read_summaries_fail_closed_on_duplicate_room_ids() {
        let duplicate = vec![
            RoomReadStateWire {
                room_id: "room-1".into(),
                latest_seq: Some("7".into()),
                read_seq: Some("3".into()),
            },
            RoomReadStateWire {
                room_id: "room-1".into(),
                latest_seq: Some("8".into()),
                read_seq: Some("4".into()),
            },
        ];
        assert!(read_summaries_from_wire(&duplicate).is_err());
    }

    #[test]
    fn read_summaries_fail_closed_on_malformed_decimal() {
        let malformed = vec![RoomReadStateWire {
            room_id: "room-1".into(),
            latest_seq: Some("oops".into()),
            read_seq: Some("1".into()),
        }];
        assert!(read_summaries_from_wire(&malformed).is_err());
    }

    #[test]
    fn patch_response_parses_canonical_cursor_body_exactly() {
        let local: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "room-1",
                "read_seq": "9"
            }
        }))
        .unwrap();
        assert!(local.ok);
        assert_eq!(
            parse_patch_read_cursor_response("room-1", local.cursor).unwrap(),
            RoomReadCursorProjection {
                read_seq: Some(9),
                mirrored_upstream_read_seq: None,
            }
        );
    }

    #[test]
    fn patch_response_parses_js_safe_decimal_strings_and_null() {
        let big: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "room-1",
                "read_seq": "9007199254740993"
            }
        }))
        .unwrap();
        assert_eq!(
            parse_patch_read_cursor_response("room-1", big.cursor).unwrap(),
            RoomReadCursorProjection {
                read_seq: Some(9_007_199_254_740_993),
                mirrored_upstream_read_seq: None,
            }
        );

        let null: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "room-1",
                "read_seq": null
            }
        }))
        .unwrap();
        assert_eq!(
            parse_patch_read_cursor_response("room-1", null.cursor).unwrap(),
            RoomReadCursorProjection {
                read_seq: None,
                mirrored_upstream_read_seq: None,
            }
        );
    }

    #[test]
    fn patch_response_rejects_bad_decimal_string() {
        let live: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "room-1",
                "read_seq": "NaN"
            }
        }))
        .unwrap();
        assert!(parse_patch_read_cursor_response("room-1", live.cursor).is_err());
    }

    #[test]
    fn patch_response_rejects_wrong_or_empty_room_id() {
        let wrong: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "room-2",
                "read_seq": "44"
            }
        }))
        .unwrap();
        assert!(parse_patch_read_cursor_response("room-1", wrong.cursor).is_err());

        let empty: ReadCursorPatchEnvelope = serde_json::from_value(serde_json::json!({
            "ok": true,
            "cursor": {
                "room_id": "",
                "read_seq": "44"
            }
        }))
        .unwrap();
        assert!(parse_patch_read_cursor_response("room-1", empty.cursor).is_err());
    }

    #[test]
    fn sse_read_cursor_decodes_canonical_wire_and_rejects_malformed_or_wrong_room_id() {
        assert_eq!(
            decode_room_tail_frame(
                "room_read_cursor",
                r#"{"room_id":"room-1","read_seq":"9007199254740993"}"#,
                "room-1",
            ),
            Some(RoomTailFrame::ReadCursor(RoomReadCursorProjection {
                read_seq: None,
                mirrored_upstream_read_seq: Some(9_007_199_254_740_993),
            }))
        );

        assert_eq!(
            decode_room_tail_frame(
                "room_read_cursor",
                r#"{"room_id":"room-1","read_seq":null}"#,
                "room-1",
            ),
            Some(RoomTailFrame::ReadCursor(RoomReadCursorProjection {
                read_seq: None,
                mirrored_upstream_read_seq: None,
            }))
        );

        assert_eq!(
            decode_room_tail_frame(
                "room_read_cursor",
                r#"{"room_id":"room-2","read_seq":"44"}"#,
                "room-1",
            ),
            None
        );
        assert_eq!(
            decode_room_tail_frame(
                "room_read_cursor",
                r#"{"room_id":"","read_seq":"44"}"#,
                "room-1",
            ),
            None
        );
        assert_eq!(
            decode_room_tail_frame(
                "room_read_cursor",
                r#"{"room_id":"room-1","read_seq":"oops"}"#,
                "room-1",
            ),
            None
        );
    }

    #[test]
    fn open_hydration_preserves_existing_read_seq_until_cursor_arrives() {
        let summaries = RwSignal::new(HashMap::from([(
            "room-1".to_string(),
            RoomReadSummary {
                latest_seq: Some(3),
                read_seq: Some(2),
            },
        )]));
        let transcript = vec![message(7)];
        let access = access_projection(RoomAccessState::Local);

        update_open_summary_from_open_room(
            &summaries,
            Some("room-1"),
            &transcript,
            Some(&access),
            None,
        );

        assert_eq!(
            summaries.get_untracked().get("room-1"),
            Some(&RoomReadSummary {
                latest_seq: Some(7),
                read_seq: Some(2),
            })
        );
    }

    #[test]
    fn lagging_mirrored_sse_cursor_cannot_lower_local_confirmed_read() {
        // Local PATCH confirms read 100.
        let local = parse_patch_read_cursor_response(
            "room-1",
            RoomReadCursorBody {
                room_id: "room-1".into(),
                read_seq: Some("100".into()),
            },
        )
        .unwrap();
        assert_eq!(
            local,
            RoomReadCursorProjection {
                read_seq: Some(100),
                mirrored_upstream_read_seq: None,
            }
        );
        assert_eq!(current_durable_read_seq(&local), Some(100));

        // A lagging mirrored SSE frame reports 90.
        let Some(RoomTailFrame::ReadCursor(lagging)) = decode_room_tail_frame(
            "room_read_cursor",
            r#"{"room_id":"room-1","read_seq":"90"}"#,
            "room-1",
        ) else {
            panic!("mirrored cursor frame should decode");
        };
        let merged = merge_read_cursor_projection(Some(&local), lagging);
        assert_eq!(
            merged,
            RoomReadCursorProjection {
                read_seq: Some(100),
                mirrored_upstream_read_seq: Some(90),
            }
        );
        assert_eq!(current_durable_read_seq(&merged), Some(100));

        // The room summary keeps the confirmed read; unread stays cleared.
        let summaries = RwSignal::new(HashMap::from([(
            "room-1".to_string(),
            RoomReadSummary {
                latest_seq: Some(100),
                read_seq: Some(100),
            },
        )]));
        update_open_summary_from_open_room(
            &summaries,
            Some("room-1"),
            &[message(100)],
            Some(&access_projection(RoomAccessState::Local)),
            Some(&merged),
        );
        assert_eq!(
            summaries.get_untracked().get("room-1"),
            Some(&RoomReadSummary {
                latest_seq: Some(100),
                read_seq: Some(100),
            })
        );
        assert!(!room_has_durable_unread(
            summaries.get_untracked().get("room-1")
        ));

        // A later, higher mirrored frame still corrects the durable read up.
        let Some(RoomTailFrame::ReadCursor(ahead)) = decode_room_tail_frame(
            "room_read_cursor",
            r#"{"room_id":"room-1","read_seq":"110"}"#,
            "room-1",
        ) else {
            panic!("mirrored cursor frame should decode");
        };
        let corrected = merge_read_cursor_projection(Some(&merged), ahead);
        assert_eq!(
            corrected,
            RoomReadCursorProjection {
                read_seq: Some(100),
                mirrored_upstream_read_seq: Some(110),
            }
        );
        assert_eq!(current_durable_read_seq(&corrected), Some(110));

        update_open_summary_from_open_room(
            &summaries,
            Some("room-1"),
            &[message(110)],
            Some(&access_projection(RoomAccessState::Local)),
            Some(&corrected),
        );
        assert_eq!(
            summaries.get_untracked().get("room-1"),
            Some(&RoomReadSummary {
                latest_seq: Some(110),
                read_seq: Some(110),
            })
        );
        assert!(!room_has_durable_unread(
            summaries.get_untracked().get("room-1")
        ));
    }

    #[test]
    fn read_cursor_merge_seeds_from_empty_and_never_clears_known_positions() {
        let mirrored = RoomReadCursorProjection {
            read_seq: None,
            mirrored_upstream_read_seq: Some(7),
        };
        assert_eq!(
            merge_read_cursor_projection(None, mirrored.clone()),
            mirrored
        );

        // An empty (null read_seq) frame cannot erase either known position.
        let known = RoomReadCursorProjection {
            read_seq: Some(12),
            mirrored_upstream_read_seq: Some(9),
        };
        let Some(RoomTailFrame::ReadCursor(empty)) = decode_room_tail_frame(
            "room_read_cursor",
            r#"{"room_id":"room-1","read_seq":null}"#,
            "room-1",
        ) else {
            panic!("null cursor frame should decode");
        };
        assert_eq!(merge_read_cursor_projection(Some(&known), empty), known);
    }

    #[test]
    fn applied_open_read_seq_folds_summary_and_durable_cursor_monotonically() {
        // Absent on both sides keeps the historical zero floor.
        assert_eq!(applied_open_read_seq(None, None), 0);
        // Either side alone still applies.
        assert_eq!(applied_open_read_seq(Some(5), None), 5);
        assert_eq!(applied_open_read_seq(None, Some(9)), 9);
        // A lagging summary can no longer mask a further durable cursor.
        assert_eq!(applied_open_read_seq(Some(5), Some(100)), 100);
        // A further summary still wins over a lagging durable cursor.
        assert_eq!(applied_open_read_seq(Some(100), Some(5)), 100);
    }

    #[test]
    fn read_cursor_clamped_acknowledgement_allows_retry_until_confirmed() {
        let request = ReadCursorRequest {
            ticket: 1,
            read_seq: 100,
        };
        let initial = RoomReadCursorProjection {
            read_seq: Some(50),
            mirrored_upstream_read_seq: None,
        };
        assert!(should_send_read_cursor(
            100,
            Some(50),
            current_durable_read_seq(&initial),
            None
        ));
        assert!(!should_send_read_cursor(
            100,
            Some(50),
            current_durable_read_seq(&initial),
            Some(request)
        ));

        // A successful PATCH can acknowledge less than the visible candidate.
        let ack =
            serde_json::from_str::<RoomReadCursorBody>(r#"{"room_id":"room-1","read_seq":"80"}"#)
                .unwrap();
        let cursor = merge_read_cursor_projection(
            Some(&initial),
            parse_patch_read_cursor_response("room-1", ack).unwrap(),
        );
        let in_flight = finish_read_cursor_request(Some(request), request);
        assert!(should_send_read_cursor(
            100,
            Some(80),
            current_durable_read_seq(&cursor),
            in_flight
        ));

        let ack =
            serde_json::from_str::<RoomReadCursorBody>(r#"{"room_id":"room-1","read_seq":"100"}"#)
                .unwrap();
        let cursor = merge_read_cursor_projection(
            Some(&cursor),
            parse_patch_read_cursor_response("room-1", ack).unwrap(),
        );
        assert!(!should_send_read_cursor(
            100,
            Some(80),
            current_durable_read_seq(&cursor),
            None
        ));
    }

    #[test]
    fn read_cursor_completion_cannot_clear_newer_or_same_sequence_retry() {
        let old = ReadCursorRequest {
            ticket: 1,
            read_seq: 100,
        };
        let newer = ReadCursorRequest {
            ticket: 2,
            read_seq: 101,
        };
        assert_eq!(finish_read_cursor_request(Some(newer), old), Some(newer));
        assert!(!should_send_read_cursor(
            101,
            Some(80),
            Some(80),
            Some(newer)
        ));

        // After a newer clamped ACK completes, the old candidate can be retried
        // while its first request is still on the wire. Sequence alone cannot
        // identify which request owns the in-flight slot.
        assert_eq!(finish_read_cursor_request(Some(newer), newer), None);
        let retry = ReadCursorRequest {
            ticket: 3,
            read_seq: 100,
        };
        assert_eq!(finish_read_cursor_request(Some(retry), old), Some(retry));
        assert!(!should_send_read_cursor(
            100,
            Some(80),
            Some(80),
            Some(retry)
        ));
        assert_eq!(finish_read_cursor_request(Some(retry), retry), None);
        assert!(should_send_read_cursor(100, Some(80), Some(80), None));
    }

    #[test]
    fn invite_redemption_is_bound_to_exact_accepted_intent_and_navigation() {
        let invite_a = PendingRoomInvite {
            room_key: "link-a".into(),
            code: "code-a".into(),
        };
        let accepted_a =
            begin_pending_invite_redemption(invite_a.clone(), 1, 7, Some(&invite_a), None).unwrap();
        assert!(pending_invite_completion_is_current(
            &accepted_a,
            1,
            7,
            Some(&invite_a),
            Some(&accepted_a)
        ));
        assert!(begin_pending_invite_redemption(
            invite_a.clone(),
            1,
            7,
            Some(&invite_a),
            Some(&accepted_a)
        )
        .is_none());

        // B is staged and accepted while A's request is still running.
        let invite_b = PendingRoomInvite {
            room_key: "link-b".into(),
            code: "code-b".into(),
        };
        let accepted_b = begin_pending_invite_redemption(
            invite_b.clone(),
            2,
            7,
            Some(&invite_b),
            Some(&accepted_a),
        )
        .unwrap();
        assert!(!pending_invite_completion_is_current(
            &accepted_a,
            2,
            7,
            Some(&invite_b),
            Some(&accepted_b)
        ));
        assert!(pending_invite_completion_is_current(
            &accepted_b,
            2,
            7,
            Some(&invite_b),
            Some(&accepted_b)
        ));
        assert!(
            begin_pending_invite_redemption(invite_a.clone(), 2, 7, Some(&invite_b), None)
                .is_none()
        );

        // Dismissal, same-value restaging, and same-room navigation each revoke
        // the old completion's right to clear the prompt, status, or open room.
        assert!(!pending_invite_completion_is_current(
            &accepted_a,
            2,
            7,
            None,
            None
        ));
        let accepted_again =
            begin_pending_invite_redemption(invite_a.clone(), 3, 7, Some(&invite_a), None).unwrap();
        assert!(!pending_invite_completion_is_current(
            &accepted_a,
            3,
            7,
            Some(&invite_a),
            Some(&accepted_again)
        ));
        assert!(!pending_invite_completion_is_current(
            &accepted_again,
            3,
            8,
            Some(&invite_a),
            Some(&accepted_again)
        ));
        assert!(!pending_invite_completion_is_current(
            &accepted_again,
            3,
            7,
            Some(&invite_a),
            None
        ));
    }

    #[test]
    fn access_caller_projection_defaults_legacy_wire_and_tracks_principal_changes() {
        let legacy: RoomAccessProjection = serde_json::from_str(r#"{"state":"live"}"#).unwrap();
        assert_eq!(legacy.caller_member_id, None);
        assert!(serde_json::to_value(&legacy)
            .unwrap()
            .get("caller_member_id")
            .is_none());
        let mut current = legacy.clone();
        current.caller_member_id = Some("member-owner".into());
        assert_ne!(current, legacy);
        assert_eq!(
            serde_json::to_value(&current).unwrap()["caller_member_id"],
            "member-owner"
        );
        let mut replaced = current.clone();
        replaced.caller_member_id = Some("member-other".into());
        assert_ne!(current, replaced);
    }

    #[test]
    fn merge_room_read_summaries_is_monotonic_and_removes_deleted_rooms() {
        let current = HashMap::from([
            (
                "room-1".to_string(),
                RoomReadSummary {
                    latest_seq: Some(9),
                    read_seq: Some(4),
                },
            ),
            (
                "room-2".to_string(),
                RoomReadSummary {
                    latest_seq: Some(8),
                    read_seq: Some(6),
                },
            ),
        ]);
        let incoming = HashMap::from([(
            "room-1".to_string(),
            RoomReadSummary {
                latest_seq: Some(5),
                read_seq: None,
            },
        )]);
        let rooms = vec![Room {
            id: "room-1".into(),
            name: "Room One".into(),
            participants: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
            trigger_policy: None,
            muted: false,
        }];

        let merged = merge_room_read_summaries(&current, &rooms, &incoming);

        assert_eq!(
            merged.get("room-1"),
            Some(&RoomReadSummary {
                latest_seq: Some(9),
                read_seq: Some(4),
            })
        );
        assert!(!merged.contains_key("room-2"));
    }

    #[test]
    fn silent_fetch_skips_during_interactive_loading_and_cleanup_is_ticket_safe() {
        assert!(should_skip_rooms_fetch(RoomsFetchMode::Silent, true));
        assert!(!should_skip_rooms_fetch(RoomsFetchMode::Silent, false));
        assert!(!should_skip_rooms_fetch(RoomsFetchMode::Interactive, true));

        let rooms_loaded = RwSignal::new(false);
        let rooms_loading = RwSignal::new(true);
        finish_rooms_fetch(
            &rooms_loaded,
            &rooms_loading,
            RoomsFetchMode::Interactive,
            false,
        );
        assert!(!rooms_loaded.get_untracked());
        assert!(rooms_loading.get_untracked());

        finish_rooms_fetch(
            &rooms_loaded,
            &rooms_loading,
            RoomsFetchMode::Interactive,
            true,
        );
        assert!(rooms_loaded.get_untracked());
        assert!(!rooms_loading.get_untracked());
    }

    #[test]
    fn owner_responses_retire_on_origin_switch_and_newer_read() {
        let url = RwSignal::new("https://old.example".into());
        let rooms = Rooms::with_url(url);
        let owner = |id: &str, name: &str| OwnerIdentity {
            participant_id: id.into(),
            display_name: name.into(),
        };
        let (old_base, old_ticket) = rooms.begin_owner_request();
        assert!(rooms.apply_owner_response(&old_base, old_ticket, owner("old", "Old")));

        url.set("https://new.example".into());
        // Reject even before the origin effect starts its replacement request.
        assert!(!rooms.apply_owner_response(&old_base, old_ticket, owner("old", "Late")));
        let (new_base, first_ticket) = rooms.begin_owner_request();
        assert_eq!(rooms.identity_id.get_untracked(), "");
        assert_eq!(rooms.identity_name.get_untracked(), "");
        let (_, latest_ticket) = rooms.begin_owner_request();
        assert!(rooms.apply_owner_response(&new_base, latest_ticket, owner("new", "Renamed")));
        assert!(!rooms.apply_owner_response(
            &new_base,
            first_ticket,
            owner("new", "Before rename")
        ));
        assert_eq!(rooms.identity_id.get_untracked(), "new");
        assert_eq!(rooms.identity_name.get_untracked(), "Renamed");

        // Returning to an origin does not revive its previous request ticket.
        url.set(old_base.clone());
        let (_, return_ticket) = rooms.begin_owner_request();
        assert!(!rooms.apply_owner_response(&old_base, old_ticket, owner("old", "Late")));
        assert!(rooms.apply_owner_response(&old_base, return_ticket, owner("old", "Current")));
    }

    #[test]
    fn attention_reads_retire_after_origin_room_query_or_ticket_changes() {
        use crate::room_attention::AttentionResponseFence;
        let owner = Owner::new();
        owner.set();
        let url = RwSignal::new("https://first.example".to_string());
        let rooms = Rooms::with_url(url);
        let ticket = RwSignal::new(1_u64);
        let slot = RwSignal::new(None::<String>);
        rooms.open_key.set(Some("room-a".into()));
        let fence = AttentionResponseFence {
            origin: url.get_untracked(),
            room: Some(("room-a".into(), rooms.generation_snapshot())),
            query: Some("needle".into()),
            ticket: 1,
        };
        assert!(fence.publish(rooms, ticket, Some("needle".into()), slot, "current".into()));
        url.set("https://second.example".into());
        assert!(!fence.publish(
            rooms,
            ticket,
            Some("needle".into()),
            slot,
            "wrong daemon".into()
        ));
        url.set(fence.origin.clone());
        rooms.generation.update(|g| *g += 1);
        assert!(!fence.publish(
            rooms,
            ticket,
            Some("needle".into()),
            slot,
            "old admission".into()
        ));
        let current = AttentionResponseFence {
            room: Some(("room-a".into(), rooms.generation_snapshot())),
            ..fence
        };
        assert!(!current.publish(rooms, ticket, Some("n".into()), slot, "old query".into()));
        ticket.set(2);
        assert!(!current.publish(
            rooms,
            ticket,
            Some("needle".into()),
            slot,
            "older search".into()
        ));
        let inbox = AttentionResponseFence {
            origin: url.get_untracked(),
            room: None,
            query: None,
            ticket: 2,
        };
        assert!(inbox.publish(rooms, ticket, None, slot, "current inbox".into()));
        ticket.set(3); // closing/reopening the panel retires its former read
        assert!(!inbox.publish(rooms, ticket, None, slot, "closed inbox".into()));
        assert_eq!(slot.get_untracked(), Some("current inbox".into()));
    }

    #[test]
    fn inbox_thread_pick_waits_for_its_room_and_retires_on_origin_switch() {
        let owner = Owner::new();
        owner.set();
        let url = RwSignal::new("https://first.example".to_string());
        let rooms = Rooms::with_url(url);
        let message: RoomMessage = serde_json::from_str(r#"{"seq":7,"author_id":"ada","author_kind":"human","kind":"message","body":"root","created_at":"2026-10-04T05:33:00Z"}"#).unwrap();
        rooms.transcript.set(vec![message]);
        rooms.open_key.set(Some("previous".into()));
        rooms.open_room.set(Some(
            serde_json::from_str(r#"{"id":"previous","name":"Previous","participants":[]}"#)
                .unwrap(),
        ));
        rooms
            .focus_thread
            .set(Some((url.get_untracked(), "target".into(), 7)));
        assert_eq!(
            rooms.take_pending_thread_focus(),
            None,
            "same seq in another room cannot consume the pick"
        );
        rooms.open_key.set(Some("target".into()));
        assert_eq!(
            rooms.take_pending_thread_focus(),
            None,
            "the previous room record is still visible"
        );
        rooms.open_room.set(Some(
            serde_json::from_str(r#"{"id":"target","name":"Target","participants":[]}"#).unwrap(),
        ));
        assert_eq!(rooms.take_pending_thread_focus(), Some(7));
        assert_eq!(rooms.focus_thread.get_untracked(), None);
        rooms
            .focus_thread
            .set(Some((url.get_untracked(), "target".into(), 7)));
        url.set("https://second.example".into());
        assert_eq!(rooms.take_pending_thread_focus(), None);
        assert_eq!(rooms.focus_thread.get_untracked(), None);
    }

    #[test]
    fn unread_dot_helper_requires_latest_ahead_of_read() {
        assert!(room_has_durable_unread(Some(&RoomReadSummary {
            latest_seq: Some(5),
            read_seq: Some(4),
        })));
        assert!(room_has_durable_unread(Some(&RoomReadSummary {
            latest_seq: Some(5),
            read_seq: None,
        })));
        assert!(!room_has_durable_unread(Some(&RoomReadSummary {
            latest_seq: Some(5),
            read_seq: Some(5),
        })));
    }
}
