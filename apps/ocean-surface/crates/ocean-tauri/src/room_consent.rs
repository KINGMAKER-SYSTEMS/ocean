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
//! * Authorization needs an explicit native confirmation dialog. A declined,
//!   cancelled, expired or invalidated consent sends zero privileged requests.
//! * Authorize/reauthorize always carry the frozen non-null
//!   `expected_definition_digest`; a stale digest is reported as refused with
//!   no legacy retry.
//! * Requests go only to a fixed numeric-loopback daemon over a raw HTTP/1.1
//!   connection: no proxy routing, no redirects, no page-supplied method, path,
//!   header or body. Replies are size-bounded and must be JSON objects.
//! * The key is read through a no-follow descriptor immediately before each
//!   privileged request, must be an owner-only regular file owned by this
//!   user, and is never cached, returned, logged or embedded in an error.
//!
//! The pure broker below owns every check; the `#[tauri::command]` glue at the
//! bottom only adds the native dialog and the main-window gate (Tauri plugin
//! ACLs do not cover application commands).

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Map, Value};

const OPERATOR_HEADER: &str = "X-Ocean-Operator";
/// Pending consents held at once; the oldest idle one is evicted past this.
const MAX_PENDING: usize = 4;
/// A frozen preview older than this must be fetched again.
const CONSENT_TTL: Duration = Duration::from_secs(120);
const MAX_REPLY_BYTES: usize = 256 * 1024;
const MAX_ID_CHARS: usize = 512;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// Text for the native confirmation dialog: what, where, which digest.
    pub(crate) fn dialog_text(&self) -> (String, String, &'static str) {
        let action = match self.mode {
            ConsentMode::Authorize => "Authorize",
            ConsentMode::Reauthorize => "Reauthorize",
        };
        (
            self.display_name.clone(),
            format!("{}\n{}", self.room, self.digest),
            action,
        )
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
    Sending,
}

#[derive(Debug)]
struct Entry {
    id: String,
    frozen: Frozen,
    created: Instant,
    phase: Phase,
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
}

impl Intent {
    fn method(&self) -> &'static str {
        match self {
            Self::Preview { .. } => "GET",
            _ => "POST",
        }
    }

    fn privileged(&self) -> bool {
        !matches!(self, Self::Preview { .. })
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
        }
    }

    fn body(&self) -> Option<String> {
        match self {
            Self::Preview { .. } => None,
            Self::Authorize { body, .. } | Self::Reauthorize { body, .. } => Some(body.to_string()),
            Self::Revoke { decision_id, .. } => {
                Some(json!({ "decision_id": decision_id }).to_string())
            }
        }
    }
}

/// Percent-encode one path segment: only RFC 3986 unreserved bytes pass, and
/// dot segments are fully encoded.
fn encode_segment(raw: &str) -> String {
    if raw == "." || raw == ".." {
        return raw.replace('.', "%2E");
    }
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// ── endpoint and credential custody ─────────────────────────────────────

/// Accept only `http://<numeric loopback>[:port][/]`. Hostnames (including
/// `localhost`), userinfo, non-loopback addresses and TLS schemes are refused
/// so the credential cannot be resolved or routed anywhere else.
pub(crate) fn loopback_endpoint(url: &str) -> Option<SocketAddr> {
    let rest = url.trim().strip_prefix("http://")?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', ' ']) {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, tail) = v6.split_once(']')?;
        let port = match tail {
            "" => 4780,
            tail => tail.strip_prefix(':')?.parse().ok()?,
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, 4780),
        }
    };
    let ip: IpAddr = host.parse().ok()?;
    (ip.is_loopback() && port != 0).then_some(SocketAddr::new(ip, port))
}

fn ocean_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("OCEAN_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("ocean-rs");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("ocean-rs");
    }
    PathBuf::from(".ocean-rs")
}

/// `OCEAN_OPERATOR_KEY_FILE`, else `<config dir>/operator.key` — the path the
/// daemon writes.
pub(crate) fn operator_key_path() -> PathBuf {
    std::env::var_os("OCEAN_OPERATOR_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ocean_config_dir().join("operator.key"))
}

const CREDENTIAL_UNAVAILABLE: &str = "operator_credential_unavailable";

/// Read the operator key through a no-follow descriptor. The opened file must
/// be the inspected regular file, owned by this user, with no group/other
/// permission bits, and hold one header-safe token. Errors carry only a code.
#[cfg(unix)]
fn read_operator_key(path: &Path) -> Result<String, &'static str> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let link = std::fs::symlink_metadata(path).map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    if link.file_type().is_symlink() || !link.is_file() {
        return Err("operator_credential_not_regular");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    let opened = file.metadata().map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    if !opened.is_file() || opened.dev() != link.dev() || opened.ino() != link.ino() {
        return Err("operator_credential_changed");
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if opened.uid() != unsafe { libc::geteuid() } {
        return Err("operator_credential_foreign_owner");
    }
    if opened.mode() & 0o077 != 0 {
        return Err("operator_credential_not_private");
    }
    if opened.len() > 4096 {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    let mut raw = String::new();
    (&mut file)
        .take(4097)
        .read_to_string(&mut raw)
        .map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    let key = raw.trim();
    let header_safe = (16..=512).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'+' | b'/'));
    if !header_safe {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    Ok(key.to_string())
}

#[cfg(not(unix))]
fn read_operator_key(_path: &Path) -> Result<String, &'static str> {
    Err(CREDENTIAL_UNAVAILABLE)
}

