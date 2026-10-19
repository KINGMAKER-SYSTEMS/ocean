//! Native Room agent consent broker (ocean-private #65).
//!
//! The daemon gates Room agent authorize/reauthorize/revoke on its header-only
//! operator credential (`X-Ocean-Operator`). This module is the only place the
//! desktop shell attaches that credential for Room agent consent, and it keeps
//! the whole decision native:
//!
//! * The page names a room and a package. Rust fetches the authoritative
//!   preview itself and freezes the exact room, package, agent, owner, digest,
//!   policies and a fresh decision id in native memory. The page receives a
//!   projection plus an opaque consent id; it can never supply the digest.
//! * Authorization and revoke need native confirmation. Cancellation received
//!   before native dispatch is claimed sends no decision request; after that
//!   claim, I/O may finish but cancellation prevents retry-state resurrection.
//! * Authorize/reauthorize always carry the frozen non-null
//!   `expected_definition_digest`; a stale digest is reported as refused with
//!   no legacy retry.
//! * Requests are built only from the typed [`Intent`]s below (no page-supplied
//!   method, path, header or body) and go through the shared
//!   `daemon_transport.rs`, which owns every transport and custody rule:
//!   numeric-loopback daemon only, raw HTTP/1.1 with no proxy routing, no
//!   redirects, replies bounded to 256 KiB JSON objects, and the key read
//!   through a no-follow descriptor immediately before each privileged
//!   request (owner-only, effective-uid owned, never cached, returned, logged
//!   or embedded in an error).
//! * Local rooms record their owner through the operator-gated bootstrap
//!   ([`Intent::Bootstrap`]). The owner is the daemon's own `GET /v1/me`
//!   `participant_id`, read natively; the page names only room and package.
//!   A bootstrap creates no binding: it lands in the same frozen preview, so
//!   authorization still needs the native dialog.
//!
//! The pure broker below owns every check; the `#[tauri::command]` glue at the
//! bottom only adds the native dialog and the main-window gate (Tauri plugin
//! ACLs do not cover application commands).

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::daemon_transport::{
    encode_segment, require_main_window, DaemonRequest, DaemonTransport, Method, Reply, SendError,
};

/// Pending decisions held at once; a preview may evict the oldest idle one.
const MAX_PENDING: usize = 4;
/// A frozen preview older than this must be fetched again.
const CONSENT_TTL: Duration = Duration::from_secs(120);
const MAX_ID_CHARS: usize = 512;
/// Longest page- or package-supplied label shown in a native dialog.
const MAX_DIALOG_LABEL_CHARS: usize = 128;
/// Longest capability line shown in the authorize dialog.
const MAX_DIALOG_CAPS_CHARS: usize = 1024;

const ACTIVATION_DEFAULT: &str = "explicit_only";
const CONTEXT_DEFAULT: &str = "invocation_only";
const MEMORY_DEFAULT: &str = "none";

// ── wire projections returned to the page ───────────────────────────────

/// Frozen preview projection. Carries no credential material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ConsentPreview {
    pub(crate) consent_id: String,
    pub(crate) room: String,
    pub(crate) package_id: String,
    pub(crate) agent_member_id: String,
    pub(crate) owner_member_id: String,
    pub(crate) display_name: String,
    pub(crate) definition_digest: String,
    pub(crate) definition_revision: Option<String>,
    pub(crate) requested_capabilities: Vec<String>,
    pub(crate) mode: ConsentMode,
    pub(crate) binding_status: Option<String>,
    pub(crate) expires_in_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConsentMode {
    Authorize,
    Reauthorize,
}

/// Result of one privileged decision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ConsentOutcome {
    pub(crate) state: OutcomeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) binding: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutcomeState {
    /// The daemon committed the decision.
    Applied,
    /// The native confirmation was declined; nothing was sent.
    Declined,
    /// The daemon answered with a refusal; nothing changed.
    Refused,
    /// The request may have reached the daemon but no reply arrived. Never
    /// retried automatically; an explicit retry reuses the same decision id.
    Unknown,
}

impl ConsentOutcome {
    fn declined() -> Self {
        Self {
            state: OutcomeState::Declined,
            status: None,
            code: None,
            binding: None,
        }
    }
}

// ── frozen native state ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frozen {
    room: String,
    package_id: String,
    agent_member_id: String,
    owner_member_id: String,
    display_name: String,
    digest: String,
    revision: Option<String>,
    requested: Vec<String>,
    mode: ConsentMode,
    binding_status: Option<String>,
    decision_id: String,
    activation_policy: String,
    context_policy: String,
    memory_scope: String,
}

impl Frozen {
    /// Text for the native confirmation dialog. Every line comes from the
    /// natively frozen preview, never page input: which agent (package, member
    /// id, owner), where, what it asks for and the exact digest. The display
    /// name is a label only and is sanitized; ids are already dialog-safe.
    pub(crate) fn dialog_text(&self) -> (String, String, &'static str) {
        let action = match self.mode {
            ConsentMode::Authorize => "Authorize",
            ConsentMode::Reauthorize => "Reauthorize",
        };
        let title = dialog_label(&self.display_name, MAX_DIALOG_LABEL_CHARS);
        let title = if title.is_empty() {
            self.package_id.clone()
        } else {
            title
        };
        let message = format!(
            "Room: {}\nPackage: {}\nAgent id: {}\nOwner: {}\nCapabilities: {}\nDigest: {}",
            self.room,
            self.package_id,
            self.agent_member_id,
            self.owner_member_id,
            capability_line(&self.requested),
            self.digest,
        );
        (title, message, action)
    }

    fn intent(&self) -> Intent {
        let common = |map: &mut Map<String, Value>| {
            map.insert("decision_id".into(), json!(self.decision_id));
            map.insert("expected_definition_digest".into(), json!(self.digest));
            map.insert("activation_policy".into(), json!(self.activation_policy));
            map.insert("context_policy".into(), json!(self.context_policy));
            map.insert("memory_scope".into(), json!(self.memory_scope));
            // Phase 1 grants nothing beyond the operator intersection.
            map.insert("room_capability_grants".into(), json!([]));
        };
        let mut body = Map::new();
        common(&mut body);
        match self.mode {
            ConsentMode::Authorize => {
                body.insert("agent_member_id".into(), json!(self.agent_member_id));
                body.insert("agent_package_id".into(), json!(self.package_id));
                body.insert("owner_member_id".into(), json!(self.owner_member_id));
                Intent::Authorize {
                    room: self.room.clone(),
                    body: Value::Object(body),
                }
            }
            ConsentMode::Reauthorize => Intent::Reauthorize {
                room: self.room.clone(),
                agent: self.agent_member_id.clone(),
                body: Value::Object(body),
            },
        }
    }

