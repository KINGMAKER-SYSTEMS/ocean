//! Keep Ocean's `auth.json` OAuth blocks fresh.
//!
//! Ocean previously only *read* OAuth tokens that external CLIs (Claude Code,
//! Codex) wrote — an expired block resolved to "missing credential" and the
//! turn hard-failed until the user re-logged-in elsewhere. This module runs at
//! the top of the turn path: for each known refreshable block in **Ocean's
//! own** `auth.json` (never `~/.codex/auth.json` or any other CLI's file) that
//! is expired or expiring inside the margin, it exchanges the refresh token at
//! the issuer's public-client token endpoint and rewrites the block atomically
//! (temp file + rename in the same directory — a crash never corrupts the
//! file other CLIs may share).
//!
//! Failure is graceful and rate-limited: a failed refresh logs, starts a
//! per-block cooldown (no hammering the endpoint every turn), and leaves
//! behavior exactly as pre-refresh (expired → missing credential).
//! Network responses are merged only into the unchanged block they refreshed,
//! after a fresh read under shared custody. No response body is logged.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{io, path::Path};

use serde_json::Value;

/// Refresh when a token expires within this margin — covers the whole turn so
/// a token can't expire mid-run.
const EXPIRY_MARGIN_SECS: i64 = 300;
/// After a failed refresh, don't retry that block for this long.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

/// One refreshable block: where it lives in auth.json and how to refresh it.
struct RefreshableBlock {
    /// Top-level key in auth.json.
    block: &'static str,
    /// Env var overriding the token endpoint (tests, region overrides).
    endpoint_env: &'static str,
    default_endpoint: &'static str,
    /// Env var overriding the OAuth client id.
    client_id_env: &'static str,
    /// The issuer's public-client id (the same one the vendor CLI itself
    /// embeds; refreshing the user's own tokens with it is the intended flow).
    default_client_id: &'static str,
}

const BLOCKS: &[RefreshableBlock] = &[
    RefreshableBlock {
        block: "claude-code",
        endpoint_env: "OCEAN_OAUTH_ANTHROPIC_TOKEN_URL",
        default_endpoint: "https://console.anthropic.com/v1/oauth/token",
        client_id_env: "OCEAN_OAUTH_ANTHROPIC_CLIENT_ID",
        default_client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    },
    RefreshableBlock {
        block: "anthropic-oauth",
        endpoint_env: "OCEAN_OAUTH_ANTHROPIC_TOKEN_URL",
        default_endpoint: "https://console.anthropic.com/v1/oauth/token",
        client_id_env: "OCEAN_OAUTH_ANTHROPIC_CLIENT_ID",
        default_client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    },
    RefreshableBlock {
        block: "openai-codex",
        endpoint_env: "OCEAN_OAUTH_OPENAI_TOKEN_URL",
        default_endpoint: "https://auth.openai.com/oauth/token",
        client_id_env: "OCEAN_OAUTH_OPENAI_CLIENT_ID",
        default_client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    },
];

/// Single-flight + per-block failure cooldowns. One global refresh pass runs
/// at a time (concurrent turns don't race the file or double-spend a rotating
/// refresh token); failed blocks are skipped until their cooldown lapses.
struct RefreshState {
    cooldowns: HashMap<String, Instant>,
}

fn state() -> &'static tokio::sync::Mutex<RefreshState> {
    static STATE: OnceLock<tokio::sync::Mutex<RefreshState>> = OnceLock::new();
    STATE.get_or_init(|| {
        tokio::sync::Mutex::new(RefreshState {
            cooldowns: HashMap::new(),
        })
    })
}

