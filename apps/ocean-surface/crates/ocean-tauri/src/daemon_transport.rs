//! The one native transport for operator-gated Room requests.
//!
//! Two shell modules attach the daemon's header-only Room operator credential
//! (`X-Ocean-Operator`): `owner_mutation.rs` (rename owner, run permission,
//! agent settings) and `room_consent.rs` (Room agent consent and revoke). Both
//! build their requests from typed intents and hand them to this module, which
//! owns every transport and custody rule once:
//!
//! * The daemon address comes from `OCEAN_DAEMON_URL` (`daemon_url_from_env`)
//!   and must be `http://<numeric loopback>[:port][/]`. Hostnames (including
//!   `localhost`), userinfo, paths, queries and TLS schemes are refused, so the
//!   credential is never resolved or routed anywhere else. An untrusted URL
//!   leaves every request unsent.
//! * Requests are raw HTTP/1.1 over one fresh connection. The caller supplies
//!   only a fixed method, a path built from its typed intent and an optional
//!   JSON body; the transport sets every header (`Host`, `Accept`,
//!   `Connection: close`, `Content-Type`/`Content-Length` with a body and the
//!   operator header for privileged requests). No `Origin`, no `Cookie`.
//! * The key is read through an `O_NOFOLLOW` descriptor immediately before
//!   each privileged request. The opened file must be the inspected regular
//!   (non-symlink) file, owned by the effective uid, with no group/other
//!   permission bits, and hold one header-safe token. It is never cached,
//!   returned, logged or embedded in an error: errors carry fixed codes only.
//!   Windows uses the daemon's protected current-user-only ACL reader instead,
//!   refusing reparse points and extra hard links with the same token bound.
//! * Replies must be JSON objects of at most 256 KiB. Redirects are refused,
//!   duplicate `Content-Length`, non-chunked transfer codings and bad chunk
//!   framing are bad replies, and a hostile chunk size can never panic.
//! * The outcome distinguishes a request that was never sent
//!   ([`SendError::NotSent`]), one whose acknowledgement was lost
//!   ([`SendError::Unknown`]) and one answered with an unacceptable reply
//!   ([`SendError::BadReply`]). Callers never retry automatically.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

/// Header the daemon reads the Room operator credential from.
const OPERATOR_HEADER: &str = "X-Ocean-Operator";
/// Largest daemon reply body accepted.
pub(crate) const MAX_REPLY_BYTES: usize = 256 * 1024;
/// Headroom for the status line and headers on top of the body bound.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest request body a caller may send (settings are ≤4000 chars of
/// instructions plus a model alias; decisions are tiny).
pub(crate) const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
/// Port used when the daemon URL names none.
const DEFAULT_PORT: u16 = 4780;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest operator key file read.
const MAX_KEY_FILE_BYTES: u64 = 4096;

pub(crate) const CREDENTIAL_UNAVAILABLE: &str = "operator_credential_unavailable";

// ── endpoint ────────────────────────────────────────────────────────────

/// Accept only `http://<numeric loopback>[:port][/]`. Hostnames (including
/// `localhost`), userinfo, paths, non-loopback addresses and TLS schemes are
/// refused so the credential cannot be resolved or routed anywhere else.
pub(crate) fn loopback_endpoint(url: &str) -> Option<SocketAddr> {
    let rest = url.trim().strip_prefix("http://")?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', ' ']) {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, tail) = v6.split_once(']')?;
        let port = match tail {
            "" => DEFAULT_PORT,
            tail => tail.strip_prefix(':')?.parse().ok()?,
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, DEFAULT_PORT),
        }
    };
    let ip: IpAddr = host.parse().ok()?;
    (ip.is_loopback() && port != 0).then_some(SocketAddr::new(ip, port))
}

// ── credential custody ──────────────────────────────────────────────────

