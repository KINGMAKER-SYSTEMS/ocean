//! Rooms Phase 2 Stage 2d — resource-aware `room_list` and `room_read` tools.
//!
//! See `docs/specs/2026-09-08-ocean-rooms-phase2-room-profile-and-contributed-folders-manifest.md`
//! §4, §7, §8, and Gate 0 Decision 8 (budgets) and Decision 12 (generations).
//!
//! The shape is the one `room_history.rs` established: the daemon mints an
//! opaque, non-serializable handle from final admission evidence and a
//! daemon-owned authority; the tools carry that handle; the authority receives
//! the fixed scope on EVERY call and re-validates the binding generation and
//! the grant generation immediately before the operation (Decision 12). Model
//! arguments name a `resource_id` and a relative path and nothing else — they
//! can never select the room, the agent, the generation, or an absolute path.
//!
//! Confinement starts with the canonical RESULT ([`confine`]): the relative path is
//! lexically normal (no `..`, no `.`, not absolute), joined to the root the
//! authority returned, canonicalized, and refused unless the result is under
//! that root. A symlink inside the folder that points outside is an escape,
//! whatever its name says. Actual I/O then walks no-follow descriptors from the
//! captured, already-canonical granted root and retains those handles through
//! list/read. Path substitution cannot redirect an operation after resolution;
//! non-Unix platforms refuse filesystem operations.
//!
//! Budgets are Decision 8's ceilings, lowered where the prompt would not
//! survive them: one listing is at most 2,000 entries and never recursive;
//! one file read is refused above 8 MiB and returned in bounded chunks with a
//! `next_offset`; chunks require at least four bytes so every UTF-8 scalar
//! fits without exceeding the requested budget. Every operation has a
//! 30-second deadline. Content that exceeds a budget fails with a typed result
//! — it is never silently expanded
//! into the transcript.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ocean_runtime::{capability::SharedTool, AgentTool, AgentToolResult, Concurrency};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Decision 8: one directory listing, non-recursive.
pub const MAX_LIST_ENTRIES: usize = 2_000;
/// Decision 8: one file read.
pub const MAX_READ_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// A chunk small enough to sit in a prompt; the tool pages with `next_offset`.
pub const DEFAULT_READ_CHUNK_BYTES: usize = 64 * 1024;
/// Enough room for the largest UTF-8 scalar, ensuring forward progress.
pub const MIN_READ_CHUNK_BYTES: usize = 4;
pub const MAX_READ_CHUNK_BYTES: usize = 512 * 1024;
/// Decision 8: bounded file operation deadline.
pub const OPERATION_DEADLINE: Duration = Duration::from_secs(30);
const MAX_ENTRY_NAME_CHARS: usize = 255;

/// Admission evidence for the resource handle. Identical to the history
/// admission — the same binding admits both — so one implementation serves
/// both handles.
pub use crate::room_history::RoomHistoryAdmission as RoomResourceAdmission;

/// Immutable scope passed to the daemon-owned authority on every call.
#[derive(Clone, PartialEq, Eq)]
pub struct RoomResourceScope {
    room_key: String,
    agent_member_id: String,
    binding_generation: u64,
}

impl std::fmt::Debug for RoomResourceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomResourceScope")
            .field("room_key", &self.room_key)
            .field("agent_member_id", &self.agent_member_id)
            .field("binding_generation", &self.binding_generation)
            .finish()
    }
}

impl RoomResourceScope {
    pub fn room_key(&self) -> &str {
        &self.room_key
    }
    pub fn agent_member_id(&self) -> &str {
        &self.agent_member_id
    }
    pub fn binding_generation(&self) -> u64 {
        self.binding_generation
    }
}

/// What an operation needs. Mirrors the grant ladder without depending on the
/// store crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomResourceOp {
    List,
    Read,
}

impl RoomResourceOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read => "read",
        }
    }
}

/// What the authority hands back for one admitted operation. `local_root`
/// stays inside the tool and is never echoed in a result.
#[derive(Debug, Clone)]
pub struct ResolvedResource {
    pub local_root: PathBuf,
    pub grant_generation: u64,
}

/// Typed refusals the authority may return. Each maps to a fixed code so the
/// model sees a stable, content-free reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomResourceError {
    /// No grant with that id in this room.
    NotFound,
    /// The grant is suspended, revoked, or expired.
    NotAvailable,
    /// The grant does not authorize this agent.
    AgentNotAuthorized,
    /// The grant's access mode does not cover the operation.
    ModeNotGranted,
    /// The binding generation the handle was minted under is no longer
    /// current, or the grant generation changed under an in-flight call.
    StaleGeneration,
    /// The operation belongs to a later phase (`write`, `execute`).
    PhaseNotOpen,
    /// The authority could not answer.
    Unavailable(String),
}

impl RoomResourceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "resource_not_found",
            Self::NotAvailable => "resource_not_available",
            Self::AgentNotAuthorized => "agent_not_authorized_for_resource",
            Self::ModeNotGranted => "access_mode_not_granted",
            Self::StaleGeneration => "stale_generation",
            Self::PhaseNotOpen => "phase_not_open",
            Self::Unavailable(_) => "resource_authority_unavailable",
        }
    }
}

impl std::fmt::Display for RoomResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(detail) => write!(f, "{}: {detail}", self.code()),
            other => f.write_str(other.code()),
        }
    }
}

impl std::error::Error for RoomResourceError {}

/// One fixed-schema audit fact (manifest §8, Decision 13). Relative paths are
/// digested, never stored; no content, no root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomResourceAuditFact {
    pub resource_id: String,
    pub grant_generation: Option<u64>,
    pub op: RoomResourceOp,
    /// SHA-256 hex of the normalized relative path (`""` for the root).
    pub relative_path_digest: String,
    pub bytes: u64,
    pub entries: u64,
    /// `ok` | a [`RoomResourceError::code`] | a local refusal code.
    pub outcome: String,
}

/// Daemon-owned authority the tools consult on every call.
#[async_trait]
pub trait RoomResourceAuthority: Send + Sync {
    /// Re-validate the scope's binding and the grant, then hand back the root
    /// for exactly one operation.
    async fn resolve(
        &self,
        scope: &RoomResourceScope,
        resource_id: &str,
        op: RoomResourceOp,
    ) -> Result<ResolvedResource, RoomResourceError>;

    /// Record one audit fact. Failures are logged by the tool, never surfaced
    /// as content, and never block a result already computed.
    async fn record(&self, scope: &RoomResourceScope, fact: RoomResourceAuditFact);
}

/// One catalog line the tool description shows the model: the alias it will
/// recognise, the opaque id it must pass, and what it may do there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomResourceCatalogEntry {
    pub resource_id: String,
    pub display_name: String,
    pub access_mode: String,
}

/// Opaque, non-serializable authority for one admitted resource reader.
#[derive(Clone)]
pub struct AdmittedRoomResources {
    scope: RoomResourceScope,
    authority: Arc<dyn RoomResourceAuthority>,
    catalog: Arc<Vec<RoomResourceCatalogEntry>>,
}

impl std::fmt::Debug for AdmittedRoomResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedRoomResources")
            .field("scope", &self.scope)
            .field("catalog", &self.catalog.len())
            .finish_non_exhaustive()
    }
}