    fn projection(&self, consent_id: &str, remaining: Duration) -> ConsentPreview {
        ConsentPreview {
            consent_id: consent_id.to_string(),
            room: self.room.clone(),
            package_id: self.package_id.clone(),
            agent_member_id: self.agent_member_id.clone(),
            owner_member_id: self.owner_member_id.clone(),
            display_name: self.display_name.clone(),
            definition_digest: self.digest.clone(),
            definition_revision: self.revision.clone(),
            requested_capabilities: self.requested.clone(),
            mode: self.mode,
            binding_status: self.binding_status.clone(),
            expires_in_ms: remaining.as_millis() as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Confirming,
    Queued,
    Sending,
}

#[derive(Debug)]
struct Entry {
    id: String,
    decision: Decision,
    created: Instant,
    phase: Phase,
}

/// One lock owns the cancellation epoch and every pending native decision.
#[derive(Default)]
struct Pending {
    epoch: Arc<()>,
    entries: Vec<Entry>,
}

#[derive(Debug)]
enum Decision {
    Consent(Box<Frozen>),
    Revoke {
        room: String,
        agent: String,
        decision_id: String,
    },
}

impl Decision {
    fn target(&self) -> (&str, &str) {
        match self {
            Self::Consent(frozen) => (&frozen.room, &frozen.agent_member_id),
            Self::Revoke { room, agent, .. } => (room, agent),
        }
    }

    fn intent(&self) -> Intent {
        match self {
            Self::Consent(frozen) => frozen.intent(),
            Self::Revoke {
                room,
                agent,
                decision_id,
            } => Intent::Revoke {
                room: room.clone(),
                agent: agent.clone(),
                decision_id: decision_id.clone(),
            },
        }
    }
}

// ── typed intents: the only requests this module can build ──────────────

#[derive(Debug, Clone, PartialEq)]
enum Intent {
    Preview {
        room: String,
        package_id: String,
    },
    Authorize {
        room: String,
        body: Value,
    },
    Reauthorize {
        room: String,
        agent: String,
        body: Value,
    },
    Revoke {
        room: String,
        agent: String,
        decision_id: String,
    },
    /// `GET /v1/me`: the daemon owner a Local bootstrap names. Unprivileged.
    Me,
    /// Local-room owner/agent bootstrap. `owner` is only ever the
    /// `participant_id` this module read from [`Intent::Me`]; the page never
    /// supplies it. Creates no binding.
    Bootstrap {
        room: String,
        package_id: String,
        owner: String,
    },
}

impl Intent {
    /// The fixed request for this intent. Only consent decisions carry the
    /// operator credential; the preview is unprivileged.
    fn request(&self) -> DaemonRequest {
        match self {
            Self::Preview { .. } | Self::Me => {
                DaemonRequest::public(Method::Get, self.path(), None)
            }
            _ => DaemonRequest::operator(Method::Post, self.path(), self.body()),
        }
    }

    fn path(&self) -> String {
        let room = |room: &str| format!("/v1/rooms/persistent/{}", encode_segment(room));
        match self {
            Self::Preview {
                room: r,
                package_id,
            } => {
                format!("{}/agents/preview/{}", room(r), encode_segment(package_id))
            }
            Self::Authorize { room: r, .. } => format!("{}/agents", room(r)),
            Self::Reauthorize { room: r, agent, .. } => {
                format!("{}/agents/{}/reauthorize", room(r), encode_segment(agent))
            }
            Self::Revoke { room: r, agent, .. } => {
                format!("{}/agents/{}/revoke", room(r), encode_segment(agent))
            }
            Self::Me => "/v1/me".to_string(),
            Self::Bootstrap { room: r, .. } => format!("{}/agents/bootstrap", room(r)),
        }
    }

    fn body(&self) -> Option<String> {
        match self {
            Self::Preview { .. } | Self::Me => None,
            // Exactly the daemon's strict `{owner_member_id, agent_package_id}`.
            Self::Bootstrap {
                package_id, owner, ..
            } => Some(
                json!({ "owner_member_id": owner, "agent_package_id": package_id }).to_string(),
            ),
            Self::Authorize { body, .. } | Self::Reauthorize { body, .. } => Some(body.to_string()),
            Self::Revoke { decision_id, .. } => {
                Some(json!({ "decision_id": decision_id }).to_string())
            }
        }
    }
}

// ── native dialog text ──────────────────────────────────────────────────

/// Characters that can forge or reorder dialog lines: C0/C1 controls
/// (including CR/LF and tab), Unicode line/paragraph separators, bidi
/// embeddings/overrides/isolates, zero-width and other invisible formatting.
fn dialog_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// A label for a native dialog: unsafe characters removed, whitespace
/// trimmed, at most `max` characters (an ellipsis marks truncation).
fn dialog_label(raw: &str, max: usize) -> String {
    let clean: String = raw.chars().filter(|c| !dialog_unsafe(*c)).collect();
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_string();
    }
    let mut out: String = clean.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn capability_line(requested: &[String]) -> String {
    if requested.is_empty() {
        return "none".into();
    }
    let mut line = String::new();
    for (shown, cap) in requested.iter().enumerate() {
        let next = if line.is_empty() {
            cap.clone()
        } else {
            format!(", {cap}")
        };
        if line.chars().count() + next.chars().count() > MAX_DIALOG_CAPS_CHARS {
            line.push_str(&format!(" (+{} more)", requested.len() - shown));
            return line;
        }
        line.push_str(&next);
    }
    line
}

/// Native revoke confirmation. The room and agent member id are the exact
/// values the revoke request will carry (both already validated dialog-safe);
/// the page-supplied display name is a sanitized label only.
fn revoke_dialog_text(room: &str, agent_member_id: &str, display_name: &str) -> (String, String) {
    let label = dialog_label(display_name, MAX_DIALOG_LABEL_CHARS);
    let title = if label.is_empty() {
        "Revoke agent".to_string()
    } else {
        format!("Revoke {label}")
    };
    (title, format!("Room: {room}\nAgent id: {agent_member_id}"))
}

// ── preview validation ──────────────────────────────────────────────────

/// Room keys use alphanumeric, `-`, `_` and `.`; package ids are agent folder
/// names drawn from the same alphabet.
fn valid_key(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 128
        && raw != "."
        && raw != ".."
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A member id the shell will send and show verbatim: non-empty, bounded,
/// untrimmed-equal and free of characters that could forge dialog lines.
fn bounded_id(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (!text.trim().is_empty()
        && text.chars().count() <= MAX_ID_CHARS
        && text.trim() == text
        && !text.chars().any(dialog_unsafe))
    .then(|| text.to_string())
}

fn valid_digest(raw: &str) -> bool {
    raw.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn policy(binding: Option<&Map<String, Value>>, field: &str, default: &str) -> String {
    binding
        .and_then(|b| b.get(field))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .unwrap_or(default)
        .to_string()
}

fn freeze_preview(
    room: &str,
    package_id: &str,
    body: &Map<String, Value>,
    decision_id: String,
) -> Result<Frozen, &'static str> {
    const BAD: &str = "invalid_preview";
    if body.get("ok") != Some(&Value::Bool(true)) {
        return Err(BAD);
    }
    if body.get("package_id").and_then(Value::as_str) != Some(package_id) {
        return Err(BAD);
    }
    let digest = body
        .get("definition_digest")
        .and_then(Value::as_str)
        .filter(|d| valid_digest(d))
        .ok_or(BAD)?
        .to_string();
    if body.get("owner_eligible") != Some(&Value::Bool(true)) {
        return Err("owner_not_eligible");
    }
    let agent_member_id = bounded_id(body.get("agent_member_id")).ok_or("owner_not_eligible")?;
    let owner_member_id = bounded_id(body.get("owner_member_id")).ok_or("owner_not_eligible")?;
    let display_name = body
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| name.chars().count() <= 256)
        .map(str::to_string)
        .unwrap_or_else(|| package_id.to_string());
    let revision = match body.get("definition_revision") {
        None | Some(Value::Null) => None,
        Some(Value::String(rev)) if rev.chars().count() <= 256 => Some(rev.clone()),
        Some(_) => return Err(BAD),
    };
    let requested = match body.get("requested_capabilities") {
        Some(Value::Array(items)) if items.len() <= 128 => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(dialog_unsafe))
                    .map(str::to_string)
                    .ok_or(BAD)
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(BAD),
    };
    let binding = match body.get("binding") {
        None | Some(Value::Null) => None,
        Some(Value::Object(binding)) => Some(binding),
        Some(_) => return Err(BAD),
    };
    if let Some(binding) = binding {
        if binding.get("agent_member_id").and_then(Value::as_str) != Some(&agent_member_id) {
            return Err(BAD);
        }
    }
    Ok(Frozen {
        room: room.to_string(),
        package_id: package_id.to_string(),
        agent_member_id,
        owner_member_id,
        display_name,
        digest,
        revision,
        requested,
        mode: if binding.is_some() {
            ConsentMode::Reauthorize
        } else {
            ConsentMode::Authorize
        },
        binding_status: binding
            .and_then(|b| b.get("status"))
            .and_then(Value::as_str)
            .map(str::to_string),
        decision_id,
        activation_policy: policy(binding, "activation_policy", ACTIVATION_DEFAULT),
        context_policy: policy(binding, "context_policy", CONTEXT_DEFAULT),
        memory_scope: policy(binding, "memory_scope", MEMORY_DEFAULT),
    })
}

fn outcome_from_reply(reply: Reply) -> ConsentOutcome {
    let binding = reply.body.get("binding").filter(|b| b.is_object()).cloned();
    if (200..300).contains(&reply.status) && reply.body.get("ok") == Some(&Value::Bool(true)) {
        ConsentOutcome {
            state: OutcomeState::Applied,
            status: Some(reply.status),
            code: None,
            binding,
        }
    } else {
        ConsentOutcome {
            state: OutcomeState::Refused,
            status: Some(reply.status),
            code: reply
                .body
                .get("error")
                .and_then(Value::as_str)
                .filter(|c| c.len() <= 64)
                .map(str::to_string)
                .or_else(|| Some("refused".into())),
            binding: None,
        }
    }
}

fn refused(code: &str) -> ConsentOutcome {
    ConsentOutcome {
        state: OutcomeState::Refused,
        status: None,
        code: Some(code.into()),
        binding: None,
    }
}

/// A bootstrap whose existing Room owner is not this daemon's owner.
const BOOTSTRAP_OWNER_CONFLICT: &str = "room_owner_conflict";

/// Map a definitive bootstrap reply. `Ok` proceeds to the preview: created
/// (201), already bootstrapped (200), or an owner conflict the preview then
/// settles against `/v1/me`. Every refusal is one fixed code.
fn bootstrap_step(reply: &Reply) -> Result<(), &'static str> {
    let code = reply.body.get("error").and_then(Value::as_str);
    match (reply.status, code) {
        (200 | 201, _) if reply.body.get("ok") == Some(&Value::Bool(true)) => Ok(()),
        (409, Some("room_owner_conflict")) => Ok(()),
        (403, Some("room_owner_required")) => Err("room_owner_required"),
        (409, Some("local_room_required")) => Err("local_room_required"),
        (409, Some("bootstrap_target_conflict")) => Err("bootstrap_target_conflict"),
        (404, Some("room_not_found")) => Err("room_not_found"),
        (404, Some("agent_package_not_found")) | (400, Some("invalid_agent_package_id")) => {
            Err("agent_package_not_found")
        }
        _ => Err("bootstrap_refused"),
    }
}

fn fresh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── broker ──────────────────────────────────────────────────────────────

pub(crate) struct ConsentBroker {
    transport: DaemonTransport,
    ttl: Duration,
    pending: Mutex<Pending>,
}