/// Refresh every expiring refreshable block in `auth_file`. Cheap no-op when
/// the file is absent or every block is fresh. Never returns an error — a
/// refresh failure degrades to today's behavior (expired block → missing
/// credential) with a warning and a cooldown.
pub async fn ensure_fresh(auth_file: &Path) {
    if !auth_file.exists() {
        return;
    }
    let mut guard = state().lock().await;

    let Ok(raw) = std::fs::read_to_string(auth_file) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return; // malformed file: resolution will surface it; don't touch
    };

    let mut refreshed = Vec::new();
    for def in BLOCKS {
        let Some((refresh, needs)) = block_needs_refresh(&json, def.block) else {
            continue;
        };
        if !needs {
            continue;
        }
        if let Some(until) = guard.cooldowns.get(def.block) {
            if until.elapsed() < FAILURE_COOLDOWN {
                continue;
            }
        }
        let endpoint =
            std::env::var(def.endpoint_env).unwrap_or_else(|_| def.default_endpoint.to_string());
        let client_id =
            std::env::var(def.client_id_env).unwrap_or_else(|_| def.default_client_id.to_string());
        match ocean_protocol::oauth::refresh_token(&endpoint, &client_id, &refresh).await {
            Ok(fresh) => {
                refreshed.push(Refreshed {
                    block: def.block,
                    expected_block: json.get(def.block).expect("block existed above").clone(),
                    used_refresh: refresh,
                    access: fresh.access_token,
                    refresh: fresh.refresh_token,
                    expires_ms: fresh
                        .expires_in_secs
                        .map(|secs| (unix_secs() + secs) * 1_000),
                });
            }
            Err(e) => {
                guard
                    .cooldowns
                    .insert(def.block.to_string(), Instant::now());
                let (class, status) = refresh_error_class(&e);
                tracing::warn!(
                    block = %def.block,
                    class,
                    status,
                    "OAuth refresh failed; leaving block as-is (60s cooldown)"
                );
            }
        }
    }

    if refreshed.is_empty() {
        return;
    }
    let attempted: Vec<_> = refreshed.iter().map(|item| item.block).collect();
    let path = auth_file.to_path_buf();
    // File-lock contention must not park a Tokio worker. The refresh singleflight
    // remains held, but filesystem custody never crosses a network call/await.
    match tokio::task::spawn_blocking(move || merge_refreshed(&path, &refreshed)).await {
        Ok(Ok(applied)) => {
            for block in attempted {
                guard.cooldowns.remove(block);
            }
            for block in applied {
                tracing::info!(block, "persisted refreshed OAuth token");
            }
        }
        result => {
            for block in attempted {
                guard.cooldowns.insert(block.to_string(), Instant::now());
            }
            match result {
                Ok(Err(error)) => tracing::warn!(
                    class = error.class(),
                    published = error.published(),
                    "OAuth refresh persistence unconfirmed (60s cooldown)"
                ),
                Err(_) => tracing::warn!(
                    class = "storage_worker",
                    "OAuth refresh persistence unconfirmed (60s cooldown)"
                ),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}

fn refresh_error_class(error: &ocean_protocol::Error) -> (&'static str, Option<u16>) {
    use ocean_protocol::Error;
    match error {
        Error::ProviderError { status, .. } => ("provider_status", Some(*status)),
        Error::Http(error) => ("transport", error.status().map(|status| status.as_u16())),
        Error::RetryExhausted { source, .. } => refresh_error_class(source),
        Error::InvalidResponse(_) => ("invalid_response", None),
        Error::Json(_) => ("invalid_json", None),
        Error::Cancelled => ("cancelled", None),
        Error::Io(_) => ("io", None),
        Error::MissingApiKey(_) => ("missing_credential", None),
        Error::UnsupportedProvider(_) => ("unsupported_provider", None),
        Error::Other(_) => ("other", None),
    }
}

// Deliberately not Debug: these fields contain credential material.
struct Refreshed {
    block: &'static str,
    expected_block: Value,
    used_refresh: String,
    access: String,
    refresh: Option<String>,
    expires_ms: Option<i64>,
}

#[derive(Debug)]
enum MergeFailure {
    Custody(io::ErrorKind),
    Read(io::ErrorKind),
    InvalidJson,
    Serialization,
    Publication(ocean_providers::AuthFileWriteError),
}

impl MergeFailure {
    fn class(&self) -> &'static str {
        match self {
            Self::Custody(io::ErrorKind::TimedOut) => "custody_timeout",
            Self::Custody(_) => "custody",
            Self::Read(io::ErrorKind::PermissionDenied) => "read_denied",
            Self::Read(_) => "read",
            Self::InvalidJson => "invalid_json",
            Self::Serialization => "serialization",
            Self::Publication(error) if error.published() => "directory_durability",
            Self::Publication(_) => "publication",
        }
    }

    fn published(&self) -> bool {
        matches!(self, Self::Publication(error) if error.published())
    }
}

/// Merge only responses whose entire original block is still present. Token
/// comparison alone cannot protect a newer login that reuses a refresh token.
fn merge_refreshed(
    auth_file: &Path,
    refreshed: &[Refreshed],
) -> Result<Vec<&'static str>, MergeFailure> {
    let guard = ocean_providers::lock_auth_file(auth_file)
        .map_err(|error| MergeFailure::Custody(error.kind()))?;
    let raw = match guard.read() {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(MergeFailure::Read(error.kind())),
    };
    let mut root: Value = serde_json::from_slice(&raw).map_err(|_| MergeFailure::InvalidJson)?;
    let map = root.as_object_mut().ok_or(MergeFailure::InvalidJson)?;
    let mut applied = Vec::new();
    for item in refreshed {
        let Some(current) = map.get_mut(item.block) else {
            continue;
        };
        if current != &item.expected_block
            || current.get("type").and_then(Value::as_str) != Some("oauth")
            || current
                .get("refresh")
                .and_then(Value::as_str)
                .map(str::trim)
                != Some(item.used_refresh.as_str())
        {
            continue;
        }
        let Some(entry) = current.as_object_mut() else {
            continue;
        };
        entry.insert("access".into(), Value::String(item.access.clone()));
        if let Some(refresh) = &item.refresh {
            entry.insert("refresh".into(), Value::String(refresh.clone()));
        }
        if let Some(expires) = item.expires_ms {
            entry.insert("expires".into(), Value::from(expires));
        }
        applied.push(item.block);
    }
    if !applied.is_empty() {
        let bytes = serde_json::to_vec_pretty(&root).map_err(|_| MergeFailure::Serialization)?;
        guard.publish(&bytes).map_err(MergeFailure::Publication)?;
    }
    Ok(applied)
}