impl AdmittedRoomResources {
    pub(crate) fn from_admission(
        admission: &impl RoomResourceAdmission,
        authority: Arc<dyn RoomResourceAuthority>,
        catalog: Vec<RoomResourceCatalogEntry>,
    ) -> anyhow::Result<Self> {
        let room_key = admission.admitted_room_key();
        let agent_member_id = admission.admitted_agent_member_id();
        anyhow::ensure!(
            !room_key.is_empty(),
            "room resource admission has no Room key"
        );
        anyhow::ensure!(
            !agent_member_id.is_empty(),
            "room resource admission has no agent member"
        );
        anyhow::ensure!(
            admission.admitted_generation() > 0,
            "room resource admission has no authority generation"
        );
        Ok(Self {
            scope: RoomResourceScope {
                room_key: room_key.to_string(),
                agent_member_id: agent_member_id.to_string(),
                binding_generation: admission.admitted_generation(),
            },
            authority,
            catalog: Arc::new(catalog),
        })
    }

    pub fn scope(&self) -> &RoomResourceScope {
        &self.scope
    }

    pub fn catalog(&self) -> &[RoomResourceCatalogEntry] {
        &self.catalog
    }

    /// The two reserved tools. Names are reserved in the ambient toolset the
    /// same way `room_history` is.
    pub fn tools(&self) -> Vec<SharedTool> {
        let description_suffix = catalog_text(&self.catalog);
        vec![
            Arc::new(RoomListTool {
                authority: self.clone(),
                description: format!(
                    "List one directory inside a folder this Room has contributed to you. \
                     Non-recursive, at most {MAX_LIST_ENTRIES} entries, names only. \
                     The Room, agent, and authority are fixed by admission; you choose a \
                     resource_id from the catalog and a relative path.{description_suffix}"
                ),
            }) as SharedTool,
            Arc::new(RoomReadTool {
                authority: self.clone(),
                description: format!(
                    "Read a UTF-8 text file inside a folder this Room has contributed to you, \
                     in bounded chunks ({MIN_READ_CHUNK_BYTES}–{MAX_READ_CHUNK_BYTES} bytes; \
                     default {DEFAULT_READ_CHUNK_BYTES}; page with \
                     next_offset). Files over {MAX_READ_FILE_BYTES} bytes and binary files are \
                     refused with a typed reason. The Room, agent, and authority are fixed by \
                     admission.{description_suffix}"
                ),
            }) as SharedTool,
        ]
    }
}

pub const ROOM_LIST_TOOL: &str = "room_list";
pub const ROOM_READ_TOOL: &str = "room_read";

fn catalog_text(catalog: &[RoomResourceCatalogEntry]) -> String {
    if catalog.is_empty() {
        return " No folders are currently contributed.".to_string();
    }
    let mut out = String::from(" Contributed folders (resource_id: alias [access]):");
    for entry in catalog {
        out.push_str(&format!(
            " {}: {} [{}];",
            entry.resource_id, entry.display_name, entry.access_mode
        ));
    }
    out
}

// ── Confinement ───────────────────────────────────────────────────────────────

/// Why a relative path was refused under a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfineRefusal {
    /// Absolute, or contains `..`/`.`, or a component the OS treats as
    /// special. Refused lexically before touching the filesystem.
    NotRelative,
    NotFound,
    /// Canonicalized outside the root (a symlink escape).
    Escapes,
}

impl ConfineRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotRelative => "invalid_relative_path",
            Self::NotFound => "path_not_found",
            Self::Escapes => "path_escapes_root",
        }
    }
}

/// Resolve `relative` under `canonical_root` and prove the result stays
/// inside. `relative` may be empty (the root itself). The check is on the
/// canonicalized RESULT, after symlink resolution — a prefix check on the
/// joined string would accept `link-to-home/.ssh` when `link-to-home` points
/// out of the folder. This is a point-in-time projection, not I/O authority;
/// list/read operations separately capture and traverse confined handles.
pub fn confine(canonical_root: &Path, relative: &str) -> Result<PathBuf, ConfineRefusal> {
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Err(ConfineRefusal::NotRelative);
    }
    for component in rel.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(ConfineRefusal::NotRelative),
        }
    }
    let joined = canonical_root.join(rel);
    let resolved = match std::fs::canonicalize(&joined) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfineRefusal::NotFound)
        }
        Err(_) => return Err(ConfineRefusal::Escapes),
    };
    if resolved != canonical_root && !resolved.starts_with(canonical_root) {
        return Err(ConfineRefusal::Escapes);
    }
    Ok(resolved)
}

type IoFailure = (&'static str, Option<String>);

#[cfg(unix)]
#[derive(Clone, Copy)]
enum OpenKind {
    Directory,
    File,
}

/// Open the already-canonical grant root without following any component.
/// Never canonicalize a substituted root into a different grant authority.
#[cfg(unix)]
fn open_root_handle(root: &Path) -> Result<std::fs::File, ConfineRefusal> {
    use rustix::fs::{open, openat, Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    if !root.is_absolute() {
        return Err(ConfineRefusal::Escapes);
    }
    let mut handle = open("/", flags, Mode::empty()).map_err(confine_open_error)?;
    for part in root.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                handle = openat(&handle, name, flags, Mode::empty()).map_err(confine_open_error)?;
            }
            _ => return Err(ConfineRefusal::Escapes),
        }
    }
    Ok(handle.into())
}

#[cfg(unix)]
fn confine_open_error(error: rustix::io::Errno) -> ConfineRefusal {
    if error == rustix::io::Errno::NOENT {
        ConfineRefusal::NotFound
    } else {
        ConfineRefusal::Escapes
    }
}

/// Resolve display-compatible aliases, then perform the actual operation only
/// through the captured root descriptor and no-follow child descriptors. The
/// private hook lets tests deterministically replace paths at the old race.
#[cfg(unix)]
fn open_confined_with(
    root: &Path,
    relative: &str,
    kind: OpenKind,
    after_resolution: impl FnOnce(&Path),
) -> Result<std::fs::File, IoFailure> {
    use rustix::fs::{openat, statat, AtFlags, FileType, Mode, OFlags};
    use std::os::unix::fs::MetadataExt;

    let root_handle = open_root_handle(root).map_err(|error| (error.code(), None))?;
    let root_identity = root_handle
        .metadata()
        .map_err(|_| ("path_not_found", None))?;
    let resolved = confine(root, relative).map_err(|error| (error.code(), None))?;
    let child = resolved
        .strip_prefix(root)
        .map_err(|_| ("path_escapes_root", None))?;
    after_resolution(&resolved);
    // A root-name substitution is refused. Changes after this check cannot
    // redirect the captured descriptor even if a directory is renamed away.
    let current_root = open_root_handle(root).map_err(|error| (error.code(), None))?;
    let current_identity = current_root
        .metadata()
        .map_err(|_| ("path_not_found", None))?;
    if root_identity.dev() != current_identity.dev()
        || root_identity.ino() != current_identity.ino()
    {
        return Err(("path_escapes_root", None));
    }

    let parts: Vec<_> = child.components().collect();
    let mut handle = root_handle;
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = *part else {
            return Err(("invalid_relative_path", None));
        };
        let last = index + 1 == parts.len();
        let expected = if last && matches!(kind, OpenKind::File) {
            FileType::RegularFile
        } else {
            FileType::Directory
        };
        let stat = statat(&handle, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| (confine_open_error(error).code(), None))?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if file_type == FileType::Symlink {
            return Err(("path_escapes_root", None));
        }
        if file_type != expected {
            let code = if last && matches!(kind, OpenKind::File) {
                "not_a_file"
            } else {
                "not_a_directory"
            };
            return Err((code, None));
        }
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if expected == FileType::Directory {
            flags |= OFlags::DIRECTORY;
        } else {
            flags |= OFlags::NOCTTY;
        }
        handle = openat(&handle, name, flags, Mode::empty())
            .map_err(|error| (confine_open_error(error).code(), None))?
            .into();
        let metadata = handle.metadata().map_err(|_| ("path_not_found", None))?;
        if expected == FileType::Directory && !metadata.is_dir() {
            return Err(("not_a_directory", None));
        }
        if expected == FileType::RegularFile && !metadata.is_file() {
            return Err(("not_a_file", None));
        }
    }
    Ok(handle)
}