/// Select an absolute Windows home, preserving the existing Unix HOME rule.
pub(crate) fn profile_home(
    home: Option<std::ffi::OsString>,
    profile: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        [home, profile]
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .find(|path| path.is_absolute())
    }
    #[cfg(not(windows))]
    {
        let _ = profile;
        home.map(PathBuf::from)
    }
}

/// The daemon state directory, with explicit config overrides first.
fn ocean_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("OCEAN_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("ocean-rs");
    }
    if let Some(home) = profile_home(std::env::var_os("HOME"), std::env::var_os("USERPROFILE")) {
        return home.join(".config").join("ocean-rs");
    }
    PathBuf::from(".ocean-rs")
}

/// `OCEAN_OPERATOR_KEY_FILE`, else `<config dir>/operator.key` — the path the
/// daemon writes and the Surface proxy reads.
pub(crate) fn operator_key_path() -> PathBuf {
    std::env::var_os("OCEAN_OPERATOR_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ocean_config_dir().join("operator.key"))
}

/// Ownership and permission policy for the opened key file: owned by the
/// effective uid, no group/other permission bits.
#[cfg(unix)]
fn credential_policy(owner_uid: u32, mode: u32, euid: u32) -> Result<(), &'static str> {
    if owner_uid != euid {
        return Err("operator_credential_foreign_owner");
    }
    if mode & 0o077 != 0 {
        return Err("operator_credential_not_private");
    }
    Ok(())
}

/// One header-safe token: 16..=512 bytes of base64/base64url alphabet.
fn header_safe_token(key: &str) -> bool {
    (16..=512).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'+' | b'/'))
}

/// Read the operator key through a no-follow descriptor. The opened file must
/// be the inspected regular file, owned by this user, with no group/other
/// permission bits, and hold one header-safe token. Errors carry only a code.
#[cfg(unix)]
pub(crate) fn read_operator_key(path: &Path) -> Result<String, &'static str> {
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
    credential_policy(opened.uid(), opened.mode(), unsafe { libc::geteuid() })?;
    if opened.len() > MAX_KEY_FILE_BYTES {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    let mut raw = String::new();
    (&mut file)
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    let key = raw.trim();
    if !header_safe_token(key) {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    Ok(key.to_string())
}

#[cfg(windows)]
pub(crate) fn read_operator_key(path: &Path) -> Result<String, &'static str> {
    use std::os::windows::fs::MetadataExt;
    let link = std::fs::symlink_metadata(path).map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    // FILE_ATTRIBUTE_REPARSE_POINT includes symlinks and junctions.
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if !link.is_file() || link.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("operator_credential_not_regular");
    }
    let mut file = ocean_observatory::auth::windows_file::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            "operator_credential_not_private"
        } else {
            CREDENTIAL_UNAVAILABLE
        }
    })?;
    if file.metadata().map_err(|_| CREDENTIAL_UNAVAILABLE)?.len() > MAX_KEY_FILE_BYTES {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    let mut raw = String::new();
    (&mut file)
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|_| CREDENTIAL_UNAVAILABLE)?;
    let key = raw.trim();
    if !header_safe_token(key) {
        return Err(CREDENTIAL_UNAVAILABLE);
    }
    Ok(key.to_owned())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn read_operator_key(_path: &Path) -> Result<String, &'static str> {
    Err(CREDENTIAL_UNAVAILABLE)
}

// ── request building blocks ─────────────────────────────────────────────

/// Percent-encode one path segment: only RFC 3986 unreserved bytes pass, and
/// dot segments are fully encoded (a dot segment is a traversal step, never
/// an id).
pub(crate) fn encode_segment(raw: &str) -> String {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }
}

/// One request built by a caller's typed intent. Page code never reaches this
/// type: callers build the method and path themselves from fixed vocabularies
/// and percent-encoded ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonRequest {
    method: Method,
    path: String,
    body: Option<String>,
    operator: bool,
}

