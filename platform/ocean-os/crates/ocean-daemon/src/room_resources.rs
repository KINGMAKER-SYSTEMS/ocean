//! Rooms Phase 2 Stage 2c — local contributed folders.
//!
//! Public candidate scope is recorded in this crate's `AGENTS.md`; imported
//! stage labels do not establish architecture acceptance or deployment approval.
//!
//! # What this module owns
//!
//! 1. **The grant flow.** An operator names a folder on THIS node and the
//!    daemon refuses the dangerous roots Decision 5 and manifest §4 fix — the
//!    filesystem root, the home directory itself, a root that canonicalizes
//!    elsewhere through a symlink, one it cannot open as a directory
//!    descriptor — before the store ever sees the path.
//! 2. **Custody of `local_root`.** The path is read from the store for two
//!    purposes only: to become a turn's cwd, and to confine a relative path.
//!    [`resource_projection`] is the single projection every route and
//!    `inspect` share, and it does not carry the root. Tests pin that no
//!    response body contains it.
//! 3. **Path confinement** ([`confine`], owned by `ocean-agent` beside the
//!    tools). Authorization is rooted in the canonical root, not a string
//!    prefix: the relative path is normalized, joined, canonicalized again, and
//!    refused unless the RESULT is under the root. A symlink inside the folder
//!    that points outside is an escape, whatever its name says.
//! 4. **The cwd rule** ([`resolve_turn_cwd`], manifest §5 as ruled in §11.4):
//!    the agent's own `agent_defaults` entry, then the profile's
//!    `default_resource_id`, then `Room.workspace_root`, else refused. The
//!    convene path and `inspect` call the same function so what the operator
//!    sees is what the turn gets.
//!
//! 5. **The authority** ([`DurableRoomResourceAuthority`], Stage 2d): what the
//!    `room_list`/`room_read` tools in `ocean-agent` consult on every call. It
//!    re-validates the binding generation and the grant on each call and
//!    records one audit row per operation; the operator preview routes run
//!    the SAME tools under the agent's current generation so an operator can
//!    see exactly what the agent would.
//!
//! `write` and `execute` may be RECORDED so intent is visible, and every
//! operation needing them is refused until Phases 4 and 5.

use std::collections::BTreeSet;
use std::path::{Component, Path as FsPath, PathBuf};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use ocean_core::RoomKey;
use ocean_store::{
    GrantRoomResourceInput, ResourceAccessMode, ResourceStatus, RoomResourceGrant, RoomStore,
    RoomStoreError, SetResourceStatusInput,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::AppState;
use crate::persistent_rooms::{persisted_room_workspace, publish_room_wake, with_rooms};
use crate::room_agent_authority::{
    decision_digest, operator, validate_decision_id, validate_member_id, ApiError,
};

const MAX_AGENTS_PER_GRANT: usize = 64;
const MAX_DISPLAY_NAME_CHARS: usize = 64;

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrantBody {
    decision_id: String,
    display_name: String,
    local_root: String,
    access_mode: String,
    #[serde(default)]
    authorized_agent_member_ids: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatusBody {
    decision_id: String,
}

#[derive(Serialize)]
struct GrantDecisionDigestInput<'a> {
    room_id: &'a str,
    display_name: &'a str,
    local_root: &'a str,
    access_mode: &'a str,
    authorized_agent_member_ids: &'a [String],
    expires_at: Option<&'a str>,
}

#[derive(Serialize)]
struct StatusDecisionDigestInput<'a> {
    room_id: &'a str,
    resource_id: &'a str,
    status: &'a str,
}

// ── Dangerous roots and canonical handles ─────────────────────────────────────

/// Why a submitted root was refused. Each maps to its own typed code so the
/// Surface can say which rule fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RootRefusal {
    /// Not an absolute path.
    NotAbsolute,
    /// Does not exist.
    NotFound,
    /// Exists but is not a directory, or cannot be opened as one.
    NotDirectory,
    /// The filesystem root, the home directory itself, or a root that
    /// canonicalizes somewhere other than itself.
    Dangerous,
}

impl RootRefusal {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::NotAbsolute => "invalid_local_root",
            Self::NotFound => "local_root_not_found",
            Self::NotDirectory => "local_root_not_directory",
            Self::Dangerous => "dangerous_root",
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| std::fs::canonicalize(home).ok())
}

/// Open the root as a directory descriptor (O_DIRECTORY, no symlink follow at
/// the leaf). Proves the root is a real directory the daemon can hold a stable
/// confined handle to, which is what Decision 5 asks of a grant root.
#[cfg(unix)]
fn open_directory_handle(path: &FsPath) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_directory_handle(path: &FsPath) -> std::io::Result<std::fs::File> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        std::fs::File::open(path)
    } else {
        Err(std::io::Error::other("not a directory"))
    }
}

/// Validate a submitted root against the manifest §4 list and return its
/// canonical form. The canonical form MUST equal the submitted form: a root
/// that reaches its real directory through a symlink is refused, because the
/// operator approved the name they typed and the grant would bind somewhere
/// else.
fn normalized_grant_root(submitted: &str) -> Result<PathBuf, RootRefusal> {
    let path = FsPath::new(submitted.trim());
    if !path.is_absolute() || submitted.trim().is_empty() {
        return Err(RootRefusal::NotAbsolute);
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(RootRefusal::NotAbsolute);
    }
    Ok(path.components().collect())
}

pub(super) fn canonical_grant_root(submitted: &str) -> Result<PathBuf, RootRefusal> {
    let path = normalized_grant_root(submitted)?;
    let canonical = match std::fs::canonicalize(&path) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RootRefusal::NotFound)
        }
        Err(_) => return Err(RootRefusal::NotDirectory),
    };
    if canonical != path {
        return Err(RootRefusal::Dangerous);
    }
    if canonical.parent().is_none() {
        return Err(RootRefusal::Dangerous);
    }
    if home_dir().is_some_and(|home| home == canonical) {
        return Err(RootRefusal::Dangerous);
    }
    match open_directory_handle(&canonical) {
        Ok(handle) => drop(handle),
        Err(_) => return Err(RootRefusal::NotDirectory),
    }
    Ok(canonical)
}