/// Lexical normalization for the audit digest: `a//b/` → `a/b`.
fn normalized_relative(relative: &str) -> String {
    Path::new(relative)
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_digest(relative: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_relative(relative).as_bytes());
    format!("{:x}", hasher.finalize())
}

// ── Tools ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    resource_id: String,
    #[serde(default)]
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    resource_id: String,
    path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    max_bytes: Option<Value>,
}

fn refusal(code: &str, detail: Option<String>) -> AgentToolResult {
    let mut body = json!({ "ok": false, "error": code });
    if let Some(detail) = detail {
        body["detail"] = json!(detail);
    }
    AgentToolResult::text(body.to_string())
}

/// Validate the id shape the daemon mints (`res-<hex>`), so a model cannot
/// turn the argument into a path or a query.
fn valid_resource_id(raw: &str) -> bool {
    raw.len() <= 128
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !raw.is_empty()
}

struct RoomListTool {
    authority: AdmittedRoomResources,
    description: String,
}

#[async_trait]
impl AgentTool for RoomListTool {
    fn name(&self) -> &str {
        ROOM_LIST_TOOL
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "resource_id": {
                    "type": "string",
                    "description": "A resource_id from the contributed-folder catalog"
                },
                "path": {
                    "type": "string",
                    "description": "Relative directory inside the folder; empty for its root"
                }
            },
            "required": ["resource_id"],
            "additionalProperties": false
        })
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Shared
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<AgentToolResult, String> {
        let args: ListArgs =
            serde_json::from_value(args).map_err(|_| "invalid room_list arguments".to_string())?;
        if !valid_resource_id(&args.resource_id) {
            return Ok(refusal("resource_not_found", None));
        }
        let scope = &self.authority.scope;
        let authority = &self.authority.authority;
        let digest = path_digest(&args.path);
        let mut fact = RoomResourceAuditFact {
            resource_id: args.resource_id.clone(),
            grant_generation: None,
            op: RoomResourceOp::List,
            relative_path_digest: digest,
            bytes: 0,
            entries: 0,
            outcome: String::new(),
        };
        let resolved = match authority
            .resolve(scope, &args.resource_id, RoomResourceOp::List)
            .await
        {
            Ok(resolved) => resolved,
            Err(error) => {
                fact.outcome = error.code().to_string();
                authority.record(scope, fact).await;
                return Ok(refusal(error.code(), None));
            }
        };
        fact.grant_generation = Some(resolved.grant_generation);
        let root = resolved.local_root.clone();
        let relative = args.path.clone();
        let listing = tokio::time::timeout(
            OPERATION_DEADLINE,
            tokio::task::spawn_blocking(move || list_dir(&root, &relative)),
        )
        .await;
        let outcome = match listing {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(("list_failed", None)),
            Err(_) => Err(("deadline_exceeded", None)),
        };
        match outcome {
            Ok((entries, truncated)) => {
                fact.entries = entries.len() as u64;
                fact.outcome = "ok".into();
                authority.record(scope, fact).await;
                Ok(AgentToolResult::text(
                    json!({
                        "ok": true,
                        "resource_id": args.resource_id,
                        "path": normalized_relative(&args.path),
                        "entries": entries,
                        "truncated": truncated,
                        "grant_generation": resolved.grant_generation.to_string(),
                    })
                    .to_string(),
                ))
            }
            Err((code, detail)) => {
                fact.outcome = code.to_string();
                authority.record(scope, fact).await;
                Ok(refusal(code, detail))
            }
        }
    }
}

type ListOutcome = Result<(Vec<Value>, bool), (&'static str, Option<String>)>;

#[cfg(unix)]
fn validated_entry_name(name: &std::ffi::CStr) -> Result<&str, IoFailure> {
    let name = name.to_str().map_err(|_| ("filename_not_utf8", None))?;
    if name.chars().count() > MAX_ENTRY_NAME_CHARS {
        return Err(("entry_name_too_long", None));
    }
    Ok(name)
}

#[cfg(unix)]
fn list_dir(root: &Path, relative: &str) -> ListOutcome {
    list_dir_with(root, relative, |_| {})
}

#[cfg(unix)]
fn list_dir_with(root: &Path, relative: &str, after_resolution: impl FnOnce(&Path)) -> ListOutcome {
    use rustix::fs::{statat, AtFlags, Dir, FileType};
    let handle = open_confined_with(root, relative, OpenKind::Directory, after_resolution)?;
    let read = Dir::read_from(&handle).map_err(|_| ("list_failed", None))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in read {
        let entry = entry.map_err(|_| ("list_failed", None))?;
        if matches!(entry.file_name().to_bytes(), b"." | b"..") {
            continue;
        }
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        let name = validated_entry_name(entry.file_name())?;
        // Metadata is relative to the same directory descriptor; never follow
        // links or reopen a path reconstructed from the directory's old name.
        let (kind, size) = match statat(&handle, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(meta) => match FileType::from_raw_mode(meta.st_mode) {
                FileType::Symlink => ("symlink", None),
                FileType::Directory => ("dir", None),
                FileType::RegularFile => ("file", Some(meta.st_size.max(0) as u64)),
                _ => ("other", None),
            },
            Err(_) => ("other", None),
        };
        entries.push(json!({ "name": name, "kind": kind, "size": size }));
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok((entries, truncated))
}

#[cfg(not(unix))]
fn list_dir(_root: &Path, _relative: &str) -> ListOutcome {
    Err(("resource_filesystem_unavailable", None))
}

struct RoomReadTool {
    authority: AdmittedRoomResources,
    description: String,
}

#[async_trait]
impl AgentTool for RoomReadTool {
    fn name(&self) -> &str {
        ROOM_READ_TOOL
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "resource_id": {
                    "type": "string",
                    "description": "A resource_id from the contributed-folder catalog"
                },
                "path": {
                    "type": "string",
                    "description": "Relative file path inside the folder"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Byte offset to start from (use next_offset to continue)"
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": MIN_READ_CHUNK_BYTES,
                    "maximum": MAX_READ_CHUNK_BYTES,
                    "description": "Maximum bytes to return; at least four to fit any UTF-8 scalar"
                }
            },
            "required": ["resource_id", "path"],
            "additionalProperties": false
        })
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Shared
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<AgentToolResult, String> {
        let args: ReadArgs =
            serde_json::from_value(args).map_err(|_| "invalid room_read arguments".to_string())?;
        if !valid_resource_id(&args.resource_id) {
            return Ok(refusal("resource_not_found", None));
        }
        let scope = &self.authority.scope;
        let authority = &self.authority.authority;
        let mut fact = RoomResourceAuditFact {
            resource_id: args.resource_id.clone(),
            grant_generation: None,
            op: RoomResourceOp::Read,
            relative_path_digest: path_digest(&args.path),
            bytes: 0,
            entries: 0,
            outcome: String::new(),
        };
        let max_bytes = match parse_chunk_budget(args.max_bytes.as_ref()) {
            Ok(max_bytes) => max_bytes,
            Err((code, detail)) => {
                fact.outcome = code.to_string();
                authority.record(scope, fact).await;
                return Ok(refusal(code, detail));
            }
        };
        let resolved = match authority
            .resolve(scope, &args.resource_id, RoomResourceOp::Read)
            .await
        {
            Ok(resolved) => resolved,
            Err(error) => {
                fact.outcome = error.code().to_string();
                authority.record(scope, fact).await;
                return Ok(refusal(error.code(), None));
            }
        };
        fact.grant_generation = Some(resolved.grant_generation);
        let root = resolved.local_root.clone();
        let relative = args.path.clone();
        let offset = args.offset.unwrap_or(0);
        let read = tokio::time::timeout(
            OPERATION_DEADLINE,
            tokio::task::spawn_blocking(move || read_chunk(&root, &relative, offset, max_bytes)),
        )
        .await;
        let outcome = match read {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(("read_failed", None)),
            Err(_) => Err(("deadline_exceeded", None)),
        };
        match outcome {
            Ok(chunk) => {
                fact.bytes = chunk.bytes.len() as u64;
                fact.outcome = "ok".into();
                authority.record(scope, fact).await;
                let text = String::from_utf8_lossy(&chunk.bytes).into_owned();
                Ok(AgentToolResult::text(
                    json!({
                        "ok": true,
                        "resource_id": args.resource_id,
                        "path": normalized_relative(&args.path),
                        "offset": offset.to_string(),
                        "bytes": chunk.bytes.len(),
                        "file_size": chunk.file_size.to_string(),
                        "next_offset": chunk.next_offset.map(|n| n.to_string()),
                        "content": text,
                        "grant_generation": resolved.grant_generation.to_string(),
                    })
                    .to_string(),
                ))
            }
            Err((code, detail)) => {
                fact.outcome = code.to_string();
                authority.record(scope, fact).await;
                Ok(refusal(code, detail))
            }
        }
    }
}