impl ConsentBroker {
    pub(crate) fn new(transport: DaemonTransport) -> Self {
        Self {
            transport,
            ttl: CONSENT_TTL,
            pending: Mutex::new(Pending::default()),
        }
    }

    /// Refuse every consent command when `OCEAN_DAEMON_URL` is not a numeric
    /// loopback daemon.
    fn endpoint(&self) -> Result<(), &'static str> {
        self.transport
            .endpoint()
            .map(|_| ())
            .ok_or("consent_unavailable")
    }

    /// Expire idle and still-confirming consents past the TTL. A consent
    /// already sending keeps its slot until its outcome settles.
    fn prune(&self, entries: &mut Vec<Entry>) {
        let ttl = self.ttl;
        entries.retain(|e| e.phase == Phase::Sending || e.created.elapsed() < ttl);
    }

    fn epoch(&self) -> Arc<()> {
        self.pending.lock().epoch.clone()
    }

    /// Fetch and freeze the authoritative preview for `package_id` in `room`.
    #[cfg(test)]
    fn preview(&self, room: &str, package_id: &str) -> Result<ConsentPreview, String> {
        self.preview_at(room, package_id, &self.epoch())
    }

    fn preview_at(
        &self,
        room: &str,
        package_id: &str,
        epoch: &Arc<()>,
    ) -> Result<ConsentPreview, String> {
        self.endpoint()?;
        if !valid_key(room) || !valid_key(package_id) {
            return Err("invalid_request".into());
        }
        self.freeze(room, package_id, None, epoch)
    }

    /// Fetch and freeze one preview. With `expected_owner`, a preview naming
    /// any other owner is refused and nothing is frozen.
    fn freeze(
        &self,
        room: &str,
        package_id: &str,
        expected_owner: Option<&str>,
        epoch: &Arc<()>,
    ) -> Result<ConsentPreview, String> {
        let intent = Intent::Preview {
            room: room.to_string(),
            package_id: package_id.to_string(),
        };
        let reply = match self.transport.send(&intent.request()) {
            Ok(reply) => reply,
            Err(SendError::NotSent(code) | SendError::BadReply(code)) => return Err(code.into()),
            Err(SendError::Unknown) => return Err("daemon_unreachable".into()),
        };
        if reply.status != 200 {
            return Err(reply
                .body
                .get("error")
                .and_then(Value::as_str)
                .filter(|c| c.len() <= 64)
                .unwrap_or("preview_refused")
                .to_string());
        }
        let frozen = freeze_preview(room, package_id, &reply.body, fresh_id())?;
        if expected_owner.is_some_and(|owner| owner != frozen.owner_member_id) {
            return Err(BOOTSTRAP_OWNER_CONFLICT.into());
        }
        let id = fresh_id();
        let mut pending = self.pending.lock();
        if !Arc::ptr_eq(epoch, &pending.epoch) {
            return Err("consent_invalidated".into());
        }
        let entries = &mut pending.entries;
        self.prune(entries);
        // One live consent per room/agent: a fresh preview supersedes an idle one.
        entries.retain(|e| {
            e.phase != Phase::Ready
                || e.decision.target() != (frozen.room.as_str(), frozen.agent_member_id.as_str())
        });
        if entries.len() >= MAX_PENDING {
            match entries.iter().position(|e| e.phase == Phase::Ready) {
                Some(oldest) => {
                    entries.remove(oldest);
                }
                None => return Err("consent_busy".into()),
            }
        }
        let preview = frozen.projection(&id, self.ttl);
        entries.push(Entry {
            id,
            decision: Decision::Consent(Box::new(frozen)),
            created: Instant::now(),
            phase: Phase::Ready,
        });
        Ok(preview)
    }

    /// Claim a ready consent for native confirmation.
    pub(crate) fn begin_confirm(&self, consent_id: &str) -> Result<Frozen, String> {
        let mut pending = self.pending.lock();
        let entries = &mut pending.entries;
        self.prune(entries);
        let entry = entries
            .iter_mut()
            .find(|e| e.id == consent_id)
            .ok_or("consent_expired")?;
        if entry.phase != Phase::Ready {
            return Err("consent_in_progress".into());
        }
        let Decision::Consent(frozen) = &entry.decision else {
            return Err("consent_invalidated".into());
        };
        entry.phase = Phase::Confirming;
        Ok((**frozen).clone())
    }

    /// Settle the native confirmation. Only a still-claimed, confirmed,
    /// unexpired consent proceeds; a declined one is discarded. An
    /// invalidation that raced the dialog, or a dialog left open past the TTL,
    /// wins and nothing is sent.
    pub(crate) fn finish_confirm(
        &self,
        consent_id: &str,
        confirmed: bool,
    ) -> Result<Option<Frozen>, String> {
        let mut pending = self.pending.lock();
        if !self.settle_confirmation(&mut pending.entries, consent_id, confirmed)? {
            return Ok(None);
        }
        let entry = pending.entries.iter().find(|e| e.id == consent_id).unwrap();
        let Decision::Consent(frozen) = &entry.decision else {
            return Err("consent_invalidated".into());
        };
        Ok(Some((**frozen).clone()))
    }

    fn settle_confirmation(
        &self,
        entries: &mut Vec<Entry>,
        id: &str,
        confirmed: bool,
    ) -> Result<bool, String> {
        let Some(index) = entries
            .iter()
            .position(|e| e.id == id && e.phase == Phase::Confirming)
        else {
            return Err("consent_invalidated".into());
        };
        if entries[index].created.elapsed() >= self.ttl {
            entries.remove(index);
            return Err("consent_expired".into());
        }
        if !confirmed {
            entries.remove(index);
            return Ok(false);
        }
        entries[index].phase = Phase::Queued;
        Ok(true)
    }

    /// The dispatch boundary: atomically claim a still-live queued decision.
    /// Cancellation received before this claim wins; after it, I/O may finish.
    fn claim_dispatch(&self, id: &str) -> Result<Intent, &'static str> {
        let mut pending = self.pending.lock();
        let entries = &mut pending.entries;
        let index = entries
            .iter()
            .position(|e| e.id == id)
            .ok_or("consent_invalidated")?;
        if entries[index].phase != Phase::Queued {
            return Err("consent_in_progress");
        }
        if entries[index].created.elapsed() >= self.ttl {
            entries.remove(index);
            return Err("consent_expired");
        }
        entries[index].phase = Phase::Sending;
        Ok(entries[index].decision.intent())
    }

    /// Send the frozen decision. A definitive reply consumes the consent; a
    /// lost acknowledgement returns it to `Ready` with the same decision id so
    /// only an explicit, re-confirmed retry can replay it.
    pub(crate) fn send_decision(&self, consent_id: &str) -> ConsentOutcome {
        let intent = match self.claim_dispatch(consent_id) {
            Ok(intent) => intent,
            Err(code) => return refused(code),
        };
        if let Err(code) = self.endpoint() {
            self.release(consent_id, true);
            return refused(code);
        }
        match self.transport.send(&intent.request()) {
            Ok(reply) => {
                self.release(consent_id, false);
                outcome_from_reply(reply)
            }
            Err(SendError::NotSent(code)) => {
                self.release(consent_id, true);
                refused(code)
            }
            Err(SendError::BadReply(code)) => {
                self.release(consent_id, true);
                ConsentOutcome {
                    state: OutcomeState::Unknown,
                    status: None,
                    code: Some(code.into()),
                    binding: None,
                }
            }
            Err(SendError::Unknown) => {
                self.release(consent_id, true);
                ConsentOutcome {
                    state: OutcomeState::Unknown,
                    status: None,
                    code: None,
                    binding: None,
                }
            }
        }
    }

    fn release(&self, consent_id: &str, keep: bool) {
        let mut pending = self.pending.lock();
        let entries = &mut pending.entries;
        if let Some(index) = entries
            .iter()
            .position(|e| e.id == consent_id && e.phase == Phase::Sending)
        {
            if keep {
                entries[index].phase = Phase::Ready;
            } else {
                entries.remove(index);
            }
        }
    }

    /// The daemon owner's `participant_id` from `GET /v1/me`, read natively.
    fn daemon_owner(&self) -> Result<String, String> {
        let reply = match self.transport.send(&Intent::Me.request()) {
            Ok(reply) => reply,
            Err(SendError::NotSent(code) | SendError::BadReply(code)) => return Err(code.into()),
            // A lost reply to a read changed nothing.
            Err(SendError::Unknown) => return Err("daemon_unreachable".into()),
        };
        if reply.status != 200 {
            return Err("owner_unavailable".into());
        }
        bounded_id(reply.body.get("participant_id")).ok_or_else(|| "owner_unavailable".into())
    }

    /// Bootstrap `package_id` into the Local `room` as the daemon owner's
    /// agent, then fetch and freeze its consent preview so the page lands in
    /// the consent sheet. The owner comes only from `GET /v1/me`.
    ///
    /// The daemon records the owner role once and refuses a different owner
    /// (`room_owner_conflict`); that refusal still proceeds to the preview,
    /// which is frozen only when it names this daemon's owner. Nothing is
    /// retried automatically: a lost bootstrap acknowledgement is
    /// `outcome_unknown` and fetches no preview.
    #[cfg(test)]
    fn bootstrap(&self, room: &str, package_id: &str) -> Result<ConsentPreview, String> {
        self.bootstrap_at(room, package_id, &self.epoch())
    }

    fn bootstrap_at(
        &self,
        room: &str,
        package_id: &str,
        epoch: &Arc<()>,
    ) -> Result<ConsentPreview, String> {
        self.endpoint()?;
        if !valid_key(room) || !valid_key(package_id) {
            return Err("invalid_request".into());
        }
        let owner = self.daemon_owner()?;
        let intent = Intent::Bootstrap {
            room: room.to_string(),
            package_id: package_id.to_string(),
            owner: owner.clone(),
        };
        let reply = match self.transport.send(&intent.request()) {
            Ok(reply) => reply,
            Err(SendError::NotSent(code)) => return Err(code.into()),
            Err(SendError::Unknown | SendError::BadReply(_)) => {
                return Err("outcome_unknown".into())
            }
        };
        bootstrap_step(&reply)?;
        self.freeze(room, package_id, Some(&owner), epoch)
    }

    /// Drop one consent, or every consent when `consent_id` is `None`
    /// (navigation, room change). A request already sending keeps its late
    /// outcome but is not returned to `Ready`.
    pub(crate) fn cancel(&self, consent_id: Option<&str>) {
        let mut pending = self.pending.lock();
        let entries = &mut pending.entries;
        match consent_id {
            Some(id) => entries.retain(|e| e.id != id),
            None => {
                entries.clear();
                pending.epoch = Arc::new(());
            }
        }
    }

    /// Register revoke before showing its dialog. An explicit retry after an
    /// unknown outcome reuses the same decision id and requires confirmation.
    fn begin_revoke(&self, room: &str, agent: &str) -> Result<String, String> {
        self.endpoint()?;
        if !valid_key(room) || bounded_id(Some(&Value::String(agent.to_string()))).is_none() {
            return Err("invalid_request".into());
        }
        let mut pending = self.pending.lock();
        let entries = &mut pending.entries;
        self.prune(entries);
        if let Some(entry) = entries.iter_mut().find(|e| {
            matches!(&e.decision, Decision::Revoke { .. }) && e.decision.target() == (room, agent)
        }) {
            if entry.phase != Phase::Ready {
                return Err("consent_in_progress".into());
            }
            entry.phase = Phase::Confirming;
            let id = entry.id.clone();
            entries.retain(|e| e.id == id || e.decision.target() != (room, agent));
            return Ok(id);
        }
        // Invalidate older authorization for this target before the dialog.
        entries.retain(|e| e.decision.target() != (room, agent));
        if entries.len() >= MAX_PENDING {
            return Err("consent_busy".into());
        }
        let id = fresh_id();
        entries.push(Entry {
            id: id.clone(),
            decision: Decision::Revoke {
                room: room.into(),
                agent: agent.into(),
                decision_id: fresh_id(),
            },
            created: Instant::now(),
            phase: Phase::Confirming,
        });
        Ok(id)
    }

    fn finish_revoke(&self, id: &str, confirmed: bool) -> Result<bool, String> {
        self.settle_confirmation(&mut self.pending.lock().entries, id, confirmed)
    }

    #[cfg(test)]
    fn revoke(&self, room: &str, agent: &str) -> ConsentOutcome {
        let id = match self.begin_revoke(room, agent) {
            Ok(id) => id,
            Err(code) => return refused(&code),
        };
        self.finish_revoke(&id, true).unwrap();
        self.send_decision(&id)
    }
}

