//! Room owner mutations from the desktop shell (operator decision 2026-10-08).
//!
//! The daemon gates three owner mutations on its header-only Room operator
//! credential (`X-Ocean-Operator`): renaming the owner (`PUT /v1/me`),
//! deciding a room run's pending permission
//! (`POST /v1/rooms/persistent/{key}/runs/{run_id}/permission`) and saving a
//! room agent's settings (`PUT /v1/rooms/persistent/{key}/agents/{agent_id}/settings`).
//! The operator decided that any first-party Ocean surface the operator is
//! signed into may make exactly these three. The desktop shell runs as the
//! operator on the daemon's machine, so it reads the daemon's mode-0600
//! `operator.key` (the same path the Surface proxy reads) and attaches it
//! here, on the Rust side:
//!
//! * only these three routes, chosen by a fixed `kind`, never a page-supplied
//!   method or path;
//! * only to a loopback daemon, so the key never leaves the machine;
//! * the key is read just before each request, never cached, never returned
//!   to the page, never logged, and never part of an error string;
//! * page code cannot add headers: the request carries exactly `Host`,
//!   `Content-Type`, `Content-Length`, `X-Ocean-Operator` and
//!   `Connection: close` (no `Origin`, no `Cookie`).
//!
//! Every other operator-gated route stays unreachable from the page.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

/// Header the daemon reads the Room operator credential from.
const OPERATOR_HEADER: &str = "X-Ocean-Operator";
/// Largest request body the page may hand the shell (settings are ≤4000
/// chars of instructions plus a model alias; decisions are tiny).
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Largest daemon reply relayed back to the page.
const MAX_REPLY_BYTES: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(15);

/// One of the three owner mutations. Nothing else can be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerMutation {
    RenameOwner,
    RunPermission { room: String, run_id: String },
    AgentSettings { room: String, agent_id: String },
}

impl OwnerMutation {
    /// Build from the page's invoke arguments. `kind` is a fixed vocabulary;
    /// ids must be non-empty and are percent-encoded into the path.
    pub(crate) fn parse(
        kind: &str,
        room: Option<&str>,
        target: Option<&str>,
    ) -> Result<Self, String> {
        let id = |value: Option<&str>, what: &str| -> Result<String, String> {
            match value {
                Some(v) if !v.is_empty() && v.len() <= 512 => Ok(v.to_string()),
                _ => Err(format!("{what} is required")),
            }
        };
        match kind {
            "rename_owner" if room.is_none() && target.is_none() => Ok(Self::RenameOwner),
            "run_permission" => Ok(Self::RunPermission {
                room: id(room, "room")?,
                run_id: id(target, "run id")?,
            }),
            "agent_settings" => Ok(Self::AgentSettings {
                room: id(room, "room")?,
                agent_id: id(target, "agent id")?,
            }),
            _ => Err("not an owner mutation".to_string()),
        }
    }

    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::RenameOwner | Self::AgentSettings { .. } => "PUT",
            Self::RunPermission { .. } => "POST",
        }
    }

    pub(crate) fn path(&self) -> String {
        match self {
            Self::RenameOwner => "/v1/me".to_string(),
            Self::RunPermission { room, run_id } => format!(
                "/v1/rooms/persistent/{}/runs/{}/permission",
                encode_segment(room),
                encode_segment(run_id)
            ),
            Self::AgentSettings { room, agent_id } => format!(
                "/v1/rooms/persistent/{}/agents/{}/settings",
                encode_segment(room),
                encode_segment(agent_id)
            ),
        }
    }
}

/// The daemon's reply, relayed to the page as-is (status + body text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OwnerMutationReply {
    pub(crate) status: u16,
    pub(crate) body: String,
}