// ── transport ───────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum SendError {
    /// Nothing reached the daemon (validation, credential, connect).
    NotSent(&'static str),
    /// The request may have reached the daemon; the reply was lost.
    Unknown,
    /// A reply arrived but is not acceptable (redirect, oversize, non-JSON).
    BadReply(&'static str),
}

struct Reply {
    status: u16,
    body: Map<String, Value>,
}

fn send(endpoint: SocketAddr, key_path: &Path, intent: &Intent) -> Result<Reply, SendError> {
    let key = if intent.privileged() {
        Some(read_operator_key(key_path).map_err(SendError::NotSent)?)
    } else {
        None
    };
    let body = intent.body().unwrap_or_default();
    let mut stream = TcpStream::connect_timeout(&endpoint, CONNECT_TIMEOUT)
        .map_err(|_| SendError::NotSent("daemon_unreachable"))?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
        intent.method(),
        intent.path(),
        endpoint
    );
    if intent.body().is_some() {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    if let Some(key) = key.as_deref() {
        head.push_str(&format!("{OPERATOR_HEADER}: {key}\r\n"));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .map_err(|_| SendError::Unknown)?;
    drop(head);
    let mut raw = Vec::new();
    stream
        .take((MAX_REPLY_BYTES + 16 * 1024 + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| SendError::Unknown)?;
    if raw.is_empty() {
        return Err(SendError::Unknown);
    }
    parse_reply(&raw)
}

fn parse_reply(raw: &[u8]) -> Result<Reply, SendError> {
    const BAD: SendError = SendError::BadReply("invalid_reply");
    if raw.len() > MAX_REPLY_BYTES + 16 * 1024 {
        return Err(SendError::BadReply("reply_too_large"));
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or(BAD)?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| BAD)?;
    let mut payload = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .filter(|line| line.starts_with("HTTP/1."))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or(BAD)?;
    if (300..400).contains(&status) {
        return Err(SendError::BadReply("redirect_refused"));
    }
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(BAD);
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(BAD);
            }
            length = Some(value.parse::<usize>().map_err(|_| BAD)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
            if !chunked {
                return Err(BAD);
            }
        }
    }
    let body = if chunked {
        let mut out = Vec::new();
        loop {
            let end = payload.windows(2).position(|w| w == b"\r\n").ok_or(BAD)?;
            let size_text = std::str::from_utf8(&payload[..end]).map_err(|_| BAD)?;
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| BAD)?;
            payload = &payload[end + 2..];
            if size == 0 {
                break;
            }
            if payload.len() < size + 2 || out.len() + size > MAX_REPLY_BYTES {
                return Err(BAD);
            }
            out.extend_from_slice(&payload[..size]);
            payload = &payload[size + 2..];
        }
        out
    } else if let Some(length) = length {
        if length > MAX_REPLY_BYTES || payload.len() < length {
            return Err(BAD);
        }
        payload[..length].to_vec()
    } else {
        if payload.len() > MAX_REPLY_BYTES {
            return Err(SendError::BadReply("reply_too_large"));
        }
        payload.to_vec()
    };
    match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(body)) => Ok(Reply { status, body }),
        _ => Err(BAD),
    }
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