// ── Tauri glue ──────────────────────────────────────────────────────────

pub(crate) struct ConsentState(pub(crate) Arc<ConsentBroker>);

fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    require_main_window(window.label(), "consent_window_refused")
}

async fn native_confirm(
    app: &tauri::AppHandle,
    title: String,
    message: String,
    action: &str,
    warning: bool,
) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(message)
        .title(title)
        .kind(if warning {
            MessageDialogKind::Warning
        } else {
            MessageDialogKind::Info
        })
        .buttons(MessageDialogButtons::OkCancelCustom(
            action.to_string(),
            "Cancel".to_string(),
        ))
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    rx.await.unwrap_or(false)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| "consent_task_failed".to_string())
}

#[tauri::command]
pub(crate) async fn room_consent_preview(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConsentState>,
    room: String,
    package_id: String,
) -> Result<ConsentPreview, String> {
    main_window(&window)?;
    let broker = state.0.clone();
    let epoch = broker.epoch();
    blocking(move || broker.preview_at(room.trim(), package_id.trim(), &epoch)).await?
}

#[tauri::command]
pub(crate) async fn room_consent_authorize(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConsentState>,
    consent_id: String,
) -> Result<ConsentOutcome, String> {
    main_window(&window)?;
    let broker = state.0.clone();
    let frozen = broker.begin_confirm(&consent_id)?;
    let (title, message, action) = frozen.dialog_text();
    let confirmed = native_confirm(&app, title, message, action, false).await;
    let Some(_) = broker.finish_confirm(&consent_id, confirmed)? else {
        return Ok(ConsentOutcome::declined());
    };
    blocking(move || broker.send_decision(&consent_id)).await
}

/// Window-gated bootstrap (pure, so the gate is testable without a webview).
#[cfg(test)]
fn bootstrap_from(
    label: &str,
    broker: &ConsentBroker,
    room: &str,
    package_id: &str,
) -> Result<ConsentPreview, String> {
    require_main_window(label, "consent_window_refused")?;
    broker.bootstrap(room.trim(), package_id.trim())
}

/// Local-room agent bootstrap, then the same frozen preview
/// `room_consent_preview` returns. Main window only; the owner is read
/// natively from `GET /v1/me`.
#[tauri::command]
pub(crate) async fn room_agent_bootstrap(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConsentState>,
    room: String,
    package_id: String,
) -> Result<ConsentPreview, String> {
    main_window(&window)?;
    let broker = state.0.clone();
    let epoch = broker.epoch();
    blocking(move || broker.bootstrap_at(room.trim(), package_id.trim(), &epoch)).await?
}

#[tauri::command]
pub(crate) fn room_consent_cancel(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConsentState>,
    consent_id: Option<String>,
) -> Result<(), String> {
    main_window(&window)?;
    state.0.cancel(consent_id.as_deref());
    Ok(())
}

#[tauri::command]
pub(crate) async fn room_agent_revoke(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ConsentState>,
    room: String,
    agent_member_id: String,
    display_name: String,
) -> Result<ConsentOutcome, String> {
    main_window(&window)?;
    let room = room.trim().to_string();
    let agent = agent_member_id.trim().to_string();
    if !valid_key(&room) || bounded_id(Some(&Value::String(agent.clone()))).is_none() {
        return Err("invalid_request".into());
    }
    let broker = state.0.clone();
    let id = broker.begin_revoke(&room, &agent)?;
    let (title, message) = revoke_dialog_text(&room, &agent, &display_name);
    let confirmed = native_confirm(&app, title, message, "Revoke", true).await;
    if !broker.finish_revoke(&id, confirmed)? {
        return Ok(ConsentOutcome::declined());
    }
    blocking(move || broker.send_decision(&id)).await
}