/// Percent-encode one path segment: only RFC 3986 unreserved bytes pass, and
/// `.`/`..` are fully encoded.
fn encode_segment(raw: &str) -> String {
    // A dot segment would be a path traversal step, never an id.
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

/// The daemon's runtime config dir, resolved exactly as the daemon and the
/// Surface proxy resolve it.
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

/// `OCEAN_OPERATOR_KEY_FILE`, else `<config dir>/operator.key` — the same
/// path the Surface proxy reads.
pub(crate) fn operator_key_path() -> PathBuf {
    std::env::var_os("OCEAN_OPERATOR_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ocean_config_dir().join("operator.key"))
}

/// Read the operator key: a regular, non-symlink, mode-0600 file whose opened
/// descriptor is the file that was inspected. Errors never carry the key.
#[cfg(unix)]
pub(crate) fn read_operator_key(path: &Path) -> Result<String, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    const UNAVAILABLE: &str = "operator credential unavailable";
    let link = std::fs::symlink_metadata(path).map_err(|_| UNAVAILABLE.to_string())?;
    if link.file_type().is_symlink() || !link.is_file() {
        return Err("operator credential must be a regular file".into());
    }
    let mut file = std::fs::File::open(path).map_err(|_| UNAVAILABLE.to_string())?;
    let opened = file.metadata().map_err(|_| UNAVAILABLE.to_string())?;
    if opened.dev() != link.dev() || opened.ino() != link.ino() || !opened.is_file() {
        return Err("operator credential changed while it was read".into());
    }
    if opened.permissions().mode() & 0o777 != 0o600 {
        return Err("operator credential must be a mode-0600 file".into());
    }
    let mut key = String::new();
    file.read_to_string(&mut key)
        .map_err(|_| UNAVAILABLE.to_string())?;
    let key = key.trim();
    if key.is_empty() || key.contains(['\r', '\n']) {
        return Err(UNAVAILABLE.into());
    }
    Ok(key.to_string())
}

#[cfg(not(unix))]
pub(crate) fn read_operator_key(_path: &Path) -> Result<String, String> {
    Err("operator credential unavailable on this platform".into())
}

/// The key goes only to the daemon on this machine.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Send one owner mutation to the loopback daemon at `host:port` with the
/// operator key from `key_path`. Blocking; call off the async runtime.
pub(crate) fn send(
    host: &str,
    port: u16,
    key_path: &Path,
    mutation: &OwnerMutation,
    body: &str,
) -> Result<OwnerMutationReply, String> {
    if !is_loopback_host(host) {
        return Err("the operator credential is only sent to a loopback daemon".into());
    }
    if body.len() > MAX_BODY_BYTES {
        return Err("request body too large".into());
    }
    let key = read_operator_key(key_path)?;
    let addr = (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .ok_or_else(|| "Ocean could not reach the daemon".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|_| "Ocean could not reach the daemon".to_string())?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n{OPERATOR_HEADER}: {key}\r\nConnection: close\r\n\r\n",
        method = mutation.method(),
        path = mutation.path(),
        len = body.len(),
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .map_err(|_| "Ocean could not reach the daemon".to_string())?;
    let mut raw = Vec::new();
    stream
        .take((MAX_REPLY_BYTES + 16 * 1024) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| "the daemon reply could not be read".to_string())?;
    parse_reply(&raw)
}

/// Parse an HTTP/1.1 reply (Content-Length, chunked, or close-delimited).
fn parse_reply(raw: &[u8]) -> Result<OwnerMutationReply, String> {
    const BAD: &str = "the daemon sent an invalid reply";
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| BAD.to_string())?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| BAD.to_string())?;
    let mut payload = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| BAD.to_string())?;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.parse::<usize>().map_err(|_| BAD.to_string())?);
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    let body = if chunked {
        let mut out = Vec::new();
        loop {
            let line_end = payload
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or_else(|| BAD.to_string())?;
            let size_text =
                std::str::from_utf8(&payload[..line_end]).map_err(|_| BAD.to_string())?;
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| BAD.to_string())?;
            payload = &payload[line_end + 2..];
            if size == 0 {
                break;
            }
            if payload.len() < size + 2 || out.len() + size > MAX_REPLY_BYTES {
                return Err(BAD.into());
            }
            out.extend_from_slice(&payload[..size]);
            payload = &payload[size + 2..];
        }
        out
    } else if let Some(length) = length {
        if length > MAX_REPLY_BYTES || payload.len() < length {
            return Err(BAD.into());
        }
        payload[..length].to_vec()
    } else {
        if payload.len() > MAX_REPLY_BYTES {
            return Err(BAD.into());
        }
        payload.to_vec()
    };
    Ok(OwnerMutationReply {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    const KEY: &str = "fixture-operator-key";

    fn key_file(dir: &Path, mode: u32) -> PathBuf {
        let path = dir.join("operator.key");
        std::fs::write(&path, format!("{KEY}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    /// A one-shot loopback "daemon" that records the raw request it gets.
    fn fake_daemon(reply: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .map(|v| v.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            socket.write_all(reply.as_bytes()).unwrap();
            String::from_utf8(seen).unwrap()
        });
        (port, handle)
    }

    #[test]
    fn only_the_three_owner_routes_can_be_built() {
        assert_eq!(
            OwnerMutation::parse("rename_owner", None, None)
                .unwrap()
                .path(),
            "/v1/me"
        );
        let decision =
            OwnerMutation::parse("run_permission", Some("team room"), Some("run/1")).unwrap();
        assert_eq!(decision.method(), "POST");
        assert_eq!(
            decision.path(),
            "/v1/rooms/persistent/team%20room/runs/run%2F1/permission"
        );
        let settings = OwnerMutation::parse("agent_settings", Some("r"), Some("..")).unwrap();
        assert_eq!(settings.method(), "PUT");
        assert_eq!(
            settings.path(),
            "/v1/rooms/persistent/r/agents/%2E%2E/settings"
        );
        for (kind, room, target) in [
            ("bindings", Some("r"), Some("a")),
            ("rename_owner", Some("r"), None),
            ("run_permission", Some("r"), None),
            ("run_permission", None, Some("x")),
            ("agent_settings", Some(""), Some("a")),
            ("/v1/me", None, None),
        ] {
            assert!(
                OwnerMutation::parse(kind, room, target).is_err(),
                "{kind} {room:?} {target:?}"
            );
        }
    }

    #[test]
    fn owner_mutation_carries_the_operator_key_and_nothing_page_supplied() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(
            "HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\ncontent-length: 18\r\n\r\n{\"ok\":false,\"x\":1}",
        );
        let mutation = OwnerMutation::parse("run_permission", Some("room"), Some("run-1")).unwrap();
        let body = r#"{"permission_id":"p","decision":"allow"}"#;
        let reply = send("127.0.0.1", port, &key, &mutation, body).unwrap();
        let seen = daemon.join().unwrap();
        assert_eq!(reply.status, 409);
        assert_eq!(reply.body, "{\"ok\":false,\"x\":1}");
        assert!(
            seen.starts_with("POST /v1/rooms/persistent/room/runs/run-1/permission HTTP/1.1\r\n")
        );
        assert!(seen.contains(&format!("\r\nX-Ocean-Operator: {KEY}\r\n")));
        assert!(seen.ends_with(body));
        let lower = seen.to_ascii_lowercase();
        assert!(!lower.contains("\r\norigin:") && !lower.contains("\r\ncookie:"));
        assert!(
            !format!("{reply:?}").contains(KEY),
            "the key never reaches the page"
        );
    }

    #[test]
    fn chunked_replies_are_decoded() {
        let reply = parse_reply(
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(
            reply,
            OwnerMutationReply {
                status: 200,
                body: "{\"a\":1}".into()
            }
        );
    }

    #[test]
    fn the_key_is_refused_unless_private_and_never_leaves_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let mutation = OwnerMutation::parse("rename_owner", None, None).unwrap();
        // Missing, group-readable, or symlinked keys refuse before any connect
        // (port 9 is never contacted: the key check fails first).
        let missing = dir.path().join("absent.key");
        let error = send("127.0.0.1", 9, &missing, &mutation, "{}").unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
        let loose = key_file(dir.path(), 0o644);
        let error = send("127.0.0.1", 9, &loose, &mutation, "{}").unwrap_err();
        assert!(error.contains("0600") && !error.contains(KEY), "{error}");
        let private = key_file(dir.path(), 0o600);
        let link = dir.path().join("link.key");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert!(send("127.0.0.1", 9, &link, &mutation, "{}").is_err());
        // A non-loopback daemon never receives the key.
        let error = send("192.0.2.1", 4780, &private, &mutation, "{}").unwrap_err();
        assert!(error.contains("loopback"), "{error}");
        assert_eq!(read_operator_key(&private).unwrap(), KEY);
    }
}