impl DaemonRequest {
    /// An unprivileged request: no credential is read or sent.
    pub(crate) fn public(method: Method, path: String, body: Option<String>) -> Self {
        Self {
            method,
            path,
            body,
            operator: false,
        }
    }

    /// A privileged request carrying the operator credential.
    pub(crate) fn operator(method: Method, path: String, body: Option<String>) -> Self {
        Self {
            method,
            path,
            body,
            operator: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_operator(&self) -> bool {
        self.operator
    }

    /// A path from a typed intent is `/`-rooted visible ASCII; anything else
    /// would be a header-injection or request-smuggling bug in a caller.
    fn valid_path(&self) -> bool {
        self.path.starts_with("/v1/")
            && self.path.bytes().all(|b| b.is_ascii_graphic())
            && !self.path.contains(['?', '#'])
    }
}

// ── outcome ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendError {
    /// Nothing reached the daemon (endpoint, validation, credential, connect).
    NotSent(&'static str),
    /// The request may have reached the daemon; the acknowledgement was lost.
    Unknown,
    /// A reply arrived but is not acceptable (redirect, oversize, non-JSON).
    BadReply(&'static str),
}

impl SendError {
    /// A fixed code for the page. Never carries credential material.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NotSent(code) | Self::BadReply(code) => code,
            Self::Unknown => "outcome_unknown",
        }
    }
}

/// A definitive daemon reply: status plus a JSON object body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Map<String, Value>,
}

// ── transport ───────────────────────────────────────────────────────────

/// The resolved numeric-loopback daemon endpoint plus the operator key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonTransport {
    endpoint: Option<SocketAddr>,
    key_path: PathBuf,
}

impl DaemonTransport {
    /// `daemon_url` should come from `daemon_url_from_env()`. An untrusted URL
    /// yields a transport that sends nothing.
    pub(crate) fn new(daemon_url: &str, key_path: PathBuf) -> Self {
        Self {
            endpoint: loopback_endpoint(daemon_url),
            key_path,
        }
    }

    /// The trusted endpoint, or `None` when `OCEAN_DAEMON_URL` was refused.
    pub(crate) fn endpoint(&self) -> Option<SocketAddr> {
        self.endpoint
    }

    /// Send one request. Blocking; call off the async runtime.
    pub(crate) fn send(&self, request: &DaemonRequest) -> Result<Reply, SendError> {
        let endpoint = self
            .endpoint
            .ok_or(SendError::NotSent("daemon_not_loopback"))?;
        if !request.valid_path() {
            return Err(SendError::NotSent("invalid_request"));
        }
        let body = request.body.as_deref();
        if body.is_some_and(|b| b.len() > MAX_REQUEST_BODY_BYTES) {
            return Err(SendError::NotSent("request_too_large"));
        }
        let key = if request.operator {
            Some(read_operator_key(&self.key_path).map_err(SendError::NotSent)?)
        } else {
            None
        };
        let mut stream = TcpStream::connect_timeout(&endpoint, CONNECT_TIMEOUT)
            .map_err(|_| SendError::NotSent("daemon_unreachable"))?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {endpoint}\r\nAccept: application/json\r\nConnection: close\r\n",
            request.method.as_str(),
            request.path,
        );
        if let Some(body) = body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        if let Some(key) = key.as_deref() {
            head.push_str(&format!("{OPERATOR_HEADER}: {key}\r\n"));
        }
        head.push_str("\r\n");
        let written = stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body.unwrap_or_default().as_bytes()));
        drop(head);
        drop(key);
        written.map_err(|_| SendError::Unknown)?;
        let mut raw = Vec::new();
        stream
            .take((MAX_REPLY_BYTES + MAX_HEAD_BYTES + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|_| SendError::Unknown)?;
        if raw.is_empty() {
            return Err(SendError::Unknown);
        }
        parse_reply(&raw)
    }
}