/// Path confinement lives in `ocean-agent` beside the tools that use it
/// (Stage 2d); re-exported so the daemon's tests pin the same rule the tools
/// enforce.
#[cfg(test)]
pub(super) use ocean_agent::{confine_room_resource_path as confine, ConfineRefusal};

// ── Projection ────────────────────────────────────────────────────────────────

/// The architecture's §7.4 safe projection. No `local_root`, no digest.
pub(super) fn resource_projection(grant: &RoomResourceGrant, now: DateTime<Utc>) -> Value {
    json!({
        "room_id": grant.room_id,
        "resource_id": grant.resource_id,
        "display_name": grant.display_name,
        "resource_kind": grant.resource_kind,
        "access_mode": grant.access_mode.as_str(),
        "authorized_agent_member_ids": grant.authorized_agent_member_ids,
        "status": grant.effective_status(now).as_str(),
        "generation": grant.generation.to_string(),
        "expires_at": grant.expires_at,
        "granted_by": grant.granted_by,
        "granted_at": grant.granted_at,
        "revoked_at": grant.revoked_at,
        "owner_node": "local",
        "decision_id": grant.decision_id,
    })
}

pub(super) fn resources_projection(grants: &[RoomResourceGrant]) -> Value {
    let now = Utc::now();
    json!(grants
        .iter()
        .map(|g| resource_projection(g, now))
        .collect::<Vec<_>>())
}

// ── The cwd rule ──────────────────────────────────────────────────────────────

/// Which manifest §5 rule chose a turn's cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnCwd {
    /// A live grant that authorizes the agent; the root stays private.
    ResourceGrant {
        resource_id: String,
        generation: u64,
        cwd: String,
    },
    /// `Room.workspace_root`, already public on the room record.
    RoomWorkspaceRoot { cwd: String },
    /// Nothing usable: the turn is refused with `workspace_unavailable`.
    Unbound,
}

impl TurnCwd {
    pub(super) fn cwd(&self) -> Option<&str> {
        match self {
            Self::ResourceGrant { cwd, .. } | Self::RoomWorkspaceRoot { cwd } => Some(cwd),
            Self::Unbound => None,
        }
    }
}