fn bounded_id(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (!text.trim().is_empty() && text.chars().count() <= MAX_ID_CHARS && text.trim() == text)
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
                    .filter(|s| !s.is_empty() && s.len() <= 128)
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

fn refused(code: &'static str) -> ConsentOutcome {
    ConsentOutcome {
        state: OutcomeState::Refused,
        status: None,
        code: Some(code.into()),
        binding: None,
    }
}

fn fresh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── broker ──────────────────────────────────────────────────────────────

pub(crate) struct ConsentBroker {
    endpoint: Option<SocketAddr>,
    key_path: PathBuf,
    ttl: Duration,
    pending: Mutex<Vec<Entry>>,
}

impl ConsentBroker {
    pub(crate) fn new(daemon_url: &str, key_path: PathBuf) -> Self {
        Self {
            endpoint: loopback_endpoint(daemon_url),
            key_path,
            ttl: CONSENT_TTL,
            pending: Mutex::new(Vec::new()),
        }
    }

    fn endpoint(&self) -> Result<SocketAddr, &'static str> {
        self.endpoint.ok_or("consent_unavailable")
    }

    fn prune(&self, entries: &mut Vec<Entry>) {
        let ttl = self.ttl;
        entries.retain(|e| e.phase != Phase::Ready || e.created.elapsed() < ttl);
    }

    /// Fetch and freeze the authoritative preview for `package_id` in `room`.
    pub(crate) fn preview(&self, room: &str, package_id: &str) -> Result<ConsentPreview, String> {
        let endpoint = self.endpoint()?;
        if !valid_key(room) || !valid_key(package_id) {
            return Err("invalid_request".into());
        }
        let intent = Intent::Preview {
            room: room.to_string(),
            package_id: package_id.to_string(),
        };
        let reply = match send(endpoint, &self.key_path, &intent) {
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
        let id = fresh_id();
        let mut entries = self.pending.lock();
        self.prune(&mut entries);
        // One live consent per room/agent: a fresh preview supersedes an idle one.
        entries.retain(|e| {
            e.phase != Phase::Ready
                || e.frozen.room != frozen.room
                || e.frozen.agent_member_id != frozen.agent_member_id
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
            frozen,
            created: Instant::now(),
            phase: Phase::Ready,
        });
        Ok(preview)
    }

    /// Claim a ready consent for native confirmation.
    pub(crate) fn begin_confirm(&self, consent_id: &str) -> Result<Frozen, String> {
        let mut entries = self.pending.lock();
        self.prune(&mut entries);
        let entry = entries
            .iter_mut()
            .find(|e| e.id == consent_id)
            .ok_or("consent_expired")?;
        if entry.phase != Phase::Ready {
            return Err("consent_in_progress".into());
        }
        entry.phase = Phase::Confirming;
        Ok(entry.frozen.clone())
    }

    /// Settle the native confirmation. Only a still-claimed, confirmed consent
    /// proceeds; a declined one is discarded. Either way an invalidation that
    /// raced the dialog wins and nothing is sent.
    pub(crate) fn finish_confirm(
        &self,
        consent_id: &str,
        confirmed: bool,
    ) -> Result<Option<Frozen>, String> {
        let mut entries = self.pending.lock();
        let Some(index) = entries
            .iter()
            .position(|e| e.id == consent_id && e.phase == Phase::Confirming)
        else {
            return Err("consent_invalidated".into());
        };
        if !confirmed {
            entries.remove(index);
            return Ok(None);
        }
        entries[index].phase = Phase::Sending;
        Ok(Some(entries[index].frozen.clone()))
    }

    /// Send the frozen decision. A definitive reply consumes the consent; a
    /// lost acknowledgement returns it to `Ready` with the same decision id so
    /// only an explicit, re-confirmed retry can replay it.
    pub(crate) fn send_decision(&self, consent_id: &str, frozen: &Frozen) -> ConsentOutcome {
        let endpoint = match self.endpoint() {
            Ok(endpoint) => endpoint,
            Err(code) => {
                self.release(consent_id, true);
                return refused(code);
            }
        };
        match send(endpoint, &self.key_path, &frozen.intent()) {
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
        let mut entries = self.pending.lock();
        if let Some(index) = entries.iter().position(|e| e.id == consent_id) {
            if keep {
                entries[index].phase = Phase::Ready;
            } else {
                entries.remove(index);
            }
        }
    }

    /// Drop one consent, or every consent when `consent_id` is `None`
    /// (navigation, room change). A request already sending keeps its late
    /// outcome but is not returned to `Ready`.
    pub(crate) fn cancel(&self, consent_id: Option<&str>) {
        let mut entries = self.pending.lock();
        match consent_id {
            Some(id) => entries.retain(|e| e.id != id),
            None => entries.clear(),
        }
    }

    /// Revoke an own agent's binding with a fresh decision id. Call only after
    /// native confirmation.
    pub(crate) fn revoke(&self, room: &str, agent_member_id: &str) -> ConsentOutcome {
        let endpoint = match self.endpoint() {
            Ok(endpoint) => endpoint,
            Err(code) => return refused(code),
        };
        if !valid_key(room)
            || bounded_id(Some(&Value::String(agent_member_id.to_string()))).is_none()
        {
            return refused("invalid_request");
        }
        let intent = Intent::Revoke {
            room: room.to_string(),
            agent: agent_member_id.to_string(),
            decision_id: fresh_id(),
        };
        // Revocation is idempotent at the daemon; a dropped consent leaves no
        // native state, so a pending one for this agent is dropped first.
        self.pending
            .lock()
            .retain(|e| !(e.frozen.room == room && e.frozen.agent_member_id == agent_member_id));
        match send(endpoint, &self.key_path, &intent) {
            Ok(reply) => outcome_from_reply(reply),
            Err(SendError::NotSent(code)) => refused(code),
            Err(SendError::BadReply(code)) => ConsentOutcome {
                state: OutcomeState::Unknown,
                status: None,
                code: Some(code.into()),
                binding: None,
            },
            Err(SendError::Unknown) => ConsentOutcome {
                state: OutcomeState::Unknown,
                status: None,
                code: None,
                binding: None,
            },
        }
    }
}

// ── Tauri glue ──────────────────────────────────────────────────────────

pub(crate) struct ConsentState(pub(crate) Arc<ConsentBroker>);

fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    (window.label() == "main")
        .then_some(())
        .ok_or_else(|| "consent_window_refused".to_string())
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
    blocking(move || broker.preview(room.trim(), package_id.trim())).await?
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
    let Some(frozen) = broker.finish_confirm(&consent_id, confirmed)? else {
        return Ok(ConsentOutcome::declined());
    };
    blocking(move || broker.send_decision(&consent_id, &frozen)).await
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
    if !valid_key(&room) || agent.is_empty() || agent.chars().count() > MAX_ID_CHARS {
        return Err("invalid_request".into());
    }
    let title: String = display_name.chars().take(128).collect();
    let title = if title.trim().is_empty() {
        agent.clone()
    } else {
        title
    };
    if !native_confirm(&app, title, room.clone(), "Revoke", true).await {
        return Ok(ConsentOutcome::declined());
    }
    let broker = state.0.clone();
    blocking(move || broker.revoke(&room, &agent)).await
}

// ── tests ───────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    const KEY: &str = "fixtureOperatorKey_0123456789abcdefABCDEF-xyz";
    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn key_file(dir: &Path, mode: u32) -> PathBuf {
        let path = dir.join("operator.key");
        std::fs::write(&path, format!("{KEY}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    }

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

    /// Loopback "daemon" answering each accepted connection with the next
    /// canned reply (`None` closes without replying). Returns raw requests.
    fn fake_daemon(replies: Vec<Option<String>>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .map(|v| v.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if raw.len() >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                if let Some(reply) = reply {
                    socket.write_all(reply.as_bytes()).unwrap();
                }
                seen.push(String::from_utf8(raw).unwrap());
            }
            seen
        });
        (port, handle)
    }

    /// A bound port that records whether anything ever connected.
    fn silent_daemon() -> (u16, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        (listener.local_addr().unwrap().port(), listener)
    }

    fn assert_never_contacted(listener: &TcpListener) {
        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("daemon was contacted: {other:?}"),
        }
    }

    fn broker(port: u16, key: &Path) -> ConsentBroker {
        ConsentBroker::new(&format!("http://127.0.0.1:{port}"), key.to_path_buf())
    }

    fn json_body(request: &str) -> Value {
        serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap()
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
        let broker = ConsentBroker::new("http://192.0.2.1:4780", PathBuf::from("/nonexistent"));
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
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert_eq!(
            read_operator_key(&link).unwrap_err(),
            "operator_credential_not_regular"
        );
        assert_eq!(
            read_operator_key(dir.path()).unwrap_err(),
            "operator_credential_not_regular"
        );
        let injected = dir.path().join("inject.key");
        std::fs::write(&injected, "abcdefghijklmnopqrstuvwx\r\nOrigin: x").unwrap();
        std::fs::set_permissions(&injected, std::fs::Permissions::from_mode(0o600)).unwrap();
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
        assert_eq!(broker.pending.lock().len(), 1);
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
            assert!(broker.pending.lock().is_empty(), "{code}");
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
        let outcome = broker.send_decision(&preview.consent_id, &frozen2);
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
        let frozen = broker
            .finish_confirm(&preview.consent_id, true)
            .unwrap()
            .unwrap();
        let outcome = broker.send_decision(&preview.consent_id, &frozen);
        assert_eq!(outcome.state, OutcomeState::Refused);
        assert_eq!(outcome.status, Some(409));
        assert_eq!(outcome.code.as_deref(), Some("definition_digest_mismatch"));
        assert_eq!(
            daemon.join().unwrap().len(),
            2,
            "exactly one decision request"
        );
        assert!(
            broker.pending.lock().is_empty(),
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
        let outcome = broker.send_decision(&preview.consent_id, &frozen);
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
        let entries = broker.pending.lock();
        assert_eq!(entries.len(), MAX_PENDING);
        assert!(entries.iter().all(|e| e.id != ids[0]), "oldest evicted");
        let decisions: std::collections::HashSet<_> = entries
            .iter()
            .map(|e| e.frozen.decision_id.clone())
            .collect();
        assert_eq!(decisions.len(), MAX_PENDING);
        drop(entries);
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
}