/// Parse an HTTP/1.1 reply (Content-Length, chunked, or close-delimited) into
/// a status and a JSON object body.
pub(crate) fn parse_reply(raw: &[u8]) -> Result<Reply, SendError> {
    const BAD: SendError = SendError::BadReply("invalid_reply");
    if raw.len() > MAX_REPLY_BYTES + MAX_HEAD_BYTES {
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
            // A hostile size must not overflow or slice out of bounds.
            if size > MAX_REPLY_BYTES.saturating_sub(out.len()) {
                return Err(BAD);
            }
            let framed = size.checked_add(2).ok_or(BAD)?;
            let (chunk, rest) = payload.split_at_checked(framed).ok_or(BAD)?;
            if &chunk[size..] != b"\r\n" {
                return Err(BAD);
            }
            out.extend_from_slice(&chunk[..size]);
            payload = rest;
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

// ── window gate ─────────────────────────────────────────────────────────

/// Operator-gated commands answer only the `main` webview window (Tauri
/// plugin ACLs do not cover application commands). `refusal` is the caller's
/// fixed error code.
pub(crate) fn require_main_window(label: &str, refusal: &'static str) -> Result<(), String> {
    (label == "main")
        .then_some(())
        .ok_or_else(|| refusal.to_string())
}

// ── shared test fixtures ────────────────────────────────────────────────

#[cfg(all(test, any(unix, windows)))]
pub(crate) mod test_support {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub(crate) const KEY: &str = "fixtureOperatorKey_0123456789abcdefABCDEF-xyz";

    pub(crate) fn key_file(dir: &Path, mode: u32) -> PathBuf {
        let path = dir.join("operator.key");
        // Replace rather than rewrite: an earlier fixture may be read-only.
        let _ = std::fs::remove_file(&path);
        #[cfg(unix)]
        {
            std::fs::write(&path, format!("{KEY}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(windows)]
        if mode & 0o077 == 0 {
            private_bytes(&path, format!("{KEY}\n").as_bytes());
        } else {
            // An ordinary inherited DACL is the Windows unsafe-file fixture.
            std::fs::write(&path, format!("{KEY}\n")).unwrap();
        }
        path
    }

    pub(crate) fn private_bytes(path: &Path, bytes: &[u8]) {
        let _ = std::fs::remove_file(path);
        #[cfg(unix)]
        {
            std::fs::write(path, bytes).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(windows)]
        {
            let mut file = ocean_observatory::auth::windows_file::create(path).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }
    }

    pub(crate) fn credential_symlink(source: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(source, link).unwrap();
    }

    pub(crate) fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// Loopback "daemon" answering each accepted connection with the next
    /// canned reply (`None` closes without replying). Returns raw requests.
    pub(crate) fn fake_daemon(
        replies: Vec<Option<String>>,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
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
                    let _ = socket.write_all(reply.as_bytes());
                }
                seen.push(String::from_utf8(raw).unwrap());
            }
            seen
        });
        (port, handle)
    }

    /// A bound port that records whether anything ever connected.
    pub(crate) fn silent_daemon() -> (u16, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        (listener.local_addr().unwrap().port(), listener)
    }

    pub(crate) fn assert_never_contacted(listener: &TcpListener) {
        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("daemon was contacted: {other:?}"),
        }
    }

    pub(crate) fn json_body(request: &str) -> serde_json::Value {
        serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap()
    }
}