/// Manifest §5, as ruled in §11.4. A grant is usable as a cwd only if it is
/// `available`, authorizes the agent, and its root still canonicalizes to
/// itself as a directory — the same liveness bar `Room.workspace_root` meets.
pub(super) fn resolve_turn_cwd(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    agent_member_id: &str,
) -> Result<TurnCwd, RoomStoreError> {
    let now = Utc::now();
    let profile = store.room_profile(room)?;
    let candidates = profile
        .as_ref()
        .map(|p| {
            p.agent_defaults
                .get(agent_member_id)
                .cloned()
                .into_iter()
                .chain(p.default_resource_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for resource_id in candidates {
        let Some(grant) = store.room_resource_grant(room, &resource_id)? else {
            continue;
        };
        if !grant.admits(agent_member_id, ResourceAccessMode::List, now) {
            continue;
        }
        if let Some(cwd) = persisted_room_workspace(&grant.local_root) {
            return Ok(TurnCwd::ResourceGrant {
                resource_id: grant.resource_id,
                generation: grant.generation,
                cwd,
            });
        }
    }
    let workspace = store
        .get(room)?
        .and_then(|record| record.room.workspace_root)
        .as_deref()
        .and_then(persisted_room_workspace);
    Ok(match workspace {
        Some(cwd) => TurnCwd::RoomWorkspaceRoot { cwd },
        None => TurnCwd::Unbound,
    })
}

// ── Routes ────────────────────────────────────────────────────────────────────

fn room_key(key: &str) -> Result<RoomKey, ApiError> {
    let room = RoomKey::new(key.trim());
    if room.as_str().is_empty() {
        return Err(ApiError::bad_request("invalid_room_key"));
    }
    Ok(room)
}

pub(super) async fn room_resources_list(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<Value>) {
    let result = (|| -> Result<Value, ApiError> {
        let room = room_key(&key)?;
        let grants = with_rooms(&state, |store| store.room_resource_grants(&room))
            .map_err(ApiError::from)?;
        Ok(json!({ "ok": true, "resources": resources_projection(&grants) }))
    })();
    match result {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err(error) => error.response(),
    }
}

pub(super) async fn room_resource_get(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let result = (|| -> Result<Value, ApiError> {
        let room = room_key(&key)?;
        let grant = with_rooms(&state, |store| {
            store.room_resource_grant(&room, resource_id.trim())
        })
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("resource_not_found"))?;
        Ok(json!({ "ok": true, "resource": resource_projection(&grant, Utc::now()) }))
    })();
    match result {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err(error) => error.response(),
    }
}

pub(super) async fn room_resource_grant(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: Result<Json<GrantBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let result = (|| {
        let principal = operator(&state, &headers)?;
        let Json(body) = body.map_err(|_| ApiError::bad_request("invalid_request"))?;
        let room = room_key(&key)?;
        let decision_id = validate_decision_id(&body.decision_id)?;

        let display_name = body.display_name.trim();
        let ok_name = display_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && display_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            && display_name.chars().count() <= MAX_DISPLAY_NAME_CHARS;
        if !ok_name {
            return Err(ApiError::bad_request("invalid_display_name"));
        }
        let access_mode = ResourceAccessMode::parse(body.access_mode.trim())
            .ok_or_else(|| ApiError::bad_request("invalid_access_mode"))?;
        if body.authorized_agent_member_ids.len() > MAX_AGENTS_PER_GRANT {
            return Err(ApiError::bad_request("too_many_agents"));
        }
        let mut agents = body
            .authorized_agent_member_ids
            .iter()
            .map(|id| validate_member_id(id, "invalid_agent_member_id"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        agents.retain(|id| !id.is_empty());
        let agents = agents.into_iter().collect::<Vec<_>>();
        let expires_at = body
            .expires_at
            .as_deref()
            .map(|raw| {
                DateTime::parse_from_rfc3339(raw.trim())
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(|_| ApiError::bad_request("invalid_expires_at"))
            })
            .transpose()?;

        // Normalize accepted path spelling without touching the filesystem.
        // Consumed decisions must replay even after expiry or root removal.
        let canonical_root = normalized_grant_root(&body.local_root)
            .map_err(|refusal| ApiError::bad_request(refusal.code()))?;
        let local_root = canonical_root
            .to_str()
            .ok_or_else(|| ApiError::bad_request("invalid_local_root"))?
            .to_string();
        let expires_text = expires_at.map(|t| t.to_rfc3339());
        let digest = decision_digest(&GrantDecisionDigestInput {
            room_id: room.as_str(),
            display_name,
            local_root: &local_root,
            access_mode: access_mode.as_str(),
            authorized_agent_member_ids: &agents,
            expires_at: expires_text.as_deref(),
        })?;
        let (grant, created, audit) = with_rooms(&state, |store| {
            // This lookup only selects fresh-request validation. The store's
            // transaction still decides exact, mismatched and cross-ledger replay.
            if store.room_decision_consumed(&room, &decision_id)?.is_none() {
                if expires_at.is_some_and(|expires| expires <= Utc::now()) {
                    return Err(ApiError::bad_request("invalid_expires_at"));
                }
                canonical_grant_root(&body.local_root)
                    .map_err(|refusal| ApiError::bad_request(refusal.code()))?;
            }
            store
                .grant_room_resource(
                    &room,
                    GrantRoomResourceInput {
                        display_name: display_name.to_string(),
                        local_root,
                        access_mode,
                        authorized_agent_member_ids: agents,
                        expires_at,
                        granted_by: principal.id().to_string(),
                        decision_id,
                        request_digest: digest,
                    },
                    Utc::now(),
                )
                .map_err(ApiError::from)
        })?;
        if let Some(audit) = audit.as_ref() {
            publish_room_wake(&state, &room, audit);
        }
        Ok((
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({
                "ok": true,
                "created": created,
                "resource": resource_projection(&grant, Utc::now()),
            }),
        ))
    })();
    match result {
        Ok((status, body)) => (status, Json(body)),
        Err(error) => error.response(),
    }
}

async fn set_status(
    state: AppState,
    key: String,
    resource_id: String,
    headers: HeaderMap,
    body: Result<Json<StatusBody>, JsonRejection>,
    status: ResourceStatus,
) -> (StatusCode, Json<Value>) {
    let result = (|| {
        let principal = operator(&state, &headers)?;
        let Json(body) = body.map_err(|_| ApiError::bad_request("invalid_request"))?;
        let room = room_key(&key)?;
        let resource_id = resource_id.trim().to_string();
        if resource_id.is_empty() {
            return Err(ApiError::bad_request("invalid_request"));
        }
        let decision_id = validate_decision_id(&body.decision_id)?;
        let digest = decision_digest(&StatusDecisionDigestInput {
            room_id: room.as_str(),
            resource_id: &resource_id,
            status: status.as_str(),
        })?;
        let (grant, changed, audit) = with_rooms(&state, |store| {
            store.set_room_resource_status(
                &room,
                &resource_id,
                SetResourceStatusInput {
                    status,
                    actor: principal.id().to_string(),
                    decision_id,
                    request_digest: digest,
                },
                Utc::now(),
            )
        })
        .map_err(ApiError::from)?;
        if let Some(audit) = audit.as_ref() {
            publish_room_wake(&state, &room, audit);
        }
        Ok(json!({
            "ok": true,
            "changed": changed,
            "resource": resource_projection(&grant, Utc::now()),
        }))
    })();
    match result {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err(error) => error.response(),
    }
}

pub(super) async fn room_resource_suspend(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<StatusBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    set_status(
        state,
        key,
        resource_id,
        headers,
        body,
        ResourceStatus::Suspended,
    )
    .await
}

pub(super) async fn room_resource_resume(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<StatusBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    set_status(
        state,
        key,
        resource_id,
        headers,
        body,
        ResourceStatus::Available,
    )
    .await
}

pub(super) async fn room_resource_revoke(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<StatusBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    set_status(
        state,
        key,
        resource_id,
        headers,
        body,
        ResourceStatus::Revoked,
    )
    .await
}

// ── The daemon-owned authority (Stage 2d) ─────────────────────────────────────

/// What the tools consult on EVERY call. Re-validates the binding generation
/// the handle was minted under and the grant's effective status, agent list,
/// access mode, and root liveness, then hands back the root for exactly one
/// operation. Nothing is cached across calls (Decision 12).
pub(super) struct DurableRoomResourceAuthority {
    pub(super) authority: crate::room_agent_authority::RoomOperationAuthority,
    /// Fixed `agent` classification for the admitted turn's audit row.
    pub(super) actor: &'static str,
}

fn needed_mode(op: ocean_agent::RoomResourceOp) -> ResourceAccessMode {
    match op {
        ocean_agent::RoomResourceOp::List => ResourceAccessMode::List,
        ocean_agent::RoomResourceOp::Read => ResourceAccessMode::Read,
    }
}

#[async_trait::async_trait]
impl ocean_agent::RoomResourceAuthority for DurableRoomResourceAuthority {
    async fn resolve(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        resource_id: &str,
        op: ocean_agent::RoomResourceOp,
    ) -> Result<ocean_agent::ResolvedResource, ocean_agent::RoomResourceError> {
        use ocean_agent::RoomResourceError as E;
        if scope.room_key() != self.authority.room.as_str()
            || scope.agent_member_id() != self.authority.member
            || scope.binding_generation() != self.authority.generation
        {
            return Err(E::StaleGeneration);
        }
        let room = RoomKey::new(scope.room_key());
        crate::persistent_rooms::with_rooms_handle(&self.authority.rooms, |store| {
            if self.authority.cancel.is_cancelled() {
                return Err(E::StaleGeneration);
            }
            let live = crate::room_agent_authority::current_binding_on(
                store,
                &room,
                scope.agent_member_id(),
                scope.binding_generation(),
            )
            .map_err(|_| E::Unavailable("room_store_unavailable".into()))?
            .is_some_and(|binding| binding.agent_definition_digest == self.authority.digest);
            if !live {
                return Err(E::StaleGeneration);
            }
            let grant = store
                .room_resource_grant(&room, resource_id)
                .map_err(|_| E::Unavailable("room_store_unavailable".into()))?
                .ok_or(E::NotFound)?;
            let now = Utc::now();
            if grant.effective_status(now) != ResourceStatus::Available {
                return Err(E::NotAvailable);
            }
            if !grant.authorizes_agent(scope.agent_member_id()) {
                return Err(E::AgentNotAuthorized);
            }
            if !grant.access_mode.allows(needed_mode(op)) {
                return Err(E::ModeNotGranted);
            }
            if !FsPath::new(&grant.local_root).is_absolute() {
                return Err(E::NotAvailable);
            }
            let root = grant.local_root;
            if self.authority.cancel.is_cancelled() {
                return Err(E::StaleGeneration);
            }
            Ok(ocean_agent::ResolvedResource {
                local_root: PathBuf::from(root),
                grant_generation: grant.generation,
            })
        })
    }

    async fn record(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        fact: ocean_agent::RoomResourceAuditFact,
    ) {
        let room = RoomKey::new(scope.room_key());
        let result = crate::persistent_rooms::with_rooms_handle(&self.authority.rooms, |store| {
            store.append_room_resource_audit(
                &room,
                ocean_store::RoomResourceAuditInput {
                    resource_id: fact.resource_id,
                    agent_member_id: scope.agent_member_id().to_string(),
                    binding_generation: scope.binding_generation(),
                    grant_generation: fact.grant_generation,
                    op: fact.op.as_str().to_string(),
                    relative_path_digest: fact.relative_path_digest,
                    bytes: fact.bytes,
                    entries: fact.entries,
                    outcome: fact.outcome,
                    actor: self.actor.to_string(),
                },
                Utc::now(),
            )
        });
        if result.is_err() {
            tracing::warn!(room = %room, error_code = "resource_audit_failed", "room resource audit row not recorded");
        }
    }
}

/// Every grant that admits `agent` for at least `list`, as the catalog the
/// tool description shows. Empty means the turn gets no resource tools.
pub(super) fn admitted_resource_catalog(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    agent_member_id: &str,
) -> Result<Vec<ocean_agent::RoomResourceCatalogEntry>, RoomStoreError> {
    let now = Utc::now();
    Ok(store
        .room_resource_grants(room)?
        .into_iter()
        .filter(|grant| grant.admits(agent_member_id, ResourceAccessMode::List, now))
        .map(|grant| ocean_agent::RoomResourceCatalogEntry {
            resource_id: grant.resource_id,
            display_name: grant.display_name,
            // Phase 2d exposes list/read only; report what the tools can do,
            // not the recorded intent.
            access_mode: if grant.access_mode.allows(ResourceAccessMode::Read) {
                "read".into()
            } else {
                "list".into()
            },
        })
        .collect())
}

// ── Operator preview routes (Stage 2d) ────────────────────────────────────────

/// The admission evidence a preview runs under: the agent's CURRENT active
/// binding generation, so the preview sees exactly what the agent would.
struct PreviewAdmission {
    room: String,
    agent_member_id: String,
    generation: u64,
}

impl ocean_agent::RoomResourceAdmission for PreviewAdmission {
    fn admitted_room_key(&self) -> &str {
        &self.room
    }
    fn admitted_agent_member_id(&self) -> &str {
        &self.agent_member_id
    }
    fn admitted_generation(&self) -> u64 {
        self.generation
    }
}

/// Request-local preview custody; never borrows or cancels an agent turn.
struct PreviewResourceAuthority(DurableRoomResourceAuthority);

#[async_trait::async_trait]
impl ocean_agent::RoomResourceAuthority for PreviewResourceAuthority {
    async fn resolve(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        resource_id: &str,
        op: ocean_agent::RoomResourceOp,
    ) -> Result<ocean_agent::ResolvedResource, ocean_agent::RoomResourceError> {
        self.0.resolve(scope, resource_id, op).await
    }

    async fn record(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        fact: ocean_agent::RoomResourceAuditFact,
    ) {
        self.0.record(scope, fact).await
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreviewBody {
    agent_member_id: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    max_bytes: Option<usize>,
}

async fn preview(
    state: AppState,
    key: String,
    resource_id: String,
    headers: HeaderMap,
    body: Result<Json<PreviewBody>, JsonRejection>,
    tool_name: &str,
) -> (StatusCode, Json<Value>) {
    let prepared = (|| {
        let _principal = operator(&state, &headers)?;
        let Json(body) = body.map_err(|_| ApiError::bad_request("invalid_request"))?;
        let room = room_key(&key)?;
        let agent = validate_member_id(&body.agent_member_id, "invalid_agent_member_id")?;
        if agent.is_empty() {
            return Err(ApiError::bad_request("invalid_request"));
        }
        let (binding, catalog) = with_rooms(&state, |store| {
            let binding = store
                .room_agent_binding(&room, &agent)?
                .map(|binding| {
                    crate::room_agent_authority::current_binding_on(
                        store,
                        &room,
                        &agent,
                        binding.generation,
                    )
                })
                .transpose()?
                .flatten();
            let catalog = admitted_resource_catalog(store, &room, &agent)?;
            Ok::<_, RoomStoreError>((binding, catalog))
        })
        .map_err(ApiError::from)?;
        let binding = binding.ok_or_else(|| ApiError::conflict("agent_binding_required"))?;
        let generation = binding.generation;
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_on_drop = cancel.clone().drop_guard();
        let authority = crate::room_agent_authority::RoomOperationAuthority {
            rooms: state.rooms.clone(),
            room: room.clone(),
            member: agent.clone(),
            generation,
            digest: binding.agent_definition_digest,
            cancel,
        };
        let admitted = state
            .runtime
            .admit_room_resources(
                &PreviewAdmission {
                    room: room.as_str().to_string(),
                    agent_member_id: agent.clone(),
                    generation,
                },
                std::sync::Arc::new(PreviewResourceAuthority(DurableRoomResourceAuthority {
                    authority,
                    actor: "operator_preview",
                })),
                catalog,
            )
            .map_err(|_| ApiError::internal("room_resources_unavailable"))?;
        let mut args = json!({ "resource_id": resource_id.trim(), "path": body.path });
        if tool_name == ocean_agent::ROOM_READ_TOOL {
            if let Some(offset) = body.offset {
                args["offset"] = json!(offset);
            }
            if let Some(max_bytes) = body.max_bytes {
                args["max_bytes"] = json!(max_bytes);
            }
        }
        Ok((admitted, args, agent, generation, cancel_on_drop))
    })();
    let (admitted, args, agent, generation, _cancel_on_drop) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return error.response(),
    };
    let Some(tool) = admitted.tools().into_iter().find(|t| t.name() == tool_name) else {
        return ApiError::internal("room_resources_unavailable").response();
    };
    match tool.execute("operator-preview", args).await {
        Ok(result) => {
            let text = result
                .content
                .iter()
                .find_map(|c| match c {
                    ocean_protocol::Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let mut body: Value = serde_json::from_str(&text)
                .unwrap_or(json!({ "ok": false, "error": "malformed_tool_result" }));
            body["via"] = json!("operator_preview");
            body["agent_member_id"] = json!(agent);
            body["binding_generation"] = json!(generation.to_string());
            (StatusCode::OK, Json(body))
        }
        Err(_) => ApiError::bad_request("invalid_request").response(),
    }
}

pub(super) async fn room_resource_preview_list(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<PreviewBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    preview(
        state,
        key,
        resource_id,
        headers,
        body,
        ocean_agent::ROOM_LIST_TOOL,
    )
    .await
}

pub(super) async fn room_resource_preview_read(
    State(state): State<AppState>,
    Path((key, resource_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<PreviewBody>, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    preview(
        state,
        key,
        resource_id,
        headers,
        body,
        ocean_agent::ROOM_READ_TOOL,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use ocean_core::{RoomParticipant, RoomParticipantKind};
    use ocean_store::{ActivationPolicy, AuthorizeAgentInput, ContextPolicy, MemoryScope};

    fn operation_fixture() -> crate::room_agent_authority::RoomOperationAuthority {
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("operation-authority");
        store
            .create(room.clone(), "Authority", None, Utc::now())
            .unwrap();
        store
            .add_participant(
                &room,
                RoomParticipant {
                    id: "human".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "Human".into(),
                },
                Utc::now(),
            )
            .unwrap();
        store
            .bootstrap_local_room_agent(
                &room,
                "human",
                RoomParticipant {
                    id: "builder".into(),
                    kind: RoomParticipantKind::Agent,
                    display_name: "Builder".into(),
                },
                "builder",
                "fixture-operator",
                Utc::now(),
            )
            .unwrap();
        let (binding, _, _) = store
            .authorize_room_agent(
                &room,
                AuthorizeAgentInput {
                    agent_member_id: "builder".into(),
                    agent_package_id: "builder".into(),
                    agent_definition_digest: "sha256:fixture".into(),
                    agent_definition_revision: None,
                    display_name: "Builder".into(),
                    owner_member_id: "human".into(),
                    authorized_by: "fixture-operator".into(),
                    activation_policy: ActivationPolicy::Mention,
                    context_policy: ContextPolicy::InvocationOnly,
                    memory_scope: MemoryScope::None,
                    requested_capabilities: vec![],
                    room_capability_grants: vec![],
                    decision_id: "fixture-decision".into(),
                    request_digest: "fixture-request".into(),
                },
                Utc::now(),
            )
            .unwrap();
        crate::room_agent_authority::RoomOperationAuthority {
            rooms: std::sync::Arc::new(std::sync::Mutex::new(store)),
            room,
            member: "builder".into(),
            generation: binding.generation,
            digest: binding.agent_definition_digest,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn resource_authority_retains_turn_custody_and_preview_permissions() {
        use crate::tests::{isolated_room_fixture_state, TestEnvRestore, AUTO_CONVENE_ENV_LOCK};
        use ocean_agent::{RoomResourceAuthority, RoomResourceError, RoomResourceOp};
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let mut state = isolated_room_fixture_state(&tmp);
        for refusal in [
            "cancel", "scope", "digest", "owner", "access", "revoke", "suspend",
        ] {
            let mut operation = operation_fixture();
            state.rooms = operation.rooms.clone();
            let resource = with_rooms(&state, |store| {
                store
                    .grant_room_resource(
                        &operation.room,
                        GrantRoomResourceInput {
                            display_name: "source".into(),
                            local_root: tmp
                                .path()
                                .canonicalize()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            access_mode: ResourceAccessMode::Read,
                            authorized_agent_member_ids: vec!["builder".into()],
                            expires_at: None,
                            granted_by: "fixture-operator".into(),
                            decision_id: "builder-grant".into(),
                            request_digest: "builder-grant".into(),
                        },
                        Utc::now(),
                    )
                    .unwrap()
                    .0
            });
            let authority = std::sync::Arc::new(DurableRoomResourceAuthority {
                authority: operation.clone(),
                actor: "agent",
            });
            let admitted = state
                .runtime
                .admit_room_resources(
                    &PreviewAdmission {
                        room: operation.room.to_string(),
                        agent_member_id: operation.member.clone(),
                        generation: operation.generation,
                    },
                    authority.clone(),
                    vec![],
                )
                .unwrap();
            assert!(authority
                .resolve(
                    admitted.scope(),
                    &resource.resource_id,
                    RoomResourceOp::Read
                )
                .await
                .is_ok());
            match refusal {
                "cancel" => operation.cancel.cancel(),
                "scope" => operation.member = "different-member".into(),
                "digest" => operation.digest = "different-definition".into(),
                "owner" => {
                    with_rooms(&state, |store| {
                        store.remove_participant(&operation.room, "human", Utc::now())
                    })
                    .unwrap();
                }
                "access" => {
                    with_rooms(&state, |store| {
                        let mut access = store.room_access(&operation.room).unwrap();
                        access.state = ocean_core::RoomAccessState::Revoked;
                        store.replace_room_access(&operation.room, &access)
                    })
                    .unwrap();
                }
                "revoke" | "suspend" => {
                    with_rooms(&state, |store| {
                        store.set_room_resource_status(
                            &operation.room,
                            &resource.resource_id,
                            SetResourceStatusInput {
                                status: if refusal == "revoke" {
                                    ResourceStatus::Revoked
                                } else {
                                    ResourceStatus::Suspended
                                },
                                actor: "fixture-operator".into(),
                                decision_id: refusal.into(),
                                request_digest: refusal.into(),
                            },
                            Utc::now(),
                        )
                    })
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let authority = DurableRoomResourceAuthority {
                authority: operation,
                actor: "agent",
            };
            let result = authority
                .resolve(
                    admitted.scope(),
                    &resource.resource_id,
                    RoomResourceOp::Read,
                )
                .await;
            if matches!(refusal, "revoke" | "suspend") {
                assert!(
                    matches!(result, Err(RoomResourceError::NotAvailable)),
                    "{refusal}: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(RoomResourceError::StaleGeneration)),
                    "{refusal}: {result:?}"
                );
            }
        }

        // Preview receives its own request custody and must pass the operator gate.
        let operation = operation_fixture();
        state.rooms = operation.rooms.clone();
        std::fs::write(tmp.path().join("fixture.txt"), "fixture-content").unwrap();
        let resource = with_rooms(&state, |store| {
            store.grant_room_resource(
                &operation.room,
                GrantRoomResourceInput {
                    display_name: "source".into(),
                    local_root: tmp
                        .path()
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    access_mode: ResourceAccessMode::Read,
                    authorized_agent_member_ids: vec!["builder".into()],
                    expires_at: None,
                    granted_by: "fixture-operator".into(),
                    decision_id: "preview-grant".into(),
                    request_digest: "preview-grant".into(),
                },
                Utc::now(),
            )
        })
        .unwrap()
        .0;
        let app = crate::room_routes().with_state(state.clone());
        let path = format!(
            "/v1/rooms/persistent/{}/resources/{}/read",
            operation.room, resource.resource_id
        );
        let body = json!({"agent_member_id": "builder", "path": "fixture.txt"});
        assert_eq!(
            route_request(
                app.clone(),
                axum::http::Method::POST,
                &path,
                body.clone(),
                false
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (status, reply) = route_request(
            app.clone(),
            axum::http::Method::POST,
            &path,
            body.clone(),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(reply.to_string().contains("fixture-content"));
        assert!(
            !operation.cancel.is_cancelled(),
            "preview must not cancel an agent's independent token"
        );
        with_rooms(&state, |store| {
            store.remove_participant(&operation.room, "human", Utc::now())
        })
        .unwrap();
        let (status, reply) = route_request(app, axum::http::Method::POST, &path, body, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{reply}");
        assert_eq!(reply["error"], "agent_binding_required");
    }

    async fn route_request(
        app: axum::Router,
        method: axum::http::Method,
        path: &str,
        body: Value,
        operator: bool,
    ) -> (StatusCode, Value) {
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if operator {
            request = request.header("x-ocean-operator", "test-room-operator");
        }
        let response = app
            .oneshot(
                request
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn grant_route_replays_after_root_removal_and_expiry() {
        use crate::tests::{isolated_room_fixture_state, TestEnvRestore, AUTO_CONVENE_ENV_LOCK};
        let _yolo_guard = crate::tests::yolo_env_guard_async().await;
        let _guard = AUTO_CONVENE_ENV_LOCK.lock().await;
        let _restore = TestEnvRestore::capture(&[
            "OCEAN_AUTH_FILE",
            "OCEAN_CODEX_AUTH_FILE",
            "OCEAN_CONFIG_DIR",
            "OCEAN_MODEL",
            "OCEAN_YOLO",
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let state = isolated_room_fixture_state(&tmp);
        let room = RoomKey::new("resource-replay-route");
        with_rooms(&state, |store| {
            store.create(room.clone(), "Replay", None, Utc::now())
        })
        .unwrap();
        let app = crate::room_routes().with_state(state.clone());
        let path = format!("/v1/rooms/persistent/{room}/resources");
        let root = tmp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap().to_string_lossy().into_owned();
        let body = json!({"decision_id": uuid::Uuid::new_v4().to_string(), "display_name": "source", "local_root": format!("{root}///"), "access_mode": "read"});
        let (status, original) = route_request(
            app.clone(),
            axum::http::Method::POST,
            &path,
            body.clone(),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{original}");
        std::fs::remove_dir(&root).unwrap();
        let before = with_rooms(&state, |store| store.transcript(&room, None)).unwrap();
        let (status, replay) = route_request(
            app.clone(),
            axum::http::Method::POST,
            &path,
            body.clone(),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay["created"], false);
        assert_eq!(replay["resource"], original["resource"]);
        let mut changed = body.clone();
        changed["access_mode"] = json!("list");
        let (status, rejected) =
            route_request(app.clone(), axum::http::Method::POST, &path, changed, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
        let mut fresh = body.clone();
        let fresh_decision = uuid::Uuid::new_v4().to_string();
        fresh["decision_id"] = json!(fresh_decision);
        let (status, rejected) =
            route_request(app.clone(), axum::http::Method::POST, &path, fresh, true).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert_eq!(rejected["error"], "local_root_not_found");
        with_rooms(&state, |store| {
            assert!(store
                .room_decision_consumed(&room, &fresh_decision)
                .unwrap()
                .is_none());
            assert_eq!(store.transcript(&room, None).unwrap(), before);
        });

        // Even an identical digest from another authority ledger is not a grant.
        let cross_decision = uuid::Uuid::new_v4().to_string();
        let digest = decision_digest(&GrantDecisionDigestInput {
            room_id: room.as_str(),
            display_name: "source",
            local_root: &root,
            access_mode: "read",
            authorized_agent_member_ids: &[],
            expires_at: None,
        })
        .unwrap();
        with_rooms(&state, |store| {
            store.put_room_profile(
                &room,
                PutRoomProfileInput {
                    repos: vec![],
                    tools: vec![],
                    credential_slots: vec![],
                    default_resource_id: None,
                    agent_defaults: BTreeMap::new(),
                    updated_by: "fixture-operator".into(),
                    decision_id: cross_decision.clone(),
                    request_digest: digest,
                },
                Utc::now(),
            )
        })
        .unwrap();
        let before = with_rooms(&state, |store| store.transcript(&room, None)).unwrap();
        let mut cross = body;
        cross["decision_id"] = json!(cross_decision);
        let (status, rejected) =
            route_request(app.clone(), axum::http::Method::POST, &path, cross, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
        assert_eq!(rejected["error"], "decision_replay_mismatch");
        assert_eq!(
            with_rooms(&state, |store| store.transcript(&room, None)).unwrap(),
            before
        );

        // Seed a once-valid request in the past without a timing-dependent sleep.
        std::fs::create_dir(&root).unwrap();
        let expires = Utc::now() - chrono::Duration::hours(1);
        let expires_text = expires.to_rfc3339();
        let decision = uuid::Uuid::new_v4().to_string();
        // A separate root avoids the original still-live grant's uniqueness constraint.
        let expired_root = tmp.path().join("expired");
        std::fs::create_dir(&expired_root).unwrap();
        let expired_root = expired_root
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let digest = decision_digest(&GrantDecisionDigestInput {
            room_id: room.as_str(),
            display_name: "expired",
            local_root: &expired_root,
            access_mode: "read",
            authorized_agent_member_ids: &[],
            expires_at: Some(&expires_text),
        })
        .unwrap();
        let original = with_rooms(&state, |store| {
            store.grant_room_resource(
                &room,
                GrantRoomResourceInput {
                    display_name: "expired".into(),
                    local_root: expired_root.clone(),
                    access_mode: ResourceAccessMode::Read,
                    authorized_agent_member_ids: vec![],
                    expires_at: Some(expires),
                    granted_by: "fixture-operator".into(),
                    decision_id: decision.clone(),
                    request_digest: digest,
                },
                expires - chrono::Duration::hours(1),
            )
        })
        .unwrap()
        .0;
        let before = with_rooms(&state, |store| store.transcript(&room, None)).unwrap();
        let body = json!({"decision_id": decision, "display_name": "expired", "local_root": expired_root, "access_mode": "read", "expires_at": expires_text});
        let (status, replay) = route_request(
            app.clone(),
            axum::http::Method::POST,
            &path,
            body.clone(),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay["resource"]["resource_id"], original.resource_id);
        assert_eq!(replay["resource"]["status"], "revoked");
        let mut fresh_expired = body;
        fresh_expired["decision_id"] = json!(uuid::Uuid::new_v4().to_string());
        let (status, rejected) =
            route_request(app, axum::http::Method::POST, &path, fresh_expired, true).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert_eq!(rejected["error"], "invalid_expires_at");
        assert_eq!(
            with_rooms(&state, |store| store.transcript(&room, None)).unwrap(),
            before
        );
    }

    use ocean_store::PutRoomProfileInput;
    use std::collections::BTreeMap;

    fn grant(
        store: &mut ocean_store::SqliteRoomStore,
        room: &RoomKey,
        root: &FsPath,
        decision: &str,
    ) -> ocean_store::RoomResourceGrant {
        store
            .grant_room_resource(
                room,
                GrantRoomResourceInput {
                    display_name: decision.into(),
                    local_root: std::fs::canonicalize(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    access_mode: ResourceAccessMode::Read,
                    authorized_agent_member_ids: vec!["helper".into()],
                    expires_at: None,
                    granted_by: "fixture-operator".into(),
                    decision_id: decision.into(),
                    request_digest: decision.into(),
                },
                Utc::now(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn cwd_prefers_authorized_agent_default_then_room_default_then_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["agent", "room", "workspace"] {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
        }
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("cwd-precedence");
        let workspace = std::fs::canonicalize(tmp.path().join("workspace"))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        store
            .create_in_workspace(
                room.clone(),
                "Cwd",
                Some(workspace.clone()),
                None,
                Utc::now(),
            )
            .unwrap();
        let agent_grant = grant(
            &mut store,
            &room,
            &tmp.path().join("agent"),
            "agent-default",
        );
        let room_grant = grant(&mut store, &room, &tmp.path().join("room"), "room-default");
        store
            .put_room_profile(
                &room,
                PutRoomProfileInput {
                    repos: vec![],
                    tools: vec![],
                    credential_slots: vec![],
                    default_resource_id: Some(room_grant.resource_id.clone()),
                    agent_defaults: BTreeMap::from([(
                        "helper".into(),
                        agent_grant.resource_id.clone(),
                    )]),
                    updated_by: "fixture-operator".into(),
                    decision_id: "profile-defaults".into(),
                    request_digest: "profile-defaults".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(
            matches!(resolve_turn_cwd(&mut store, &room, "helper").unwrap(), TurnCwd::ResourceGrant { resource_id, .. } if resource_id == agent_grant.resource_id)
        );
        store
            .set_room_resource_status(
                &room,
                &agent_grant.resource_id,
                SetResourceStatusInput {
                    status: ResourceStatus::Suspended,
                    actor: "fixture-operator".into(),
                    decision_id: "suspend-agent".into(),
                    request_digest: "suspend-agent".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(
            matches!(resolve_turn_cwd(&mut store, &room, "helper").unwrap(), TurnCwd::ResourceGrant { resource_id, .. } if resource_id == room_grant.resource_id)
        );
        store
            .set_room_resource_status(
                &room,
                &room_grant.resource_id,
                SetResourceStatusInput {
                    status: ResourceStatus::Suspended,
                    actor: "fixture-operator".into(),
                    decision_id: "suspend-room".into(),
                    request_digest: "suspend-room".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::RoomWorkspaceRoot {
                cwd: workspace.clone()
            }
        );
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "different-agent").unwrap(),
            TurnCwd::RoomWorkspaceRoot { cwd: workspace }
        );
        std::fs::remove_dir(tmp.path().join("workspace")).unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::Unbound
        );
    }

    #[cfg(unix)]
    #[test]
    fn readonly_catalog_omits_roots_and_cwd_refuses_root_symlink_substitution() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("cwd-substitution");
        store.create(room.clone(), "Cwd", None, Utc::now()).unwrap();
        let resource = grant(&mut store, &room, &root, "default");
        store
            .put_room_profile(
                &room,
                PutRoomProfileInput {
                    repos: vec![],
                    tools: vec![],
                    credential_slots: vec![],
                    default_resource_id: Some(resource.resource_id.clone()),
                    agent_defaults: BTreeMap::new(),
                    updated_by: "fixture-operator".into(),
                    decision_id: "profile".into(),
                    request_digest: "profile".into(),
                },
                Utc::now(),
            )
            .unwrap();
        let catalog = admitted_resource_catalog(&mut store, &room, "helper").unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].access_mode, "read");
        assert!(admitted_resource_catalog(&mut store, &room, "other-agent")
            .unwrap()
            .is_empty());
        assert!(matches!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::ResourceGrant { .. }
        ));
        std::fs::rename(&root, tmp.path().join("moved-root")).unwrap();
        symlink(&outside, &root).unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::Unbound
        );
    }

    fn canon(p: &FsPath) -> PathBuf {
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn every_dangerous_root_in_the_manifest_list_is_refused_by_name() {
        // §4.1 the filesystem root
        assert_eq!(
            canonical_grant_root("/").unwrap_err(),
            RootRefusal::Dangerous
        );
        // §4.2 the home directory itself (subdirectories are fine)
        if let Some(home) = home_dir() {
            assert_eq!(
                canonical_grant_root(home.to_str().unwrap()).unwrap_err(),
                RootRefusal::Dangerous
            );
        }
        // §4.3 a root that canonicalizes elsewhere through a symlink
        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(
                canonical_grant_root(link.to_str().unwrap()).unwrap_err(),
                RootRefusal::Dangerous
            );
        }
        // §4.4 a root the daemon cannot open as a directory
        let file = tmp.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            canonical_grant_root(canon(&file).to_str().unwrap()).unwrap_err(),
            RootRefusal::NotDirectory
        );
        // Not absolute, dot components, missing
        assert_eq!(
            canonical_grant_root("relative/dir").unwrap_err(),
            RootRefusal::NotAbsolute
        );
        assert_eq!(
            canonical_grant_root("/tmp/../etc").unwrap_err(),
            RootRefusal::NotAbsolute
        );
        assert_eq!(
            canonical_grant_root("").unwrap_err(),
            RootRefusal::NotAbsolute
        );
        assert_eq!(
            canonical_grant_root("/definitely/not/here/ocean-2c").unwrap_err(),
            RootRefusal::NotFound
        );
        // And a plain canonical directory is accepted as itself.
        let good = canon(&real);
        assert_eq!(canonical_grant_root(good.to_str().unwrap()).unwrap(), good);
    }

    #[test]
    fn confinement_is_on_the_canonical_result_not_the_string() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/file.txt"), b"ok").unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"no").unwrap();
        let root = canon(&root);

        assert_eq!(confine(&root, "").unwrap(), root);
        assert_eq!(confine(&root, "sub").unwrap(), root.join("sub"));
        assert_eq!(
            confine(&root, "sub/file.txt").unwrap(),
            root.join("sub/file.txt")
        );
        assert_eq!(
            confine(&root, "/etc/passwd").unwrap_err(),
            ConfineRefusal::NotRelative
        );
        assert_eq!(
            confine(&root, "../outside/secret").unwrap_err(),
            ConfineRefusal::NotRelative
        );
        assert_eq!(
            confine(&root, "sub/../../outside").unwrap_err(),
            ConfineRefusal::NotRelative
        );
        assert_eq!(
            confine(&root, "./sub").unwrap_err(),
            ConfineRefusal::NotRelative
        );
        assert_eq!(
            confine(&root, "missing.txt").unwrap_err(),
            ConfineRefusal::NotFound
        );
        assert_eq!(ConfineRefusal::NotRelative.code(), "invalid_relative_path");
        assert_eq!(ConfineRefusal::NotFound.code(), "path_not_found");
        assert_eq!(ConfineRefusal::Escapes.code(), "path_escapes_root");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            // The joined STRING is under the root; the resolved path is not.
            assert_eq!(
                confine(&root, "escape").unwrap_err(),
                ConfineRefusal::Escapes
            );
            assert_eq!(
                confine(&root, "escape/secret").unwrap_err(),
                ConfineRefusal::Escapes
            );
            // A symlink that stays inside is fine.
            std::os::unix::fs::symlink(root.join("sub"), root.join("alias")).unwrap();
            assert_eq!(
                confine(&root, "alias/file.txt").unwrap(),
                root.join("sub/file.txt")
            );
        }
    }

    #[test]
    fn the_projection_never_carries_the_root() {
        let grant = RoomResourceGrant {
            room_id: RoomKey::new("r"),
            resource_id: "res-1".into(),
            display_name: "source".into(),
            local_root: "/Users/private/secret-root".into(),
            resource_kind: "folder".into(),
            access_mode: ResourceAccessMode::Read,
            authorized_agent_member_ids: vec!["builder".into()],
            expires_at: None,
            generation: 3,
            status: ResourceStatus::Available,
            granted_by: "op".into(),
            granted_at: Utc::now(),
            decision_id: "d".into(),
            request_digest: "sha".into(),
            revoked_at: None,
            revoked_by: None,
        };
        let rendered = resource_projection(&grant, Utc::now()).to_string();
        assert!(!rendered.contains("secret-root"), "{rendered}");
        assert!(!rendered.contains("\"sha\""), "{rendered}");
        assert!(rendered.contains("\"generation\":\"3\""));
        let mut expired = grant.clone();
        expired.expires_at = Some(Utc::now() - chrono::Duration::seconds(5));
        assert_eq!(
            resource_projection(&expired, Utc::now())["status"],
            json!("revoked")
        );
    }
}