#[derive(Debug)]
struct ReadChunk {
    bytes: Vec<u8>,
    file_size: u64,
    next_offset: Option<u64>,
}

type ReadOutcome = Result<ReadChunk, (&'static str, Option<String>)>;

fn parse_chunk_budget(max_bytes: Option<&Value>) -> Result<usize, IoFailure> {
    let Some(max_bytes) = max_bytes else {
        return Ok(DEFAULT_READ_CHUNK_BYTES);
    };
    let max_bytes = max_bytes
        .as_u64()
        .filter(|bytes| (MIN_READ_CHUNK_BYTES as u64..=MAX_READ_CHUNK_BYTES as u64).contains(bytes))
        .ok_or(("invalid_chunk_budget", None))?;
    usize::try_from(max_bytes).map_err(|_| ("invalid_chunk_budget", None))
}

fn validate_chunk_budget(max_bytes: usize) -> Result<(), IoFailure> {
    if !(MIN_READ_CHUNK_BYTES..=MAX_READ_CHUNK_BYTES).contains(&max_bytes) {
        return Err(("invalid_chunk_budget", None));
    }
    Ok(())
}

#[cfg(unix)]
fn read_chunk(root: &Path, relative: &str, offset: u64, max_bytes: usize) -> ReadOutcome {
    read_chunk_with(root, relative, offset, max_bytes, |_| {})
}

#[cfg(unix)]
fn read_chunk_with(
    root: &Path,
    relative: &str,
    offset: u64,
    max_bytes: usize,
    after_resolution: impl FnOnce(&Path),
) -> ReadOutcome {
    use std::io::{Read as _, Seek as _, SeekFrom};
    validate_chunk_budget(max_bytes)?;
    if relative.trim().is_empty() {
        return Err(("invalid_relative_path", None));
    }
    let mut handle = open_confined_with(root, relative, OpenKind::File, after_resolution)?;
    let meta = handle.metadata().map_err(|_| ("path_not_found", None))?;
    if !meta.is_file() {
        return Err(("not_a_file", None));
    }
    let file_size = meta.len();
    if file_size > MAX_READ_FILE_BYTES {
        return Err((
            "file_too_large",
            Some(format!(
                "{file_size} bytes exceeds the {MAX_READ_FILE_BYTES} byte read budget"
            )),
        ));
    }
    if offset > file_size {
        return Err(("offset_out_of_range", None));
    }
    // Binary check on the FIRST chunk only: a file that starts as text is text.
    if offset == 0 {
        let mut probe = vec![0u8; 8_192.min(file_size as usize)];
        handle
            .read_exact(&mut probe)
            .map_err(|_| ("read_failed", None))?;
        if probe.contains(&0u8) {
            return Err(("binary_not_supported", None));
        }
        handle
            .seek(SeekFrom::Start(0))
            .map_err(|_| ("read_failed", None))?;
    } else {
        handle
            .seek(SeekFrom::Start(offset))
            .map_err(|_| ("read_failed", None))?;
    }
    let want = max_bytes.min((file_size - offset) as usize);
    let mut bytes = vec![0u8; want];
    handle
        .read_exact(&mut bytes)
        .map_err(|_| ("read_failed", None))?;
    if bytes.contains(&0) {
        return Err(("binary_not_supported", None));
    }
    // Only an incomplete trailing sequence may be deferred to the next
    // chunk. Invalid leading/interior bytes must never become lossy text.
    let end = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) if error.error_len().is_none() && offset + (bytes.len() as u64) < file_size => {
            error.valid_up_to()
        }
        Err(_) => return Err(("binary_not_supported", None)),
    };
    if end == 0 && !bytes.is_empty() {
        return Err(("binary_not_supported", None));
    }
    bytes.truncate(end);
    let next = offset + bytes.len() as u64;
    Ok(ReadChunk {
        bytes,
        file_size,
        next_offset: (next < file_size).then_some(next),
    })
}