// ── tests ───────────────────────────────────────────────────────────────

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::test_support::*;
    use super::*;
    fn transport(port: u16, key: &Path) -> DaemonTransport {
        DaemonTransport::new(&format!("http://127.0.0.1:{port}"), key.to_path_buf())
    }

    fn privileged() -> DaemonRequest {
        DaemonRequest::operator(Method::Put, "/v1/me".into(), Some("{}".into()))
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_profile_and_link_custody() {
        use std::ffi::OsString;
        let profile = OsString::from(r"C:\Users\Coworker");
        assert_eq!(
            profile_home(None, Some(profile.clone())),
            Some(PathBuf::from(&profile))
        );
        assert_eq!(
            profile_home(Some("relative".into()), Some(profile)),
            Some(PathBuf::from(r"C:\Users\Coworker"))
        );
        assert_eq!(
            profile_home(Some(r"D:\Chosen".into()), Some(r"C:\Users\Coworker".into())),
            Some(PathBuf::from(r"D:\Chosen"))
        );
        assert_eq!(profile_home(None, Some("relative".into())), None);
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let before = std::fs::read(&key).unwrap();
        std::fs::hard_link(&key, dir.path().join("second.key")).unwrap();
        let (port, listener) = silent_daemon();
        assert!(matches!(
            transport(port, &key).send(&privileged()),
            Err(SendError::NotSent(_))
        ));
        assert_never_contacted(&listener);
        assert_eq!(std::fs::read(&key).unwrap(), before);
    }

    #[test]
    fn only_numeric_loopback_endpoints_are_trusted() {
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:4780"),
            Some("127.0.0.1:4780".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:4780/"),
            Some("127.0.0.1:4780".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://[::1]:9/"),
            Some("[::1]:9".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://[::1]"),
            Some("[::1]:4780".parse().unwrap())
        );
        assert_eq!(
            loopback_endpoint("http://127.0.0.1"),
            Some("127.0.0.1:4780".parse().unwrap())
        );
        for url in [
            "http://localhost:4780",
            "http://LOCALHOST",
            "http://localhost.:4780",
            "https://127.0.0.1:4780",
            "http://10.0.0.5:4780",
            "http://0.0.0.0:4780",
            "http://[::]:4780",
            "http://user@127.0.0.1:4780",
            "http://user:pw@127.0.0.1:4780",
            "http://127.0.0.1:4780/v1",
            "http://127.0.0.1:4780?x=1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:99999",
            "http://127.1:4780",
            "http://2130706433:4780",
            "127.0.0.1:4780",
            "",
            "http://",
            "http://evil.example#@127.0.0.1",
            "http://[::1]x",
        ] {
            assert_eq!(loopback_endpoint(url), None, "{url}");
        }
    }

    #[test]
    fn an_untrusted_endpoint_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        for url in ["http://localhost:4780", "http://192.0.2.1:4780"] {
            let transport = DaemonTransport::new(url, key.clone());
            assert_eq!(transport.endpoint(), None);
            assert_eq!(
                transport.send(&privileged()).unwrap_err(),
                SendError::NotSent("daemon_not_loopback")
            );
        }
    }

    #[test]
    fn key_files_are_refused_unless_private_regular_and_header_safe() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_operator_key(&dir.path().join("absent.key")).unwrap_err(),
            CREDENTIAL_UNAVAILABLE
        );
        for mode in [0o640, 0o644, 0o604, 0o660, 0o606] {
            let loose = key_file(dir.path(), mode);
            assert_eq!(
                read_operator_key(&loose).unwrap_err(),
                "operator_credential_not_private",
                "{mode:o}"
            );
        }
        // Owner-only modes other than 0600 are accepted (strictest of the two
        // former readers: no group/other bits, rather than exactly 0600).
        for mode in [0o400, 0o600] {
            let private = key_file(dir.path(), mode);
            assert_eq!(read_operator_key(&private).unwrap(), KEY, "{mode:o}");
        }
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
        for content in [
            "abcdefghijklmnopqrstuvwx\r\nOrigin: x",
            "short",
            "has space in the middle of it",
            "",
        ] {
            let path = dir.path().join("odd.key");
            private_bytes(&path, content.as_bytes());
            let error = read_operator_key(&path).unwrap_err();
            assert_eq!(error, CREDENTIAL_UNAVAILABLE, "{content:?}");
        }
        let big = dir.path().join("big.key");
        private_bytes(&big, "a".repeat(5000).as_bytes());
        assert_eq!(read_operator_key(&big).unwrap_err(), CREDENTIAL_UNAVAILABLE);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_owned_by_another_user_is_refused() {
        assert_eq!(credential_policy(501, 0o100600, 501), Ok(()));
        assert_eq!(credential_policy(501, 0o100400, 501), Ok(()));
        assert_eq!(
            credential_policy(0, 0o100600, 501),
            Err("operator_credential_foreign_owner")
        );
        assert_eq!(
            credential_policy(502, 0o100600, 501),
            Err("operator_credential_foreign_owner")
        );
        assert_eq!(
            credential_policy(501, 0o100640, 501),
            Err("operator_credential_not_private")
        );
    }

    #[test]
    fn a_refused_credential_never_connects_or_leaks_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let (port, listener) = silent_daemon();
        let loose = key_file(dir.path(), 0o644);
        let error = transport(port, &loose).send(&privileged()).unwrap_err();
        assert_eq!(error, SendError::NotSent("operator_credential_not_private"));
        assert!(!format!("{error:?}").contains(KEY));
        assert!(!error.code().contains(KEY));
        let missing = dir.path().join("absent.key");
        assert_eq!(
            transport(port, &missing).send(&privileged()).unwrap_err(),
            SendError::NotSent(CREDENTIAL_UNAVAILABLE)
        );
        assert_never_contacted(&listener);
    }

    #[test]
    fn privileged_requests_carry_only_transport_headers_and_public_ones_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![
            Some(http("200 OK", r#"{"ok":true}"#)),
            Some(http("200 OK", r#"{"ok":true}"#)),
        ]);
        let t = transport(port, &key);
        let body = r#"{"display_name":"x"}"#;
        let reply = t
            .send(&DaemonRequest::operator(
                Method::Put,
                "/v1/me".into(),
                Some(body.into()),
            ))
            .unwrap();
        assert_eq!(reply.status, 200);
        t.send(&DaemonRequest::public(Method::Get, "/v1/me".into(), None))
            .unwrap();
        let seen = daemon.join().unwrap();
        assert_eq!(
            seen[0],
            format!(
                "PUT /v1/me HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Ocean-Operator: {KEY}\r\n\r\n{body}",
                body.len()
            )
        );
        assert_eq!(
            seen[1],
            format!(
                "GET /v1/me HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
            )
        );
        assert!(!format!("{reply:?}").contains(KEY));
    }

    #[test]
    fn malformed_paths_and_oversized_bodies_are_never_sent() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, listener) = silent_daemon();
        let t = transport(port, &key);
        for path in [
            "/v1/me HTTP/1.1\r\nX: y",
            "/v1/me?x=1",
            "/v1/a b",
            "v1/me",
            "/v2/me",
            "http://evil/v1/me",
        ] {
            assert_eq!(
                t.send(&DaemonRequest::operator(Method::Put, path.into(), None))
                    .unwrap_err(),
                SendError::NotSent("invalid_request"),
                "{path:?}"
            );
        }
        let huge = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
        assert_eq!(
            t.send(&DaemonRequest::operator(
                Method::Put,
                "/v1/me".into(),
                Some(huge)
            ))
            .unwrap_err(),
            SendError::NotSent("request_too_large")
        );
        assert_never_contacted(&listener);
    }

    #[test]
    fn an_unreachable_daemon_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        assert_eq!(
            transport(port, &key).send(&privileged()).unwrap_err(),
            SendError::NotSent("daemon_unreachable")
        );
    }

    #[test]
    fn a_lost_acknowledgement_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let (port, daemon) = fake_daemon(vec![None]);
        let error = transport(port, &key).send(&privileged()).unwrap_err();
        assert_eq!(error, SendError::Unknown);
        assert_eq!(error.code(), "outcome_unknown");
        let seen = daemon.join().unwrap();
        assert_eq!(seen.len(), 1, "the request did reach the daemon");
        assert!(seen[0].contains(KEY));
    }

    #[test]
    fn redirects_are_refused_over_the_wire() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        for status in [
            "301 Moved Permanently",
            "302 Found",
            "307 Temporary Redirect",
        ] {
            let (port, daemon) = fake_daemon(vec![Some(format!(
                "HTTP/1.1 {status}\r\nlocation: http://evil.example/\r\ncontent-length: 2\r\n\r\n{{}}"
            ))]);
            assert_eq!(
                transport(port, &key).send(&privileged()).unwrap_err(),
                SendError::BadReply("redirect_refused"),
                "{status}"
            );
            assert_eq!(daemon.join().unwrap().len(), 1, "no redirect followed");
        }
    }

    #[test]
    fn oversize_and_invalid_replies_are_bad_replies() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file(dir.path(), 0o600);
        let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_REPLY_BYTES));
        let close_delimited_huge = format!("HTTP/1.1 200 OK\r\n\r\n{huge}");
        let cases = vec![
            (http("200 OK", &huge), "invalid_reply"),
            (close_delimited_huge, "reply_too_large"),
            (http("200 OK", "not json"), "invalid_reply"),
            (http("200 OK", "[1,2]"), "invalid_reply"),
            (http("200 OK", "\"text\""), "invalid_reply"),
            (http("204 No Content", ""), "invalid_reply"),
            ("garbage without a head".to_string(), "invalid_reply"),
        ];
        for (reply, code) in cases {
            let (port, daemon) = fake_daemon(vec![Some(reply)]);
            assert_eq!(
                transport(port, &key).send(&privileged()).unwrap_err(),
                SendError::BadReply(code)
            );
            daemon.join().unwrap();
        }
        // A JSON object at exactly the bound is accepted.
        let fits = format!(
            "{{\"pad\":\"{}\"}}",
            "x".repeat(MAX_REPLY_BYTES - "{\"pad\":\"\"}".len())
        );
        assert_eq!(fits.len(), MAX_REPLY_BYTES);
        let reply = parse_reply(http("200 OK", &fits).as_bytes()).unwrap();
        assert_eq!(reply.status, 200);
        // Non-2xx JSON replies are definitive, not errors.
        let reply =
            parse_reply(http("409 Conflict", r#"{"ok":false,"error":"x"}"#).as_bytes()).unwrap();
        assert_eq!(reply.status, 409);
        assert_eq!(reply.body["error"], "x");
    }

    #[test]
    fn chunked_replies_decode_and_bad_framing_is_refused() {
        let reply = parse_reply(
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["a"], 1);
        for raw in [
            &b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\ncontent-length: 2\r\n\r\n{}"[..],
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: gzip\r\n\r\n{}",
            b"SSH-2.0 200\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nno-colon-header\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nffffffffffffffff\r\n{}\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nfffffffffffffffe\r\n{}\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n2\r\n{}XX0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n40001\r\n{}\r\n0\r\n\r\n",
        ] {
            assert_eq!(
                parse_reply(raw).unwrap_err(),
                SendError::BadReply("invalid_reply"),
                "{}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn path_segments_are_percent_encoded_and_dot_segments_neutralized() {
        assert_eq!(encode_segment("team room"), "team%20room");
        assert_eq!(encode_segment("run/1"), "run%2F1");
        assert_eq!(encode_segment(".."), "%2E%2E");
        assert_eq!(encode_segment("."), "%2E");
        assert_eq!(encode_segment("a.b-c_d~e"), "a.b-c_d~e");
        assert_eq!(encode_segment("x\r\ny?#"), "x%0D%0Ay%3F%23");
    }

    #[test]
    fn only_the_main_window_passes_the_gate() {
        assert_eq!(require_main_window("main", "refused"), Ok(()));
        for label in ["", "Main", "floating", "main ", "main-2"] {
            assert_eq!(
                require_main_window(label, "refused"),
                Err("refused".to_string()),
                "{label:?}"
            );
        }
    }
}