/// `Some((refresh_token, needs_refresh))` for an oauth block that HAS a
/// refresh token; `None` when the block is absent, non-oauth, or unrefreshable.
fn block_needs_refresh(json: &Value, block: &str) -> Option<(String, bool)> {
    let entry = json.get(block)?;
    if entry.get("type").and_then(Value::as_str) != Some("oauth") {
        return None;
    }
    let refresh = entry
        .get("refresh")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    // No known expiry → treat as fresh (nothing to gain by refreshing blind).
    let Some(expires) = entry.get("expires").and_then(Value::as_i64) else {
        return Some((refresh, false));
    };
    let expires_secs = if expires >= 1_000_000_000_000 {
        expires / 1_000
    } else {
        expires
    };
    Some((refresh, expires_secs <= unix_secs() + EXPIRY_MARGIN_SECS))
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn needs_refresh_only_for_expiring_oauth_blocks_with_refresh_tokens() {
        let far = (unix_secs() + 86_400) * 1_000;
        let soon = (unix_secs() + 60) * 1_000;
        let j = json!({
            "claude-code": { "type": "oauth", "access": "a", "refresh": "r", "expires": soon },
            "fresh": { "type": "oauth", "access": "a", "refresh": "r", "expires": far },
            "no-refresh": { "type": "oauth", "access": "a", "expires": 1 },
            "api-key-block": { "api_key": "k" }
        });
        assert_eq!(
            block_needs_refresh(&j, "claude-code").map(|(_, n)| n),
            Some(true),
            "expiring-within-margin block must refresh"
        );
        assert_eq!(
            block_needs_refresh(&j, "fresh").map(|(_, n)| n),
            Some(false),
            "fresh block must not refresh"
        );
        assert!(
            block_needs_refresh(&j, "no-refresh").is_none(),
            "no refresh token → unrefreshable"
        );
        assert!(block_needs_refresh(&j, "api-key-block").is_none());
        assert!(block_needs_refresh(&j, "absent").is_none());
    }

    fn seed(path: &Path, root: &Value) {
        let guard = ocean_providers::lock_auth_file(path).unwrap();
        guard.publish(&serde_json::to_vec(root).unwrap()).unwrap();
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn response(root: &Value, block: &'static str) -> Refreshed {
        Refreshed {
            block,
            expected_block: root[block].clone(),
            used_refresh: root[block]["refresh"].as_str().unwrap().trim().to_string(),
            access: "synthetic-refreshed-access".into(),
            refresh: Some("synthetic-rotated-refresh".into()),
            expires_ms: Some((unix_secs() + 3600) * 1_000),
        }
    }

    #[test]
    fn merge_never_resurrects_removed_blocks_or_clobbers_newer_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = json!({
            "claude-code": {"type":"oauth","access":"old","refresh":"old-a","expires":1},
            "openai-codex": {"type":"oauth","access":"old","refresh":"old-b","expires":1},
            "anthropic-oauth": {"type":"oauth","access":"old","refresh":"old-c","expires":1,"accountId":"fixture-account"},
            "deepseek": {"api_key":"fixture-key"}
        });
        seed(&path, &original);
        let replies = [
            response(&original, "claude-code"),
            response(&original, "openai-codex"),
            response(&original, "anthropic-oauth"),
        ];
        // A cooperating logout/login/key writer commits while refresh is away.
        let mut current = original.clone();
        current.as_object_mut().unwrap().remove("claude-code");
        current["openai-codex"] =
            json!({"type":"oauth","access":"new-login","refresh":"new-login-refresh","expires":9});
        current["deepseek"] = json!({"api_key":"updated-fixture-key"});
        seed(&path, &current);
        let applied = merge_refreshed(&path, &replies).unwrap();
        assert_eq!(applied, ["anthropic-oauth"]);
        let after = read(&path);
        assert!(after.get("claude-code").is_none());
        assert!(after["openai-codex"] == current["openai-codex"]);
        assert!(after["deepseek"] == current["deepseek"]);
        assert!(after["anthropic-oauth"]["access"] == "synthetic-refreshed-access");
        assert!(after["anthropic-oauth"]["accountId"] == original["anthropic-oauth"]["accountId"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777 == 0o600);
        }
        // Removing the whole file is not permission to recreate stale tokens.
        let guard = ocean_providers::lock_auth_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        drop(guard);
        assert!(merge_refreshed(&path, &replies).unwrap().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn merge_same_refresh_newer_block_and_api_key_replacement_remain_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = json!({"openai-codex": {
            "type":"oauth","access":"old","refresh":"same-refresh","expires":1,"accountId":"old-account"
        }});
        let reply = response(&original, "openai-codex");
        for replacement in [
            json!({"type":"oauth","access":"new-login","refresh":"same-refresh","expires":9,"accountId":"new-account"}),
            json!({"api_key":"replacement-fixture-key"}),
        ] {
            seed(&path, &json!({"openai-codex": replacement}));
            let before = std::fs::read(&path).unwrap();
            assert!(merge_refreshed(&path, std::slice::from_ref(&reply))
                .unwrap()
                .is_empty());
            assert!(std::fs::read(&path).unwrap() == before);
        }
    }

    #[test]
    fn merge_refresh_racing_unrelated_writer_uses_its_latest_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = json!({
            "claude-code": {"type":"oauth","access":"old","refresh":"fixture-refresh","expires":1},
            "deepseek": {"api_key":"before"}
        });
        seed(&path, &original);
        let reply = response(&original, "claude-code");
        let worker_path = path.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let refresh = std::thread::spawn(move || {
            // The request already used the old credential snapshot.
            ready_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            merge_refreshed(&worker_path, &[reply]).unwrap()
        });
        ready_rx.recv().unwrap();
        let mut current = original;
        current["deepseek"] = json!({"api_key":"after"});
        seed(&path, &current);
        finish_tx.send(()).unwrap();
        assert_eq!(refresh.join().unwrap(), ["claude-code"]);
        let after = read(&path);
        assert!(after["deepseek"] == current["deepseek"]);
        assert!(after["claude-code"]["refresh"] == "synthetic-rotated-refresh");
    }

    #[test]
    fn merge_invalid_latest_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = json!({"claude-code": {"type":"oauth","access":"old","refresh":"fixture-refresh","expires":1}});
        let reply = response(&original, "claude-code");
        for invalid in [b"{invalid fixture".as_slice(), b"[]".as_slice()] {
            std::fs::write(&path, invalid).unwrap();
            let error = merge_refreshed(&path, std::slice::from_ref(&reply)).unwrap_err();
            assert!(matches!(error, MergeFailure::InvalidJson));
            assert!(std::fs::read(&path).unwrap() == invalid);
        }
    }

    #[test]
    fn refresh_error_projection_has_only_fixed_class_and_status() {
        let error = ocean_protocol::Error::ProviderError {
            status: 401,
            body: "synthetic response content that must stay unlogged".into(),
        };
        assert_eq!(refresh_error_class(&error), ("provider_status", Some(401)));
        let error = ocean_protocol::Error::Other("synthetic arbitrary detail".into());
        assert_eq!(refresh_error_class(&error), ("other", None));
    }

    /// End-to-end against a fake token endpoint: the expiring block is
    /// refreshed on disk (new access + rotated refresh + future expiry), the
    /// fresh block and unrelated keys are untouched.
    #[tokio::test]
    async fn ensure_fresh_rewrites_expiring_block_via_endpoint_override() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    let _ = sock.read(&mut buf).await;
                    let body =
                        r#"{"access_token":"minty","refresh_token":"rotated","expires_in":3600}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(
            &path,
            json!({
                "claude-code": { "type": "oauth", "access": "stale", "refresh": "old-rt", "expires": 1 },
                "deepseek": { "api_key": "keep-me" }
            })
            .to_string(),
        )
        .unwrap();

        // This is the only refresh test setting the endpoint override. Restore
        // the inherited value even if an assertion or refresh task panics.
        struct RestoreEndpoint(Option<std::ffi::OsString>);
        impl Drop for RestoreEndpoint {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("OCEAN_OAUTH_ANTHROPIC_TOKEN_URL", value),
                    None => std::env::remove_var("OCEAN_OAUTH_ANTHROPIC_TOKEN_URL"),
                }
            }
        }
        let _restore = RestoreEndpoint(std::env::var_os("OCEAN_OAUTH_ANTHROPIC_TOKEN_URL"));
        std::env::set_var(
            "OCEAN_OAUTH_ANTHROPIC_TOKEN_URL",
            format!("http://{addr}/oauth/token"),
        );
        ensure_fresh(&path).await;

        let round: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(round["claude-code"]["access"] == "minty");
        assert!(round["claude-code"]["refresh"] == "rotated");
        let exp = round["claude-code"]["expires"].as_i64().unwrap();
        assert!(exp > unix_secs() * 1_000, "expiry moved into the future");
        assert!(round["deepseek"]["api_key"] == "keep-me");
    }
}