#[cfg(not(unix))]
fn read_chunk(_root: &Path, _relative: &str, _offset: u64, _max_bytes: usize) -> ReadOutcome {
    Err(("resource_filesystem_unavailable", None))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct TestAdmission;
    impl RoomResourceAdmission for TestAdmission {
        fn admitted_room_key(&self) -> &str {
            "hq"
        }
        fn admitted_agent_member_id(&self) -> &str {
            "builder"
        }
        fn admitted_generation(&self) -> u64 {
            3
        }
    }

    struct FakeAuthority {
        root: PathBuf,
        grant_generation: u64,
        current_generation: Mutex<u64>,
        refuse: Option<RoomResourceError>,
        facts: Mutex<Vec<RoomResourceAuditFact>>,
        seen_scopes: Mutex<Vec<RoomResourceScope>>,
    }

    #[async_trait]
    impl RoomResourceAuthority for FakeAuthority {
        async fn resolve(
            &self,
            scope: &RoomResourceScope,
            resource_id: &str,
            _op: RoomResourceOp,
        ) -> Result<ResolvedResource, RoomResourceError> {
            self.seen_scopes.lock().unwrap().push(scope.clone());
            if scope.binding_generation() != *self.current_generation.lock().unwrap() {
                return Err(RoomResourceError::StaleGeneration);
            }
            if let Some(refuse) = &self.refuse {
                return Err(refuse.clone());
            }
            if resource_id != "res-1" {
                return Err(RoomResourceError::NotFound);
            }
            Ok(ResolvedResource {
                local_root: self.root.clone(),
                grant_generation: self.grant_generation,
            })
        }

        async fn record(&self, _scope: &RoomResourceScope, fact: RoomResourceAuditFact) {
            self.facts.lock().unwrap().push(fact);
        }
    }

    fn fixture(
        refuse: Option<RoomResourceError>,
    ) -> (tempfile::TempDir, Arc<FakeAuthority>, AdmittedRoomResources) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("hello.txt"), "hello, room\nline two\n").unwrap();
        std::fs::write(root.join("sub/bin.dat"), [0u8, 159, 146, 150]).unwrap();
        let authority = Arc::new(FakeAuthority {
            root: root.clone(),
            grant_generation: 7,
            current_generation: Mutex::new(3),
            refuse,
            facts: Mutex::new(Vec::new()),
            seen_scopes: Mutex::new(Vec::new()),
        });
        let admitted = AdmittedRoomResources::from_admission(
            &TestAdmission,
            authority.clone(),
            vec![RoomResourceCatalogEntry {
                resource_id: "res-1".into(),
                display_name: "source".into(),
                access_mode: "read".into(),
            }],
        )
        .unwrap();
        (tmp, authority, admitted)
    }

    async fn call(admitted: &AdmittedRoomResources, name: &str, args: Value) -> Value {
        let tool = admitted
            .tools()
            .into_iter()
            .find(|t| t.name() == name)
            .expect("tool");
        let result = tool.execute("call-1", args).await.unwrap();
        let text = match &result.content[0] {
            ocean_protocol::Content::Text { text } => text.clone(),
            other => panic!("unexpected content {other:?}"),
        };
        serde_json::from_str(&text).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_is_scoped_confined_non_recursive_and_audited() {
        let (_tmp, authority, admitted) = fixture(None);
        let out = call(&admitted, ROOM_LIST_TOOL, json!({"resource_id": "res-1"})).await;
        assert_eq!(out["ok"], json!(true), "{out}");
        let names: Vec<&str> = out["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["hello.txt", "sub"]);
        assert_eq!(out["entries"][1]["kind"], json!("dir"));
        assert_eq!(out["grant_generation"], json!("7"));
        assert!(
            !out.to_string().contains("/root"),
            "no root in output: {out}"
        );

        {
            let scopes = authority.seen_scopes.lock().unwrap();
            assert_eq!(scopes[0].room_key(), "hq");
            assert_eq!(scopes[0].agent_member_id(), "builder");
            assert_eq!(scopes[0].binding_generation(), 3);
        }

        for (path, code) in [
            ("../", "invalid_relative_path"),
            ("/etc", "invalid_relative_path"),
            ("nope", "path_not_found"),
            ("hello.txt", "not_a_directory"),
        ] {
            let out = call(
                &admitted,
                ROOM_LIST_TOOL,
                json!({"resource_id": "res-1", "path": path}),
            )
            .await;
            assert_eq!(out["ok"], json!(false), "{path}");
            assert_eq!(out["error"], json!(code), "{path}");
        }
        let out = call(&admitted, ROOM_LIST_TOOL, json!({"resource_id": "res-9"})).await;
        assert_eq!(out["error"], json!("resource_not_found"));
        let out = call(&admitted, ROOM_LIST_TOOL, json!({"resource_id": "../x"})).await;
        assert_eq!(out["error"], json!("resource_not_found"));

        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts[0].outcome, "ok");
        assert_eq!(facts[0].entries, 2);
        assert_eq!(facts[0].grant_generation, Some(7));
        assert_eq!(facts[0].op, RoomResourceOp::List);
        assert_eq!(facts[0].relative_path_digest, path_digest(""));
        assert!(facts.iter().any(|f| f.outcome == "invalid_relative_path"));
        assert!(facts.iter().any(|f| f.outcome == "resource_not_found"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_is_chunked_utf8_safe_and_refuses_binary_and_escapes() {
        let (_tmp, authority, admitted) = fixture(None);
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "hello.txt", "max_bytes": 6}),
        )
        .await;
        assert_eq!(out["ok"], json!(true), "{out}");
        assert_eq!(out["content"], json!("hello,"));
        assert_eq!(out["next_offset"], json!("6"));
        assert_eq!(out["file_size"], json!("21"));
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "hello.txt", "offset": 6}),
        )
        .await;
        assert_eq!(out["content"], json!(" room\nline two\n"));
        assert_eq!(out["next_offset"], serde_json::Value::Null);

        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "sub/bin.dat"}),
        )
        .await;
        assert_eq!(out["error"], json!("binary_not_supported"));
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "sub"}),
        )
        .await;
        assert_eq!(out["error"], json!("not_a_file"));
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "../etc/passwd"}),
        )
        .await;
        assert_eq!(out["error"], json!("invalid_relative_path"));
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "hello.txt", "offset": 99}),
        )
        .await;
        assert_eq!(out["error"], json!("offset_out_of_range"));

        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts[0].bytes, 6);
        assert_eq!(facts[0].op, RoomResourceOp::Read);
        assert_eq!(facts[0].relative_path_digest, path_digest("hello.txt"));
        assert!(facts.iter().any(|f| f.outcome == "binary_not_supported"));
    }

    #[cfg(unix)]
    #[test]
    fn path_replacement_after_resolution_cannot_escape_the_held_root() {
        for replacement in ["leaf", "parent", "root"] {
            let (tmp, authority, _admitted) = fixture(None);
            let root = &authority.root;
            std::fs::write(root.join("sub/inside.txt"), "inside").unwrap();
            let outside = std::fs::canonicalize(tmp.path()).unwrap().join("outside");
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("inside.txt"), "outside secret").unwrap();
            let result = read_chunk_with(root, "sub/inside.txt", 0, 1024, |_| {
                let (source, destination) = match replacement {
                    "leaf" => (root.join("sub/inside.txt"), outside.join("inside.txt")),
                    "parent" => (root.join("sub"), outside.clone()),
                    "root" => (root.clone(), outside.clone()),
                    _ => unreachable!(),
                };
                std::fs::rename(&source, source.with_extension("held")).unwrap();
                std::os::unix::fs::symlink(destination, source).unwrap();
            });
            assert_eq!(result.unwrap_err().0, "path_escapes_root", "{replacement}");
        }
        for replacement in ["parent", "root"] {
            let (tmp, authority, _admitted) = fixture(None);
            let root = &authority.root;
            let outside = std::fs::canonicalize(tmp.path()).unwrap().join("outside");
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("outside-secret-name"), "secret").unwrap();
            let result = list_dir_with(root, "sub", |_| {
                let source = if replacement == "root" {
                    root.clone()
                } else {
                    root.join("sub")
                };
                std::fs::rename(&source, source.with_extension("held")).unwrap();
                std::os::unix::fs::symlink(&outside, source).unwrap();
            });
            assert_eq!(result.unwrap_err().0, "path_escapes_root", "{replacement}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_substituted_grant_root_is_not_recanonicalized_into_authority() {
        let (tmp, authority, _admitted) = fixture(None);
        let outside = std::fs::canonicalize(tmp.path()).unwrap().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("hello.txt"), "outside secret").unwrap();
        std::fs::rename(&authority.root, authority.root.with_extension("held")).unwrap();
        std::os::unix::fs::symlink(outside, &authority.root).unwrap();
        assert_eq!(
            read_chunk(&authority.root, "hello.txt", 0, 1024)
                .unwrap_err()
                .0,
            "path_escapes_root"
        );
        assert_eq!(
            list_dir(&authority.root, "").unwrap_err().0,
            "path_escapes_root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn in_root_aliases_remain_readable_and_special_files_are_refused() {
        let (_tmp, authority, _admitted) = fixture(None);
        let root = &authority.root;
        std::os::unix::fs::symlink(root.join("hello.txt"), root.join("alias")).unwrap();
        assert_eq!(
            read_chunk(root, "alias", 0, 1024).unwrap().bytes,
            b"hello, room\nline two\n"
        );
        let _listener = std::os::unix::net::UnixListener::bind(root.join("socket")).unwrap();
        assert_eq!(
            read_chunk(root, "socket", 0, 1024).unwrap_err().0,
            "not_a_file"
        );
        #[cfg(target_os = "linux")]
        {
            let handle = open_root_handle(root).unwrap();
            rustix::fs::mkfifoat(
                &handle,
                "pipe",
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            assert_eq!(
                read_chunk(root, "pipe", 0, 1024).unwrap_err().0,
                "not_a_file"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_nul_invalid_utf8_is_refused_without_replacement_text() {
        let (_tmp, authority, admitted) = fixture(None);
        for bytes in [
            b"\xffabcdef".as_slice(),
            b"abc\xffdef".as_slice(),
            b"abc\xf0\x9f".as_slice(),
        ] {
            std::fs::write(authority.root.join("invalid.txt"), bytes).unwrap();
            let out = call(
                &admitted,
                ROOM_READ_TOOL,
                json!({"resource_id":"res-1","path":"invalid.txt"}),
            )
            .await;
            assert_eq!(out["error"], json!("binary_not_supported"));
            assert!(out.get("content").is_none());
        }
        std::fs::write(authority.root.join("unicode.txt"), "abcéZ").unwrap();
        let first = read_chunk(&authority.root, "unicode.txt", 0, MIN_READ_CHUNK_BYTES).unwrap();
        assert_eq!(first.bytes, b"abc");
        assert_eq!(first.next_offset, Some(3));
        let second = read_chunk(&authority.root, "unicode.txt", 3, MIN_READ_CHUNK_BYTES).unwrap();
        assert_eq!(std::str::from_utf8(&second.bytes).unwrap(), "éZ");
        assert_eq!(second.next_offset, None);
    }

    #[tokio::test]
    async fn unsupported_chunk_budgets_refuse_before_folder_resolution_or_access() {
        let (_tmp, authority, admitted) = fixture(None);
        std::fs::remove_dir_all(&authority.root).unwrap();
        let tool = admitted
            .tools()
            .into_iter()
            .find(|tool| tool.name() == ROOM_READ_TOOL)
            .unwrap();
        let schema = tool.parameters();
        assert_eq!(schema["properties"]["max_bytes"]["minimum"], json!(4));
        assert_eq!(
            schema["properties"]["max_bytes"]["maximum"],
            json!(MAX_READ_CHUNK_BYTES)
        );
        assert!(tool.description().contains("4–524288 bytes"));
        for budget in [0, 1, 2, 3, MAX_READ_CHUNK_BYTES + 1, usize::MAX] {
            let out = call(
                &admitted,
                ROOM_READ_TOOL,
                json!({"resource_id":"res-1","path":"absent.txt","max_bytes":budget}),
            )
            .await;
            assert_eq!(out["ok"], json!(false), "budget {budget}: {out}");
            assert_eq!(out["error"], json!("invalid_chunk_budget"));
            assert!(out.get("content").is_none());
            assert!(out.get("next_offset").is_none());
            #[cfg(unix)]
            assert_eq!(
                read_chunk_with(&authority.root, "absent.txt", 0, budget, |_| {
                    panic!("invalid budget reached filesystem hook")
                })
                .unwrap_err()
                .0,
                "invalid_chunk_budget"
            );
        }
        assert!(authority.seen_scopes.lock().unwrap().is_empty());
        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts.len(), 6);
        assert!(facts.iter().all(|fact| {
            fact.outcome == "invalid_chunk_budget"
                && fact.grant_generation.is_none()
                && fact.bytes == 0
                && fact.entries == 0
        }));
    }

    #[tokio::test]
    async fn unsupported_json_chunk_budgets_are_typed_audited_refusals() {
        let (_tmp, authority, admitted) = fixture(None);
        std::fs::remove_dir_all(&authority.root).unwrap();
        let budgets = [
            json!(-1_i64),
            json!(i64::MIN),
            json!(u64::MAX),
            json!(4.5),
            json!(-0.5),
            json!(1e100),
            json!("4"),
            json!(true),
            json!([4]),
            json!({"bytes": 4}),
        ];
        for (index, budget) in budgets.iter().enumerate() {
            let out = call(
                &admitted,
                ROOM_READ_TOOL,
                json!({"resource_id":"res-1","path":"absent.txt","max_bytes":budget}),
            )
            .await;
            assert_eq!(
                out,
                json!({"ok": false, "error": "invalid_chunk_budget"}),
                "budget {budget}"
            );
            assert!(authority.seen_scopes.lock().unwrap().is_empty());
            let facts = authority.facts.lock().unwrap();
            assert_eq!(facts.len(), index + 1, "budget {budget}");
            assert_eq!(
                facts[index],
                RoomResourceAuditFact {
                    resource_id: "res-1".into(),
                    grant_generation: None,
                    op: RoomResourceOp::Read,
                    relative_path_digest: path_digest("absent.txt"),
                    bytes: 0,
                    entries: 0,
                    outcome: "invalid_chunk_budget".into(),
                }
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_and_null_chunk_budgets_keep_defaults_and_integer_bounds() {
        let (_tmp, authority, admitted) = fixture(None);
        let file_bytes = DEFAULT_READ_CHUNK_BYTES + 8;
        std::fs::write(authority.root.join("default.txt"), "x".repeat(file_bytes)).unwrap();
        let cases = [
            (None, DEFAULT_READ_CHUNK_BYTES),
            (Some(Value::Null), DEFAULT_READ_CHUNK_BYTES),
            (
                Some(json!(DEFAULT_READ_CHUNK_BYTES)),
                DEFAULT_READ_CHUNK_BYTES,
            ),
            (Some(json!(MIN_READ_CHUNK_BYTES)), MIN_READ_CHUNK_BYTES),
            (Some(json!(MAX_READ_CHUNK_BYTES)), file_bytes),
        ];
        for (index, (budget, expected_bytes)) in cases.into_iter().enumerate() {
            let mut args = json!({"resource_id":"res-1","path":"default.txt"});
            if let Some(budget) = budget {
                args["max_bytes"] = budget;
            }
            let out = call(&admitted, ROOM_READ_TOOL, args).await;
            assert_eq!(out["ok"], json!(true));
            assert_eq!(out["bytes"], json!(expected_bytes));
            assert_eq!(out["content"], json!("x".repeat(expected_bytes)));
            assert_eq!(out["file_size"], json!(file_bytes.to_string()));
            assert_eq!(out["grant_generation"], json!("7"));
            assert_eq!(
                out["next_offset"],
                json!((expected_bytes < file_bytes).then(|| expected_bytes.to_string()))
            );
            assert_eq!(authority.seen_scopes.lock().unwrap().len(), index + 1);
            let facts = authority.facts.lock().unwrap();
            assert_eq!(facts.len(), index + 1);
            assert_eq!(facts[index].outcome, "ok");
            assert_eq!(facts[index].bytes, expected_bytes as u64);
            assert_eq!(facts[index].grant_generation, Some(7));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn leading_utf8_scalars_fit_the_minimum_chunk_at_eof_and_before_continuation() {
        let (_tmp, authority, admitted) = fixture(None);
        for scalar in ["é", "€", "🙂"] {
            for text in [scalar.to_string(), format!("{scalar}XYZ")] {
                std::fs::write(authority.root.join("unicode.txt"), &text).unwrap();
                for budget in 0..MIN_READ_CHUNK_BYTES {
                    let out = call(
                        &admitted,
                        ROOM_READ_TOOL,
                        json!({"resource_id":"res-1","path":"unicode.txt","max_bytes":budget}),
                    )
                    .await;
                    assert_eq!(
                        out["error"],
                        json!("invalid_chunk_budget"),
                        "{text:?}: {out}"
                    );
                    assert!(out.get("content").is_none());
                }
                let out = call(
                    &admitted,
                    ROOM_READ_TOOL,
                    json!({"resource_id":"res-1","path":"unicode.txt","max_bytes":4}),
                )
                .await;
                assert_eq!(out["ok"], json!(true), "{text:?}: {out}");
                let content = out["content"].as_str().unwrap();
                assert!(content.starts_with(scalar));
                assert!(text.starts_with(content));
                assert!(content.len() <= MIN_READ_CHUNK_BYTES);
                assert_eq!(out["bytes"], json!(content.len()));
                if content.len() < text.len() {
                    assert_eq!(out["next_offset"], json!(content.len().to_string()));
                } else {
                    assert!(out["next_offset"].is_null());
                }
                let eof = read_chunk(&authority.root, "unicode.txt", text.len() as u64, 4).unwrap();
                assert!(eof.bytes.is_empty());
                assert_eq!(eof.next_offset, None);
                assert_eq!(eof.file_size, text.len() as u64);
            }
        }
        std::fs::write(authority.root.join("empty.txt"), b"").unwrap();
        let empty = read_chunk(&authority.root, "empty.txt", 0, 4).unwrap();
        assert!(empty.bytes.is_empty());
        assert_eq!(empty.file_size, 0);
        assert_eq!(empty.next_offset, None);
    }

    #[cfg(unix)]
    #[test]
    fn mixed_utf8_chunks_are_bounded_and_every_continuation_advances() {
        let (_tmp, authority, _admitted) = fixture(None);
        let text = "é€🙂aZéé€🙂";
        std::fs::write(authority.root.join("unicode.txt"), text).unwrap();
        for budget in [4, 5, 6, 7, 8, MAX_READ_CHUNK_BYTES] {
            let mut offset = 0;
            let mut restored = Vec::new();
            loop {
                let chunk = read_chunk(&authority.root, "unicode.txt", offset, budget).unwrap();
                assert!(!chunk.bytes.is_empty());
                assert!(chunk.bytes.len() <= budget);
                assert!(std::str::from_utf8(&chunk.bytes).is_ok());
                let expected_next = offset + chunk.bytes.len() as u64;
                restored.extend_from_slice(&chunk.bytes);
                if let Some(next) = chunk.next_offset {
                    assert_eq!(next, expected_next);
                    assert!(next > offset && next < text.len() as u64);
                    offset = next;
                } else {
                    assert_eq!(expected_next, text.len() as u64);
                    break;
                }
            }
            assert_eq!(restored, text.as_bytes(), "budget {budget}");
        }
        // Every byte offset inside a scalar is invalid; only returned scalar
        // boundaries are supported continuations.
        for offset in 0..text.len() {
            if !text.is_char_boundary(offset) {
                assert_eq!(
                    read_chunk(&authority.root, "unicode.txt", offset as u64, 4)
                        .unwrap_err()
                        .0,
                    "binary_not_supported",
                    "offset {offset}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_utf8_at_scalar_boundaries_and_eof_never_becomes_content() {
        let (_tmp, authority, admitted) = fixture(None);
        for bytes in [
            b"\xc3".as_slice(),
            b"\xe2\x82".as_slice(),
            b"\xf0\x9f\x99".as_slice(),
            b"\xc3X".as_slice(),
            b"\xe2\x82X".as_slice(),
            b"\xf0\x9f\x99X".as_slice(),
            b"\xc0\xaf".as_slice(),
            b"\xed\xa0\x80".as_slice(),
            b"\xf4\x90\x80\x80".as_slice(),
        ] {
            for prefix in ["", "abcd"] {
                let mut file = prefix.as_bytes().to_vec();
                file.extend_from_slice(bytes);
                std::fs::write(authority.root.join("invalid.txt"), &file).unwrap();
                let out = call(
                    &admitted,
                    ROOM_READ_TOOL,
                    json!({"resource_id":"res-1","path":"invalid.txt",
                        "offset":prefix.len(),"max_bytes":4}),
                )
                .await;
                assert_eq!(
                    out["error"],
                    json!("binary_not_supported"),
                    "{file:?}: {out}"
                );
                assert!(out.get("content").is_none());
                assert!(out.get("next_offset").is_none());
            }
        }
        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts.len(), 18);
        assert!(facts
            .iter()
            .all(|fact| fact.outcome == "binary_not_supported" && fact.bytes == 0));
    }

    #[cfg(unix)]
    #[test]
    fn production_entry_name_validation_refuses_raw_invalid_bytes_on_every_unix() {
        use std::ffi::CString;

        for raw_name in [
            b"bad-\xff.txt".as_slice(),
            b"bad-\xfe.txt".as_slice(),
            b"bad-\xf0\x9f\x99.txt".as_slice(),
        ] {
            let name = CString::new(raw_name).unwrap();
            let failure = validated_entry_name(&name).unwrap_err();
            assert_eq!(failure, ("filename_not_utf8", None), "{raw_name:?}");
        }
        for text in ["bad-�.txt", "valid-é-€-🙂.txt"] {
            let name = CString::new(text).unwrap();
            assert_eq!(validated_entry_name(&name).unwrap(), text);
        }
        let exact = "a".repeat(MAX_ENTRY_NAME_CHARS);
        let name = CString::new(exact.as_str()).unwrap();
        assert_eq!(validated_entry_name(&name).unwrap(), exact);
        let overlong = CString::new("a".repeat(MAX_ENTRY_NAME_CHARS + 1)).unwrap();
        assert_eq!(
            validated_entry_name(&overlong).unwrap_err(),
            ("entry_name_too_long", None)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invalid_unix_names_refuse_the_listing_without_lossy_or_partial_projection() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let (_tmp, authority, admitted) = fixture(None);
        let directory = authority.root.join("sub");
        std::fs::write(directory.join("valid-é-€-🙂.txt"), b"unicode name").unwrap();
        std::fs::write(directory.join("bad-�.txt"), b"literal replacement scalar").unwrap();
        let mut created = Vec::new();
        let mut filesystem_refusals = 0;
        for raw_name in [b"bad-\xff.txt".as_slice(), b"bad-\xfe.txt".as_slice()] {
            let path = directory.join(OsString::from_vec(raw_name.to_vec()));
            match std::fs::write(&path, b"invalid filename") {
                Ok(()) => created.push(path),
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::ILSEQ.raw_os_error()) =>
                {
                    filesystem_refusals += 1;
                    continue;
                }
                Err(error) => panic!("unexpected synthetic filename creation failure: {error}"),
            }
            let out = call(
                &admitted,
                ROOM_LIST_TOOL,
                json!({"resource_id":"res-1","path":"sub"}),
            )
            .await;
            assert_eq!(out["ok"], json!(false));
            assert_eq!(out["error"], json!("filename_not_utf8"));
            assert!(out.get("entries").is_none());
            assert!(out.get("detail").is_none());
        }
        // Both raw byte names coexist and would previously project to the
        // same display name on supporting filesystems. APFS may instead
        // reject creation with EILSEQ; the production validator's raw-byte
        // test still runs there without depending on filesystem support.
        assert_eq!(created.len() + filesystem_refusals, 2);
        if !created.is_empty() {
            assert_eq!(
                list_dir(&authority.root, "sub").unwrap_err().0,
                "filename_not_utf8"
            );
        }
        if filesystem_refusals > 0 {
            eprintln!("invalid-name fixture: {filesystem_refusals} creations refused by filesystem with EILSEQ; raw-byte production validator exercised separately");
        }
        let rejected_listings = created.len();
        for path in created {
            std::fs::remove_file(path).unwrap();
        }
        let out = call(
            &admitted,
            ROOM_LIST_TOOL,
            json!({"resource_id":"res-1","path":"sub"}),
        )
        .await;
        assert_eq!(out["ok"], json!(true));
        let names: Vec<_> = out["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["bad-�.txt", "bin.dat", "valid-é-€-🙂.txt"]);
        let read = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id":"res-1","path":"sub/valid-é-€-🙂.txt"}),
        )
        .await;
        assert_eq!(read["content"], json!("unicode name"));
        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts.len(), rejected_listings + 2);
        assert!(facts[..rejected_listings].iter().all(|fact| {
            fact.outcome == "filename_not_utf8" && fact.entries == 0 && fact.bytes == 0
        }));
        assert_eq!(facts[rejected_listings].outcome, "ok");
        assert_eq!(facts[rejected_listings].entries, 3);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_same_resource_handle_refuses_after_authority_generation_changes() {
        let (_tmp, authority, admitted) = fixture(None);
        let first = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id":"res-1","path":"hello.txt"}),
        )
        .await;
        assert_eq!(first["ok"], json!(true));
        *authority.current_generation.lock().unwrap() = 4;
        std::fs::remove_dir_all(&authority.root).unwrap();
        for name in [ROOM_READ_TOOL, ROOM_LIST_TOOL] {
            let result = call(
                &admitted,
                name,
                json!({"resource_id":"res-1","path":"hello.txt"}),
            )
            .await;
            assert_eq!(result["error"], json!("stale_generation"));
            assert!(result.get("content").is_none());
            assert!(result.get("entries").is_none());
        }
        assert_eq!(authority.seen_scopes.lock().unwrap().len(), 3);
        assert!(authority
            .seen_scopes
            .lock()
            .unwrap()
            .iter()
            .all(|scope| scope.binding_generation() == 3));
    }

    #[cfg(not(unix))]
    #[test]
    fn unsupported_filesystems_refuse_without_pathname_fallback() {
        assert_eq!(
            list_dir(Path::new("nonexistent"), "").unwrap_err().0,
            "resource_filesystem_unavailable"
        );
        assert_eq!(
            read_chunk(Path::new("nonexistent"), "file", 0, 1024)
                .unwrap_err()
                .0,
            "resource_filesystem_unavailable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_escape_is_refused_on_the_resolved_path() {
        let (tmp, _authority, admitted) = fixture(None);
        let root = std::fs::canonicalize(tmp.path()).unwrap().join("root");
        let outside = std::fs::canonicalize(tmp.path()).unwrap().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "no").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let out = call(&admitted, ROOM_LIST_TOOL, json!({"resource_id": "res-1"})).await;
        let escape = out["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "escape")
            .unwrap();
        assert_eq!(
            escape["kind"],
            json!("symlink"),
            "listed as a link, never followed"
        );
        let out = call(
            &admitted,
            ROOM_LIST_TOOL,
            json!({"resource_id": "res-1", "path": "escape"}),
        )
        .await;
        assert_eq!(out["error"], json!("path_escapes_root"));
        let out = call(
            &admitted,
            ROOM_READ_TOOL,
            json!({"resource_id": "res-1", "path": "escape/secret.txt"}),
        )
        .await;
        assert_eq!(out["error"], json!("path_escapes_root"));
    }

    #[tokio::test]
    async fn an_authority_refusal_is_the_result_and_is_audited_before_any_io() {
        let (_tmp, authority, admitted) = fixture(Some(RoomResourceError::StaleGeneration));
        let out = call(&admitted, ROOM_LIST_TOOL, json!({"resource_id": "res-1"})).await;
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"], json!("stale_generation"));
        let facts = authority.facts.lock().unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].outcome, "stale_generation");
        assert_eq!(facts[0].grant_generation, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_listing_is_capped_at_the_decision_8_ceiling() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for i in 0..(MAX_LIST_ENTRIES + 5) {
            std::fs::write(root.join(format!("f{i:05}")), b"").unwrap();
        }
        let (entries, truncated) = list_dir(&root, "").unwrap();
        assert_eq!(entries.len(), MAX_LIST_ENTRIES);
        assert!(truncated);
    }

    #[test]
    fn the_catalog_is_in_the_description_and_from_admission_checks_evidence() {
        let (_tmp, _authority, admitted) = fixture(None);
        let tools = admitted.tools();
        assert_eq!(tools.len(), 2);
        assert!(tools[0].description().contains("res-1: source [read]"));
        assert_eq!(tools[0].name(), ROOM_LIST_TOOL);
        assert_eq!(tools[1].name(), ROOM_READ_TOOL);

        struct Bad;
        impl RoomResourceAdmission for Bad {
            fn admitted_room_key(&self) -> &str {
                "hq"
            }
            fn admitted_agent_member_id(&self) -> &str {
                "builder"
            }
            fn admitted_generation(&self) -> u64 {
                0
            }
        }
        let err = AdmittedRoomResources::from_admission(
            &Bad,
            Arc::new(FakeAuthority {
                root: PathBuf::from("/"),
                grant_generation: 1,
                current_generation: Mutex::new(3),
                refuse: None,
                facts: Mutex::new(Vec::new()),
                seen_scopes: Mutex::new(Vec::new()),
            }),
            vec![],
        )
        .unwrap_err();
        assert!(err.to_string().contains("generation"));
    }
}