// ── tests ───────────────────────────────────────────────────────────────

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::daemon_transport::test_support::*;
    use crate::daemon_transport::{
        loopback_endpoint, parse_reply, read_operator_key, CREDENTIAL_UNAVAILABLE, MAX_REPLY_BYTES,
    };
    use std::path::{Path, PathBuf};

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn preview_body(eligible: bool, binding: Option<Value>) -> String {
        json!({
            "ok": true,
            "package_id": "scout",
            "display_name": "Scout",
            "definition_digest": DIGEST,
            "definition_revision": null,
            "requested_capabilities": ["read"],
            "grantable_capabilities": [],
            "agent_member_id": "agent-scout-1",
            "owner_member_id": "member-owner",
            "owner_eligible": eligible,
            "binding": binding,
        })
        .to_string()
    }

    fn broker(port: u16, key: &Path) -> ConsentBroker {
        ConsentBroker::new(DaemonTransport::new(
            &format!("http://127.0.0.1:{port}"),
            key.to_path_buf(),
        ))
    }

    #[test]
    fn only_numeric_loopback_endpoints_are_trusted() {
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:4780"),
            Some("127.0.0.1:4780".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://[::1]:9/"),
            Some("[::1]:9".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://127.0.0.1"),
            Some("127.0.0.1:4780".parse().unwrap())
        );
        for url in [
            "http://localhost:4780",
            "https://127.0.0.1:4780",
            "http://10.0.0.5:4780",
            "http://user@127.0.0.1:4780",
            "http://127.0.0.1:4780/v1",
            "http://127.0.0.1:0",
            "127.0.0.1:4780",
            "http://evil.example#@127.0.0.1",
        ] {
            assert_eq!(loopback_endpoint(url), None, "{url}");
        }
        for url in ["http://192.0.2.1:4780", "http://localhost:4780"] {
            let broker =
                ConsentBroker::new(DaemonTransport::new(url, PathBuf::from("/nonexistent")));
            assert_eq!(
                broker.preview("room", "scout").unwrap_err(),
                "consent_unavailable"
            );
            assert_eq!(
                broker.revoke("room", "agent").code.as_deref(),
                Some("consent_unavailable")
            );
        }
        let broker = ConsentBroker::new(DaemonTransport::new(
            "http://192.0.2.1:4780",
            PathBuf::from("/nonexistent"),
        ));
        assert_eq!(
            broker.preview("room", "scout").unwrap_err(),
            "consent_unavailable"
        );
    }

    #[test]
    fn credential_custody_refuses_unsafe_files_without_leaking_the_key() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_operator_key(&dir.path().join("absent.key")).unwrap_err(),
            CREDENTIAL_UNAVAILABLE
        );
        let loose = key_file(dir.path(), 0o640);
        assert_eq!(
            read_operator_key(&loose).unwrap_err(),
            "operator_credential_not_private"
        );
        let private = key_file(dir.path(), 0o600);
        let link = dir.path().join("link.key");
        credential_symlink(&private, &link);
        assert_eq!(
            read_operator_key(&link).unwrap_err(),
            "operator_credential_not_regular"
        );
        assert_eq!(
            read_operator_key(dir.path()).unwrap_err(),
            "operator_credential_not_regular"
        );
        let injected = dir.path().join("inject.key");
        private_bytes(&injected, b"abcdefghijklmnopqrstuvwx\r\nOrigin: x");
        assert_eq!(
            read_operator_key(&injected).unwrap_err(),
            CREDENTIAL_UNAVAILABLE
        );
        assert_eq!(read_operator_key(&private).unwrap(), KEY);
    }

    #[test]
    fn preview_freezes_native_state_and_never_sends_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", &preview_body(true, None)))]);
        let broker = broker(port, &key);
        let preview = broker.preview("team.room", "scout").unwrap();
        let seen = daemon.join().unwrap();
        assert!(seen[0]
            .starts_with("GET /v1/rooms/persistent/team.room/agents/preview/scout HTTP/1.1\r\n"));
        assert!(!seen[0].contains(KEY), "preview is unprivileged");
        assert_eq!(preview.definition_digest, DIGEST);
        assert_eq!(preview.mode, ConsentMode::Authorize);
        assert_eq!(preview.agent_member_id, "agent-scout-1");
        assert!(!serde_json::to_string(&preview).unwrap().contains(KEY));
        assert_eq!(broker.pending.lock().entries.len(), 1);
    }

    #[test]
    fn preview_refuses_ineligible_malformed_redirected_and_oversized_replies() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let bad_digest = preview_body(true, None).replace(DIGEST, "sha256:XYZ");
        let other_package = preview_body(true, None).replace("\"scout\"", "\"other\"");
        let huge = format!(
            "{{\"ok\":true,\"pad\":\"{}\"}}",
            "x".repeat(MAX_REPLY_BYTES)
        );
        let cases = vec![
            (http("200 OK", &preview_body(false, None)), "owner_not_eligible"),
            (http("200 OK", &bad_digest), "invalid_preview"),
            (http("200 OK", &other_package), "invalid_preview"),
            (
                "HTTP/1.1 302 Found\r\nlocation: http://evil.example/\r\ncontent-length: 2\r\n\r\n{}".into(),
                "redirect_refused",
            ),
            (http("200 OK", "not json"), "invalid_reply"),
            (http("200 OK", "[1,2]"), "invalid_reply"),
            (http("200 OK", &huge), "invalid_reply"),
            (
                http("404 Not Found", r#"{"ok":false,"error":"room_not_found"}"#),
                "room_not_found",
            ),
        ];
        for (reply, code) in cases {
            let (port, daemon) = fake_daemon(vec![Some(reply)]);
            let broker = broker(port, &key);
            assert_eq!(broker.preview("room", "scout").unwrap_err(), code);
            daemon.join().unwrap();
            assert!(broker.pending.lock().entries.is_empty(), "{code}");
        }
        let broker = broker(9, &key);
        for (room, package) in [
            ("a/b", "scout"),
            ("room", "../x"),
            ("", "scout"),
            ("room", ".."),
        ] {
            assert_eq!(
                broker.preview(room, package).unwrap_err(),
                "invalid_request"
            );
        }
    }

    #[test]
    fn confirmed_authorize_sends_the_frozen_digest_once_with_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let applied = json!({"ok": true, "created": true, "binding": {"agent_member_id": "agent-scout-1", "status": "active", "generation": "1"}});
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, None))),
            Some(http("201 Created", &applied.to_string())),
        ]);
        let broker = broker(port, &key);
        let preview = broker.preview("room", "scout").unwrap();
        let frozen = broker.begin_confirm(&preview.consent_id).unwrap();
        assert_eq!(
            broker.begin_confirm(&preview.consent_id).unwrap_err(),
            "consent_in_progress"
        );
        let frozen2 = broker
            .finish_confirm(&preview.consent_id, true)
            .unwrap()
            .unwrap();
        assert_eq!(frozen, frozen2);
        let outcome = broker.send_decision(&preview.consent_id);
        let seen = daemon.join().unwrap();
        assert_eq!(outcome.state, OutcomeState::Applied);
        assert_eq!(outcome.binding.unwrap()["status"], "active");
        let request = &seen[1];
        assert!(request.starts_with("POST /v1/rooms/persistent/room/agents HTTP/1.1\r\n"));
        assert!(request.contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("\r\norigin:") && !lower.contains("\r\ncookie:"));
        let body = json_body(request);
        assert_eq!(body["expected_definition_digest"], DIGEST);
        assert_eq!(body["agent_member_id"], "agent-scout-1");
        assert_eq!(body["owner_member_id"], "member-owner");
        assert_eq!(body["agent_package_id"], "scout");
        assert_eq!(body["room_capability_grants"], json!([]));
        assert!(uuid::Uuid::parse_str(body["decision_id"].as_str().unwrap()).is_ok());
        // Consumed: a second authorize of the same consent cannot be sent.
        assert_eq!(
            broker.begin_confirm(&preview.consent_id).unwrap_err(),
            "consent_expired"
        );
    }

    #[test]
    fn declined_or_invalidated_confirmation_sends_zero_privileged_requests() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, None))),
            Some(http("200 OK", &preview_body(true, None))),
            Some(http("200 OK", &preview_body(true, None))),
        ]);
        let broker = broker(port, &key);
        let a = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&a.consent_id).unwrap();
        assert_eq!(broker.finish_confirm(&a.consent_id, false).unwrap(), None);
        let b = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&b.consent_id).unwrap();
        broker.cancel(None); // navigation while the dialog is open
        assert_eq!(
            broker.finish_confirm(&b.consent_id, true).unwrap_err(),
            "consent_invalidated"
        );
        let c = broker.preview("room", "scout").unwrap();
        broker.cancel(Some(&c.consent_id));
        assert_eq!(
            broker.begin_confirm(&c.consent_id).unwrap_err(),
            "consent_expired"
        );
        // Only the three unprivileged previews ever reached the daemon.
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(seen
            .iter()
            .all(|r| r.starts_with("GET ") && !r.contains(KEY)));
    }

    #[test]
    fn stale_digest_is_refused_without_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, None))),
            Some(http(
                "409 Conflict",
                r#"{"ok":false,"error":"definition_digest_mismatch"}"#,
            )),
        ]);
        let broker = broker(port, &key);
        let preview = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&preview.consent_id).unwrap();
        let _frozen = broker
            .finish_confirm(&preview.consent_id, true)
            .unwrap()
            .unwrap();
        let outcome = broker.send_decision(&preview.consent_id);
        assert_eq!(outcome.state, OutcomeState::Refused);
        assert_eq!(outcome.status, Some(409));
        assert_eq!(outcome.code.as_deref(), Some("definition_digest_mismatch"));
        assert_eq!(
            daemon.join().unwrap().len(),
            2,
            "exactly one decision request"
        );
        assert!(
            broker.pending.lock().entries.is_empty(),
            "a refused consent is consumed"
        );
    }

    #[test]
    fn lost_acknowledgement_is_unknown_and_replays_only_the_same_decision() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let binding = json!({"agent_member_id": "agent-scout-1", "status": "suspended", "activation_policy": "mention", "context_policy": "invocation_only", "memory_scope": "none"});
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, Some(binding)))),
            None,
        ]);
        let broker = broker(port, &key);
        let preview = broker.preview("room", "scout").unwrap();
        assert_eq!(preview.mode, ConsentMode::Reauthorize);
        broker.begin_confirm(&preview.consent_id).unwrap();
        let frozen = broker
            .finish_confirm(&preview.consent_id, true)
            .unwrap()
            .unwrap();
        let outcome = broker.send_decision(&preview.consent_id);
        let seen = daemon.join().unwrap();
        assert_eq!(outcome.state, OutcomeState::Unknown);
        assert_eq!(seen.len(), 2, "no automatic retry");
        assert!(seen[1].starts_with(
            "POST /v1/rooms/persistent/room/agents/agent-scout-1/reauthorize HTTP/1.1\r\n"
        ));
        let body = json_body(&seen[1]);
        assert_eq!(body["expected_definition_digest"], DIGEST);
        assert_eq!(body["activation_policy"], "mention");
        assert!(body.get("agent_package_id").is_none());
        // An explicit retry re-confirms and replays the identical decision id.
        let retry = broker.begin_confirm(&preview.consent_id).unwrap();
        assert_eq!(retry.decision_id, frozen.decision_id);
    }

    #[test]
    fn consents_are_bounded_expire_and_use_fresh_decision_ids() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let replies = (0..MAX_PENDING + 1)
            .map(|i| {
                Some(http(
                    "200 OK",
                    &preview_body(true, None).replace("agent-scout-1", &format!("agent-{i}")),
                ))
            })
            .collect();
        let (port, daemon) = fake_daemon(replies);
        let mut broker = broker(port, &key);
        let ids: Vec<_> = (0..=MAX_PENDING)
            .map(|_| broker.preview("room", "scout").unwrap().consent_id)
            .collect();
        daemon.join().unwrap();
        let pending = broker.pending.lock();
        let entries = &pending.entries;
        assert_eq!(entries.len(), MAX_PENDING);
        assert!(entries.iter().all(|e| e.id != ids[0]), "oldest evicted");
        let decisions: std::collections::HashSet<_> = entries
            .iter()
            .map(|e| match &e.decision {
                Decision::Consent(frozen) => frozen.decision_id.clone(),
                _ => panic!("not consent"),
            })
            .collect();
        assert_eq!(decisions.len(), MAX_PENDING);
        drop(pending);
        broker.ttl = Duration::ZERO;
        assert_eq!(
            broker.begin_confirm(&ids[1]).unwrap_err(),
            "consent_expired"
        );
    }

    #[test]
    fn revoke_posts_a_fresh_decision_and_unreachable_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http(
            "200 OK",
            r#"{"ok":true,"applied":true,"binding":{"status":"revoked"}}"#,
        ))]);
        let outcome = broker(port, &key).revoke("room", "agent scout");
        let seen = daemon.join().unwrap();
        assert_eq!(outcome.state, OutcomeState::Applied);
        assert!(seen[0].starts_with(
            "POST /v1/rooms/persistent/room/agents/agent%20scout/revoke HTTP/1.1\r\n"
        ));
        assert!(seen[0].contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        assert!(
            uuid::Uuid::parse_str(json_body(&seen[0])["decision_id"].as_str().unwrap()).is_ok()
        );

        // A broken credential refuses before any connection.
        let (port, listener) = silent_daemon();
        let loose = key_file(dir.path(), 0o644);
        let outcome = broker(port, &loose).revoke("room", "agent");
        assert_eq!(
            outcome.code.as_deref(),
            Some("operator_credential_not_private")
        );
        assert_never_contacted(&listener);
    }

    #[test]
    fn chunked_replies_decode_and_bad_framing_is_refused() {
        let reply = parse_reply(
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n",
        )
        .ok()
        .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["a"], 1);
        assert!(parse_reply(
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\ncontent-length: 2\r\n\r\n{}"
        )
        .is_err());
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\ntransfer-encoding: gzip\r\n\r\n{}").is_err());
        assert!(parse_reply(b"SSH-2.0 200\r\n\r\n{}").is_err());
    }

    #[test]
    fn a_dialog_left_open_past_the_ttl_sends_nothing_and_confirming_entries_expire() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, None))),
            Some(http("200 OK", &preview_body(true, None))),
        ]);
        let mut broker = broker(port, &key);
        let a = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&a.consent_id).unwrap();
        // The operator confirms only after the frozen preview expired.
        broker.ttl = Duration::ZERO;
        assert_eq!(
            broker.finish_confirm(&a.consent_id, true).unwrap_err(),
            "consent_expired"
        );
        assert!(
            broker.pending.lock().entries.is_empty(),
            "expired consent removed"
        );

        // A consent stuck in Confirming past the TTL is pruned too.
        broker.ttl = CONSENT_TTL;
        let b = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&b.consent_id).unwrap();
        broker.ttl = Duration::ZERO;
        broker.prune(&mut broker.pending.lock().entries);
        assert!(
            broker.pending.lock().entries.is_empty(),
            "confirming consent pruned"
        );
        assert_eq!(
            broker.finish_confirm(&b.consent_id, true).unwrap_err(),
            "consent_invalidated"
        );
        // Only the two unprivileged previews ever reached the daemon.
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen
            .iter()
            .all(|r| r.starts_with("GET ") && !r.contains(KEY)));
    }

    #[test]
    fn revoke_dialog_names_the_exact_agent_and_sanitizes_the_page_label() {
        let (title, message) = revoke_dialog_text(
            "team.room",
            "agent-scout-1",
            "Scout\nAgent id: agent-other\r\u{2028}\u{202E}\u{200B}\t",
        );
        assert_eq!(title, "Revoke ScoutAgent id: agent-other");
        assert_eq!(message, "Room: team.room\nAgent id: agent-scout-1");
        let (title, _) = revoke_dialog_text("room", "agent-1", " \n\u{7}");
        assert_eq!(title, "Revoke agent");
        let (title, _) = revoke_dialog_text("room", "agent-1", &"x".repeat(1000));
        assert_eq!(
            title.chars().count(),
            "Revoke ".len() + MAX_DIALOG_LABEL_CHARS
        );
        assert!(title.ends_with('\u{2026}'));

        // An agent id that would forge dialog lines is refused unsent.
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, listener) = silent_daemon();
        for agent in ["agent\nRoom: other", "agent\u{202E}x", "agent\u{2028}"] {
            assert_eq!(
                broker(port, &key).revoke("room", agent).code.as_deref(),
                Some("invalid_request"),
                "{agent:?}"
            );
        }
        assert_never_contacted(&listener);
    }

    #[test]
    fn authorize_dialog_shows_frozen_package_agent_owner_and_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let body = preview_body(true, None)
            .replace("\"Scout\"", "\"Scout\\nOwner: someone-else\"")
            .replace("[\"read\"]", "[\"read\",\"write\"]");
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", &body))]);
        let broker = broker(port, &key);
        let preview = broker.preview("team.room", "scout").unwrap();
        daemon.join().unwrap();
        let frozen = broker.begin_confirm(&preview.consent_id).unwrap();
        let (title, message, action) = frozen.dialog_text();
        assert_eq!(action, "Authorize");
        assert_eq!(title, "ScoutOwner: someone-else");
        assert_eq!(
            message,
            format!(
                "Room: team.room\nPackage: scout\nAgent id: agent-scout-1\nOwner: member-owner\nCapabilities: read, write\nDigest: {DIGEST}"
            )
        );

        // Ids or capabilities that could forge dialog lines are not frozen.
        for (body, code) in [
            (
                preview_body(true, None).replace("agent-scout-1", "agent\\nOwner: x"),
                "owner_not_eligible",
            ),
            (
                preview_body(true, None).replace("member-owner", "member\\u202Eowner"),
                "owner_not_eligible",
            ),
            (
                preview_body(true, None).replace("[\"read\"]", "[\"read\\nwrite\"]"),
                "invalid_preview",
            ),
        ] {
            let (port, daemon) = fake_daemon(vec![Some(http("200 OK", &body))]);
            assert_eq!(
                super::tests::broker(port, &key)
                    .preview("room", "scout")
                    .unwrap_err(),
                code
            );
            daemon.join().unwrap();
        }
        let many: Vec<String> = (0..128).map(|i| format!("capability-{i:03}")).collect();
        let line = capability_line(&many);
        assert!(line.chars().count() < MAX_DIALOG_CAPS_CHARS + 32);
        assert!(line.ends_with("more)"));
        assert_eq!(capability_line(&[]), "none");
    }

    #[test]
    fn hostile_chunk_sizes_are_bad_replies_and_free_the_slot() {
        for raw in [
            &b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nffffffffffffffff\r\n{}\r\n0\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nfffffffffffffffe\r\n{}\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n2\r\n{}XX0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n40001\r\n{}\r\n0\r\n\r\n",
        ] {
            assert_eq!(
                parse_reply(raw).err(),
                Some(SendError::BadReply("invalid_reply"))
            );
        }

        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let hostile = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nffffffffffffffff\r\n{}\r\n0\r\n\r\n";
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &preview_body(true, None))),
            Some(hostile.to_string()),
        ]);
        let broker = broker(port, &key);
        let preview = broker.preview("room", "scout").unwrap();
        broker.begin_confirm(&preview.consent_id).unwrap();
        let frozen = broker
            .finish_confirm(&preview.consent_id, true)
            .unwrap()
            .unwrap();
        let outcome = broker.send_decision(&preview.consent_id);
        assert_eq!(daemon.join().unwrap().len(), 2);
        assert_eq!(outcome.state, OutcomeState::Unknown);
        assert_eq!(outcome.code.as_deref(), Some("invalid_reply"));
        // The slot is not stuck in Sending: an explicit retry can re-confirm
        // the same decision.
        let retry = broker.begin_confirm(&preview.consent_id).unwrap();
        assert_eq!(retry.decision_id, frozen.decision_id);
    }

    // Deterministic custody interleavings: channels stop real loopback I/O at
    // a known request, without sleeping to guess which side won the race.
    fn paused_daemon(
        replies: Vec<Option<String>>,
        pause_at: usize,
    ) -> (
        u16,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<Vec<String>>,
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (resume, resume_rx) = std::sync::mpsc::channel();
        let daemon = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (i, reply) in replies.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                loop {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    raw.push(byte[0]);
                    if let Some(end) = raw.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..end]);
                        let len = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .map(|s| s.parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if raw.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                seen.push(String::from_utf8(raw).unwrap());
                if i == pause_at {
                    arrived_tx.send(()).unwrap();
                    resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                if let Some(reply) = reply {
                    socket.write_all(reply.as_bytes()).unwrap();
                }
            }
            seen
        });
        (port, arrived, resume, daemon)
    }

    /// Seed the same parsed frozen decision as a preview; these tests exercise
    /// the subsequent confirmation/dispatch boundary against a silent socket.
    fn seed_consent(broker: &ConsentBroker) -> String {
        let frozen = freeze_preview(
            "room",
            "scout",
            &serde_json::from_str(&preview_body(true, None)).unwrap(),
            fresh_id(),
        )
        .unwrap();
        let id = fresh_id();
        broker.pending.lock().entries.push(Entry {
            id: id.clone(),
            decision: Decision::Consent(Box::new(frozen)),
            created: Instant::now(),
            phase: Phase::Ready,
        });
        id
    }

    fn queue_decision(broker: &ConsentBroker, revoke: bool) -> String {
        if revoke {
            let id = broker.begin_revoke("room", "agent-scout-1").unwrap();
            assert!(broker.finish_revoke(&id, true).unwrap());
            id
        } else {
            let id = seed_consent(broker);
            broker.begin_confirm(&id).unwrap();
            broker.finish_confirm(&id, true).unwrap().unwrap();
            id
        }
    }

    #[test]
    fn queued_decisions_cancel_or_expire_before_dispatch_without_contact() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        for revoke in [false, true] {
            for action in ["cancel_one", "cancel_all", "expire"] {
                let (port, listener) = silent_daemon();
                let mut broker = broker(port, &key);
                let id = queue_decision(&broker, revoke);
                match action {
                    "cancel_one" => broker.cancel(Some(&id)),
                    "cancel_all" => broker.cancel(None),
                    _ => broker.ttl = Duration::ZERO,
                }
                let outcome = broker.send_decision(&id);
                assert_eq!(outcome.state, OutcomeState::Refused);
                assert_eq!(
                    outcome.code.as_deref(),
                    Some(if action == "expire" {
                        "consent_expired"
                    } else {
                        "consent_invalidated"
                    })
                );
                assert!(broker.pending.lock().entries.is_empty());
                assert_never_contacted(&listener);
            }
        }
    }

    #[test]
    fn revoke_dialog_is_cancellable_and_invalidates_queued_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, listener) = silent_daemon();
        let broker = broker(port, &key);
        let authorization = queue_decision(&broker, false);
        let revoke = broker.begin_revoke("room", "agent-scout-1").unwrap();
        assert_eq!(
            broker.send_decision(&authorization).code.as_deref(),
            Some("consent_invalidated")
        );
        broker.cancel(None);
        assert_eq!(
            broker.finish_revoke(&revoke, true).unwrap_err(),
            "consent_invalidated"
        );
        assert_eq!(
            broker.send_decision(&revoke).code.as_deref(),
            Some("consent_invalidated")
        );
        let declined = broker.begin_revoke("room", "agent-scout-1").unwrap();
        assert!(!broker.finish_revoke(&declined, false).unwrap());
        assert!(broker.pending.lock().entries.is_empty());
        assert_never_contacted(&listener);
    }

    #[test]
    fn cancellation_fences_preview_and_bootstrap_insertion_across_network_waits() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        for bootstrap in [false, true] {
            let mut replies = Vec::new();
            if bootstrap {
                replies.push(Some(http("200 OK", &me_body("member-owner"))));
                replies.push(Some(http("201 Created", &bootstrapped(true))));
            }
            replies.push(Some(http("200 OK", &preview_body(true, None))));
            // For bootstrap, pause the initial owner read: the epoch must span
            // the entire operation, not just its final preview fetch.
            let (port, arrived, resume, daemon) = paused_daemon(replies, 0);
            let broker = Arc::new(broker(port, &key));
            let worker_broker = broker.clone();
            let epoch = broker.epoch();
            let worker = std::thread::spawn(move || {
                if bootstrap {
                    worker_broker.bootstrap_at("room", "scout", &epoch)
                } else {
                    worker_broker.preview_at("room", "scout", &epoch)
                }
            });
            arrived.recv_timeout(Duration::from_secs(5)).unwrap();
            broker.cancel(None);
            resume.send(()).unwrap();
            assert_eq!(worker.join().unwrap().unwrap_err(), "consent_invalidated");
            assert!(broker.pending.lock().entries.is_empty());
            let seen = daemon.join().unwrap();
            assert_eq!(seen.len(), if bootstrap { 3 } else { 1 });
            // Cancellation invalidates consent insertion, not a bootstrap
            // side effect: bootstrap is not an authorization/binding.
            assert!(seen.iter().all(|r| !r.contains("/authorize ")));
        }
    }

    #[test]
    fn cancellation_before_a_queued_preview_worker_still_fences_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", &preview_body(true, None)))]);
        let broker = broker(port, &key);
        let epoch = broker.epoch(); // captured by command before spawn_blocking
        broker.cancel(None);
        assert_eq!(
            broker.preview_at("room", "scout", &epoch).unwrap_err(),
            "consent_invalidated"
        );
        assert!(broker.pending.lock().entries.is_empty());
        assert_eq!(daemon.join().unwrap().len(), 1);
    }

    #[test]
    fn cancelled_inflight_completions_never_restore_retry_state_or_hold_lock() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        for revoke in [false, true] {
            let (port, arrived, resume, daemon) = paused_daemon(vec![None], 0);
            let broker = Arc::new(broker(port, &key));
            let id = queue_decision(&broker, revoke);
            let worker_broker = broker.clone();
            let worker_id = id.clone();
            let worker = std::thread::spawn(move || worker_broker.send_decision(&worker_id));
            arrived.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                broker.send_decision(&id).code.as_deref(),
                Some("consent_in_progress")
            );
            // This must complete while the socket is still blocked.
            broker.cancel(None);
            let fresh = seed_consent(&broker);
            resume.send(()).unwrap();
            assert_eq!(worker.join().unwrap().state, OutcomeState::Unknown);
            assert_eq!(daemon.join().unwrap().len(), 1, "no duplicate dispatch");
            let pending = broker.pending.lock();
            assert_eq!(pending.entries.len(), 1);
            assert_eq!(
                pending.entries[0].id, fresh,
                "late completion leaves newer state alone"
            );
            assert_eq!(pending.entries[0].phase, Phase::Ready);
        }
    }

    #[test]
    fn revoke_unknown_requires_reconfirmation_and_replays_same_decision() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            None,
            Some(http(
                "200 OK",
                r#"{"ok":true,"applied":true,"binding":{"status":"revoked"}}"#,
            )),
        ]);
        let broker = broker(port, &key);
        let id = broker.begin_revoke("room", "agent-scout-1").unwrap();
        assert!(broker.finish_revoke(&id, true).unwrap());
        assert_eq!(broker.send_decision(&id).state, OutcomeState::Unknown);
        assert_eq!(
            broker.send_decision(&id).code.as_deref(),
            Some("consent_in_progress")
        );
        let retry = broker.begin_revoke("room", "agent-scout-1").unwrap();
        assert_eq!(retry, id);
        assert!(broker.finish_revoke(&retry, true).unwrap());
        assert_eq!(broker.send_decision(&retry).state, OutcomeState::Applied);
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(json_body(&seen[0]), json_body(&seen[1]));
        assert!(broker.pending.lock().entries.is_empty());
    }

    #[test]
    fn tracked_revokes_share_the_consent_bound_and_expire_in_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, listener) = silent_daemon();
        let mut broker = broker(port, &key);
        let id = seed_consent(&broker);
        broker.begin_confirm(&id).unwrap();
        let ids: Vec<_> = (1..MAX_PENDING)
            .map(|i| broker.begin_revoke("room", &format!("agent-{i}")).unwrap())
            .collect();
        assert_eq!(
            broker.begin_revoke("room", "overflow").unwrap_err(),
            "consent_busy"
        );
        broker.ttl = Duration::ZERO;
        assert_eq!(
            broker.finish_revoke(&ids[0], true).unwrap_err(),
            "consent_expired"
        );
        assert_never_contacted(&listener);
    }

    // ── Local bootstrap (PR G) ──────────────────────────────────────────

    fn me_body(participant: &str) -> String {
        json!({ "participant_id": participant, "display_name": "Owner" }).to_string()
    }

    fn bootstrapped(created: bool) -> String {
        json!({
            "ok": true,
            "created": created,
            "room_id": "team.room",
            "owner_member_id": "member-owner",
            "agent_member_id": "scout",
            "agent_package_id": "scout",
            "owner_eligible": true,
        })
        .to_string()
    }

    fn refusal(status: &str, code: &str) -> String {
        http(status, &json!({ "ok": false, "error": code }).to_string())
    }

    #[test]
    fn bootstrap_intent_is_a_fixed_operator_post_with_the_strict_body() {
        let intent = Intent::Bootstrap {
            room: "team room".into(),
            package_id: "scout".into(),
            owner: "member-owner".into(),
        };
        let request = intent.request();
        assert!(request.is_operator());
        assert_eq!(
            request,
            DaemonRequest::operator(
                Method::Post,
                "/v1/rooms/persistent/team%20room/agents/bootstrap".into(),
                Some(
                    json!({ "owner_member_id": "member-owner", "agent_package_id": "scout" })
                        .to_string()
                ),
            )
        );
        let me = Intent::Me.request();
        assert!(!me.is_operator(), "the owner read is unprivileged");
        assert_eq!(
            me,
            DaemonRequest::public(Method::Get, "/v1/me".into(), None)
        );
    }

    #[test]
    fn bootstrap_takes_the_owner_from_v1_me_and_lands_in_the_frozen_preview() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &me_body("member-owner"))),
            Some(http("201 Created", &bootstrapped(true))),
            Some(http("200 OK", &preview_body(true, None))),
        ]);
        let broker = broker(port, &key);
        let preview = broker.bootstrap("team.room", "scout").unwrap();
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(seen[0].starts_with("GET /v1/me HTTP/1.1\r\n"));
        assert!(!seen[0].contains(KEY), "the owner read is unprivileged");
        assert!(seen[1]
            .starts_with("POST /v1/rooms/persistent/team.room/agents/bootstrap HTTP/1.1\r\n"));
        assert!(seen[1].contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        let lower = seen[1].to_ascii_lowercase();
        assert!(!lower.contains("\r\norigin:") && !lower.contains("\r\ncookie:"));
        assert_eq!(
            json_body(&seen[1]),
            json!({ "owner_member_id": "member-owner", "agent_package_id": "scout" })
        );
        assert!(seen[2]
            .starts_with("GET /v1/rooms/persistent/team.room/agents/preview/scout HTTP/1.1\r\n"));
        assert!(!seen[2].contains(KEY));
        // The same projection the preview command returns, now authorizable.
        assert_eq!(preview.owner_member_id, "member-owner");
        assert_eq!(preview.mode, ConsentMode::Authorize);
        assert_eq!(preview.definition_digest, DIGEST);
        assert!(!serde_json::to_string(&preview).unwrap().contains(KEY));
        assert!(broker.begin_confirm(&preview.consent_id).is_ok());
    }

    #[test]
    fn bootstrap_owner_is_only_ever_the_v1_me_participant() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        // Whatever /v1/me names is the owner sent; the page has no owner input.
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &me_body("member-me"))),
            Some(refusal("403 Forbidden", "room_owner_required")),
        ]);
        assert_eq!(
            broker(port, &key).bootstrap("room", "scout").unwrap_err(),
            "room_owner_required"
        );
        let seen = daemon.join().unwrap();
        assert_eq!(json_body(&seen[1])["owner_member_id"], "member-me");

        // An unusable /v1/me never reaches the privileged bootstrap.
        for reply in [
            http("200 OK", &me_body("member\nOwner: x")),
            http("200 OK", &me_body("")),
            http("200 OK", &me_body(" padded")),
            http("200 OK", r#"{"display_name":"Owner"}"#),
            http("200 OK", r#"{"participant_id":7}"#),
            refusal("500 Internal Server Error", "room_store_error"),
        ] {
            let (port, daemon) = fake_daemon(vec![Some(reply)]);
            let broker = broker(port, &key);
            assert_eq!(
                broker.bootstrap("room", "scout").unwrap_err(),
                "owner_unavailable"
            );
            assert_eq!(daemon.join().unwrap().len(), 1, "no bootstrap sent");
            assert!(broker.pending.lock().entries.is_empty());
        }
    }

    #[test]
    fn bootstrap_refusals_map_to_fixed_codes_and_fetch_no_preview() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let cases = [
            (
                refusal("403 Forbidden", "room_owner_required"),
                "room_owner_required",
            ),
            (
                refusal("409 Conflict", "local_room_required"),
                "local_room_required",
            ),
            (
                refusal("409 Conflict", "bootstrap_target_conflict"),
                "bootstrap_target_conflict",
            ),
            (refusal("404 Not Found", "room_not_found"), "room_not_found"),
            (
                refusal("404 Not Found", "agent_package_not_found"),
                "agent_package_not_found",
            ),
            (
                refusal("400 Bad Request", "invalid_agent_package_id"),
                "agent_package_not_found",
            ),
            (
                refusal("403 Forbidden", "operator_required"),
                "bootstrap_refused",
            ),
            (
                refusal("409 Conflict", "Arbitrary\ntext"),
                "bootstrap_refused",
            ),
            (http("200 OK", r#"{"ok":false}"#), "bootstrap_refused"),
        ];
        for (reply, code) in cases {
            let (port, daemon) = fake_daemon(vec![
                Some(http("200 OK", &me_body("member-owner"))),
                Some(reply),
            ]);
            let broker = broker(port, &key);
            assert_eq!(broker.bootstrap("room", "scout").unwrap_err(), code);
            assert_eq!(daemon.join().unwrap().len(), 2, "{code}: no preview");
            assert!(broker.pending.lock().entries.is_empty(), "{code}");
        }
    }

    #[test]
    fn an_existing_bootstrap_proceeds_only_when_the_preview_names_this_owner() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        // Already bootstrapped (200) and an owner conflict whose preview names
        // the /v1/me owner both land in the consent sheet.
        for reply in [
            http("200 OK", &bootstrapped(false)),
            refusal("409 Conflict", "room_owner_conflict"),
        ] {
            let (port, daemon) = fake_daemon(vec![
                Some(http("200 OK", &me_body("member-owner"))),
                Some(reply),
                Some(http("200 OK", &preview_body(true, None))),
            ]);
            let broker = broker(port, &key);
            let preview = broker.bootstrap("room", "scout").unwrap();
            assert_eq!(preview.owner_member_id, "member-owner");
            assert_eq!(daemon.join().unwrap().len(), 3);
            assert_eq!(broker.pending.lock().entries.len(), 1);
        }
        // A preview naming another owner is refused and freezes nothing.
        for reply in [
            refusal("409 Conflict", "room_owner_conflict"),
            http("201 Created", &bootstrapped(true)),
        ] {
            let (port, daemon) = fake_daemon(vec![
                Some(http("200 OK", &me_body("member-me"))),
                Some(reply),
                Some(http("200 OK", &preview_body(true, None))),
            ]);
            let broker = broker(port, &key);
            assert_eq!(
                broker.bootstrap("room", "scout").unwrap_err(),
                "room_owner_conflict"
            );
            assert_eq!(daemon.join().unwrap().len(), 3);
            assert!(broker.pending.lock().entries.is_empty());
        }
    }

    #[test]
    fn bootstrap_not_sent_and_unknown_outcomes_are_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        // Lost acknowledgement: unknown, no retry, no preview.
        let (port, daemon) =
            fake_daemon(vec![Some(http("200 OK", &me_body("member-owner"))), None]);
        let broker_a = broker(port, &key);
        assert_eq!(
            broker_a.bootstrap("room", "scout").unwrap_err(),
            "outcome_unknown"
        );
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].contains(KEY), "the bootstrap did reach the daemon");
        assert!(broker_a.pending.lock().entries.is_empty());

        // A bad reply after sending is unknown too.
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &me_body("member-owner"))),
            Some(http("200 OK", "not json")),
        ]);
        assert_eq!(
            broker(port, &key).bootstrap("room", "scout").unwrap_err(),
            "outcome_unknown"
        );
        assert_eq!(daemon.join().unwrap().len(), 2);

        // A refused credential: only the unprivileged owner read went out.
        let loose = key_file(dir.path(), 0o644);
        let (port, daemon) = fake_daemon(vec![Some(http("200 OK", &me_body("member-owner")))]);
        assert_eq!(
            broker(port, &loose).bootstrap("room", "scout").unwrap_err(),
            "operator_credential_not_private"
        );
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].starts_with("GET /v1/me ") && !seen[0].contains(KEY));

        // An unreachable daemon, an untrusted URL or invalid ids send nothing.
        let unreachable = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        assert_eq!(
            broker(unreachable, &key)
                .bootstrap("room", "scout")
                .unwrap_err(),
            "daemon_unreachable"
        );
        let untrusted =
            ConsentBroker::new(DaemonTransport::new("http://localhost:4780", key.clone()));
        assert_eq!(
            untrusted.bootstrap("room", "scout").unwrap_err(),
            "consent_unavailable"
        );
        let (port, listener) = silent_daemon();
        for (room, package) in [("a/b", "scout"), ("room", ".."), ("", "scout")] {
            assert_eq!(
                broker(port, &key).bootstrap(room, package).unwrap_err(),
                "invalid_request"
            );
        }
        assert_never_contacted(&listener);
    }

    #[test]
    fn bootstrap_answers_only_the_main_window() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, listener) = silent_daemon();
        let broker_b = broker(port, &key);
        for label in ["floating", "Main", "", "main-2"] {
            assert_eq!(
                bootstrap_from(label, &broker_b, "room", "scout").unwrap_err(),
                "consent_window_refused",
                "{label:?}"
            );
        }
        assert_never_contacted(&listener);

        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", &me_body("member-owner"))),
            Some(http("201 Created", &bootstrapped(true))),
            Some(http("200 OK", &preview_body(true, None))),
        ]);
        let preview = bootstrap_from("main", &broker(port, &key), " room ", " scout ").unwrap();
        assert_eq!(preview.room, "room");
        assert_eq!(daemon.join().unwrap().len(), 3);
    }
}
