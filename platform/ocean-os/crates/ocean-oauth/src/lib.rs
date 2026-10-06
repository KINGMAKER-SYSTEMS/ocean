//! Ocean OAuth 2.0 + PKCE login.
//!
//! Binds a localhost callback server, builds the provider's authorize URL for
//! the caller to open in a browser, catches the redirect, exchanges the
//! authorization code for tokens, and writes the credential block into Ocean's
//! auth file in the exact shape [`ocean_providers`] (and the turn-time refresh
//! pass in `ocean-agent`) already consume.
//!
//! This crate performs fresh logins only — token refresh already exists
//! (`ocean-agent::oauth_refresh`) and reuses the block shape written here.

mod pkce;
mod providers;
mod server;
mod store;
mod util;

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use providers::{bind_spec, build_authorize_url, build_block, consts, exchange};
use server::{CallbackResult, CallbackServer};

/// Browser-callback grace period before a pending login is abandoned.
const FLOW_TIMEOUT_SECS: u64 = 300;

/// Environment override for the Anthropic (Claude) token endpoint. Matches the
/// same variable `ocean-agent::oauth_refresh` honors, so a test or operator can
/// point both the login and the refresh pass at a shared mock/issuer.
const ENV_ANTHROPIC_TOKEN_URL: &str = "OCEAN_OAUTH_ANTHROPIC_TOKEN_URL";
/// Environment override for the OpenAI Codex token endpoint. See
/// [`ENV_ANTHROPIC_TOKEN_URL`].
const ENV_OPENAI_TOKEN_URL: &str = "OCEAN_OAUTH_OPENAI_TOKEN_URL";

/// The provider whose OAuth flow is being driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    /// Anthropic — Claude Pro/Max subscription (`claude-code` auth block).
    Claude,
    /// OpenAI Codex — ChatGPT plan (`openai-codex` auth block).
    Codex,
}

impl OAuthProvider {
    /// Short human-facing identifier: `"claude"` or `"codex"`.
    pub fn label(self) -> &'static str {
        match self {
            OAuthProvider::Claude => "claude",
            OAuthProvider::Codex => "codex",
        }
    }

    /// Key under which this provider's block is stored in Ocean's auth JSON:
    /// `"claude-code"` or `"openai-codex"`.
    pub fn auth_json_key(self) -> &'static str {
        match self {
            OAuthProvider::Claude => "claude-code",
            OAuthProvider::Codex => "openai-codex",
        }
    }

    /// Parse the short identifier [`OAuthProvider::label`] produces.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "claude" => Some(OAuthProvider::Claude),
            "codex" => Some(OAuthProvider::Codex),
            _ => None,
        }
    }

    /// Every provider this crate can log in, in display order.
    pub const ALL: [OAuthProvider; 2] = [OAuthProvider::Claude, OAuthProvider::Codex];

    fn token_url_env(self) -> &'static str {
        match self {
            OAuthProvider::Claude => ENV_ANTHROPIC_TOKEN_URL,
            OAuthProvider::Codex => ENV_OPENAI_TOKEN_URL,
        }
    }
}

/// Revocable custody for one login attempt. Keep a clone until cancellation has
/// settled, even if the future awaiting publication is dropped or aborted.
#[derive(Clone, Default)]
pub struct PublicationFence {
    cancelled: Arc<AtomicBool>,
    published: Arc<AtomicBool>,
    publication: Arc<tokio::sync::Mutex<()>>,
}

impl PublicationFence {
    /// Revoke future publication immediately. A writer already inside custody
    /// may finish; call `settle` before reporting cancellation or removing auth.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether an admitted publication completed successfully. Read after
    /// settlement to distinguish cancellation from an already-completed login.
    pub fn published(&self) -> bool {
        self.published.load(Ordering::SeqCst)
    }

    /// Wait for any admitted writer, including an orphaned blocking worker.
    /// Cancellation of this wait is safe: the fence remains revoked and a
    /// subsequent owner can settle it again.
    pub async fn settle(&self) {
        let _guard = self.publication.lock().await;
    }

    /// Run credential persistence off the async executor. The blocking closure
    /// owns the guard, so aborting its async caller cannot release custody.
    pub async fn persist<T: Send + 'static>(
        &self,
        write: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let guard = Arc::clone(&self.publication).lock_owned().await;
        let cancelled = Arc::clone(&self.cancelled);
        let published = Arc::clone(&self.published);
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            if cancelled.load(Ordering::SeqCst) {
                bail!("credential publication cancelled");
            }
            let result = write();
            if result.is_ok() {
                published.store(true, Ordering::SeqCst);
            }
            result
        })
        .await
        .map_err(|_| anyhow!("credential storage worker failed"))?
    }
}

/// A login in progress: the callback server is bound and the authorize URL is
/// ready to open. Call [`LoginSession::finish`] to await the browser callback,
/// exchange the code, and persist the credential.
pub struct LoginSession {
    /// Full authorize URL — open this in the browser.
    pub authorize_url: String,
    /// `http://localhost:{port}/launch` — a short copy/paste target that 302s
    /// to [`LoginSession::authorize_url`] (survives TUI viewport truncation).
    pub launch_url: String,
    provider: OAuthProvider,
    server: CallbackServer,
    verifier: String,
    redirect_uri: String,
    auth_path: PathBuf,
    /// Token endpoint override (from the per-provider env var); `None` falls
    /// back to the provider's public default. Read once at [`begin`] time.
    token_url_override: Option<String>,
}

/// Result of a completed login.
#[derive(Debug)]
pub struct LoginOutcome {
    /// The provider that was logged in.
    pub provider: OAuthProvider,
    /// The auth file the credential block was written to.
    pub auth_file: PathBuf,
    /// Absolute expiry of the access token, in milliseconds since the Unix
    /// epoch (matches the `expires` field consumed by `ocean_providers`).
    pub expires_ms: i64,
    /// Account identifier carried by the token, when available
    /// (`account.uuid` for Claude, the JWT `chatgpt_account_id` for Codex).
    pub account_id: Option<String>,
}

/// What Ocean's auth file says about one provider's OAuth block. Never carries
/// a token: this is the shape a status route may hand to a browser.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OAuthBlockStatus {
    /// A `type:"oauth"` block with a non-empty access or refresh token exists.
    pub present: bool,
    /// The block carries a refresh token, so an expired access token renews at
    /// turn time without a new browser login.
    pub refreshable: bool,
    /// Access-token expiry in ms since the epoch, normalized from seconds when
    /// the file stored seconds. `None` when unknown.
    pub expires_ms: Option<i64>,
}

/// Read `provider`'s OAuth block status from the auth file. A missing file or
/// block is `present: false`, not an error; an unreadable or non-JSON file is.
pub fn oauth_block_status(
    provider: OAuthProvider,
    auth_file: Option<PathBuf>,
) -> Result<OAuthBlockStatus> {
    let path = resolve_auth_path(auth_file)?;
    let Some(block) = store::read_block(&path, provider.auth_json_key())? else {
        return Ok(OAuthBlockStatus::default());
    };
    let non_empty = |key: &str| {
        block
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    };
    let is_oauth = block.get("type").and_then(serde_json::Value::as_str) == Some("oauth");
    let refreshable = is_oauth && non_empty("refresh");
    let present = is_oauth && (non_empty("access") || refreshable);
    // `expires` is ms when large, seconds otherwise — the same rule
    // `ocean-providers` applies when it reads this block.
    let expires_ms = block
        .get("expires")
        .and_then(serde_json::Value::as_i64)
        .map(|raw| {
            if raw >= 1_000_000_000_000 {
                raw
            } else {
                raw.saturating_mul(1000)
            }
        });
    Ok(OAuthBlockStatus {
        present,
        refreshable,
        expires_ms: present.then_some(expires_ms).flatten(),
    })
}

/// Sign `provider` out by removing its block from the auth file, preserving
/// every other block. Returns whether a block was removed.
pub fn logout(provider: OAuthProvider, auth_file: Option<PathBuf>) -> Result<bool> {
    let path = resolve_auth_path(auth_file)?;
    store::remove_and_write(&path, provider.auth_json_key())
}

/// Resolve the Ocean auth file path: an explicit argument wins, otherwise the
/// configured environment location (`ocean_providers::ProviderEnv`).
fn resolve_auth_path(auth_file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = auth_file {
        return Ok(path);
    }
    ocean_providers::ProviderEnv::from_process()
        .auth_file
        .ok_or_else(|| anyhow!("no Ocean auth file configured (set OCEAN_AUTH_FILE)"))
}

/// Persist a plain API key for `provider_key` (e.g. `"glm"`, `"deepseek"`) into
/// Ocean's auth file as `{provider_key: {"api_key": key}}`. Returns the path
/// written.
///
/// `auth_file` overrides the configured location; `None` resolves the same way
/// the OAuth login flows do (`resolve_auth_path`: `OCEAN_AUTH_FILE`, then the
/// default config path). The key is trimmed and a blank key is rejected. The
/// write reuses [`store::merge_and_write`] — atomic (temp + rename, 0600) and
/// every unrelated provider block is preserved. Env vars always win over a
/// file key at resolve time; this fn only writes the file side.
pub fn store_api_key(provider_key: &str, key: &str, auth_file: Option<PathBuf>) -> Result<PathBuf> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        bail!("api key for {provider_key} is empty");
    }
    let path = resolve_auth_path(auth_file)?;
    let block = serde_json::json!({ "api_key": trimmed });
    store::merge_and_write(&path, provider_key, block)?;
    Ok(path)
}

/// Read the per-provider token-endpoint override from the environment, if any.
/// An empty value is treated as unset.
fn env_token_url(provider: OAuthProvider) -> Option<String> {
    std::env::var(provider.token_url_env())
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Bind the callback server and build the authorize URL. Does NOT open a
/// browser — the caller decides how to present [`LoginSession::authorize_url`]
/// / [`LoginSession::launch_url`].
pub async fn begin(provider: OAuthProvider, auth_file: Option<PathBuf>) -> Result<LoginSession> {
    let pkce_pair = pkce::generate();
    let state = pkce::generate_state();
    let auth_path = resolve_auth_path(auth_file)?;
    let token_url_override = env_token_url(provider);

    let spec = bind_spec(provider);
    let server = CallbackServer::bind(&spec, state.clone()).await?;
    let redirect_uri = server.redirect_uri.clone();
    let launch_url = server.launch_url.clone();
    let authorize_url = build_authorize_url(provider, &state, &pkce_pair.challenge, &redirect_uri);
    server.set_pending_url(authorize_url.clone());

    Ok(LoginSession {
        authorize_url,
        launch_url,
        provider,
        server,
        verifier: pkce_pair.verifier,
        redirect_uri,
        auth_path,
        token_url_override,
    })
}

impl LoginSession {
    /// Await the browser callback (300s timeout), exchange the authorization
    /// code for tokens, and persist the credential block. Consumes the session;
    /// the callback server shuts down on return.
    pub async fn finish(self) -> Result<LoginOutcome> {
        self.finish_with_publication(PublicationFence::default())
            .await
    }

    /// Complete a login under caller-retained publication custody. Cancellation
    /// must revoke this fence, stop the session task, and settle the fence.
    pub async fn finish_with_publication(
        mut self,
        publication: PublicationFence,
    ) -> Result<LoginOutcome> {
        let callback = match tokio::time::timeout(
            Duration::from_secs(FLOW_TIMEOUT_SECS),
            self.server.next_result(),
        )
        .await
        {
            Ok(resolved) => resolved?,
            Err(_elapsed) => {
                bail!("login timed out waiting for browser callback after {FLOW_TIMEOUT_SECS}s")
            }
        };
        // `/launch` is no longer active once the flow has resolved.
        self.server.clear_pending();

        let (code, state) = match callback {
            CallbackResult::Ok { code, state } => (code, state),
            CallbackResult::Err(message) => bail!("{message}"),
        };

        let token_url = self
            .token_url_override
            .as_deref()
            .unwrap_or_else(|| consts(self.provider).token_url);
        let token = exchange(
            self.provider,
            token_url,
            &code,
            &state,
            &self.redirect_uri,
            &self.verifier,
        )
        .await?;
        let block = build_block(self.provider, &token);
        let auth_path = self.auth_path.clone();
        let provider_key = self.provider.auth_json_key();
        publication
            .persist(move || store::merge_and_write(&auth_path, provider_key, block))
            .await?;

        Ok(LoginOutcome {
            provider: self.provider,
            auth_file: self.auth_path.clone(),
            expires_ms: token.expires_ms,
            account_id: token.account_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::store_api_key;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// RAII temp dir; cleaned up even on panic (mirrors `store::tests`).
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fresh() -> (TempDir, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ocean-oauth-store-api-key-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let auth = dir.join("auth.json");
        (TempDir(dir), auth)
    }

    #[cfg(unix)]
    fn mode_is_0600(path: &std::path::Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777 == 0o600)
            .unwrap_or(false)
    }

    #[test]
    fn writes_api_key_block_preserving_unrelated_keys() {
        let (_guard, auth) = fresh();
        // Seed an unrelated provider block AND a stale glm block — the stale
        // glm block must be fully replaced, the unrelated block preserved.
        std::fs::write(
            &auth,
            r#"{"deepseek":{"api_key":"sk-ds"},"glm":{"api_key":"OLD"}}"#,
        )
        .unwrap();

        let written = store_api_key("glm", "  sk-glm-new  ", Some(auth.clone())).unwrap();
        assert_eq!(written, auth);

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(
            v["deepseek"]["api_key"], "sk-ds",
            "unrelated block preserved"
        );
        assert_eq!(v["glm"]["api_key"], "sk-glm-new");
        assert!(
            v["glm"].get("api_key").and_then(Value::as_str) != Some("OLD"),
            "stale glm block must be replaced"
        );
        #[cfg(unix)]
        assert!(mode_is_0600(&auth), "expected 0600 on auth file");
    }

    #[test]
    fn voice_keys_preserve_agent_oauth_and_each_other() {
        let (_guard, auth) = fresh();
        std::fs::write(
            &auth,
            r#"{"claude-code":{"type":"oauth","access":"claude-token"},"openai-codex":{"type":"oauth","access":"codex-token"}}"#,
        )
        .unwrap();

        store_api_key("xai", "xai-voice-key", Some(auth.clone())).unwrap();
        store_api_key("openai-realtime", "openai-voice-key", Some(auth.clone())).unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(v["xai"]["api_key"], "xai-voice-key");
        assert_eq!(v["openai-realtime"]["api_key"], "openai-voice-key");
        assert_eq!(v["claude-code"]["access"], "claude-token");
        assert_eq!(v["openai-codex"]["access"], "codex-token");
        assert!(
            v.get("openai").is_none(),
            "voice save must not create agent OpenAI auth"
        );
        #[cfg(unix)]
        assert!(mode_is_0600(&auth), "expected 0600 on auth file");
    }

    #[test]
    fn rejects_empty_key_after_trim() {
        let (_guard, auth) = fresh();
        let err = store_api_key("glm", "   \n  ", Some(auth.clone())).unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "expected empty-key error, got: {err}"
        );
        // Nothing written — the temp file was never created.
        assert!(!auth.exists());
    }

    #[test]
    fn creates_file_and_parent_dirs_when_absent() {
        let (_guard, auth) = fresh();
        // Wipe the parent so the file's directory doesn't exist yet.
        let parent = auth.parent().unwrap();
        std::fs::remove_dir_all(parent).unwrap();
        assert!(!parent.exists());

        let written = store_api_key("deepseek", "sk-ds", Some(auth.clone())).unwrap();
        assert_eq!(written, auth);
        assert!(auth.exists());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(v["deepseek"]["api_key"], "sk-ds");
        #[cfg(unix)]
        assert!(mode_is_0600(&auth));
    }
}

#[cfg(test)]
mod status_tests {
    use super::{logout, oauth_block_status, OAuthProvider};

    fn auth_file(body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ocean-oauth-status-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn status_reads_presence_refreshability_and_normalized_expiry() {
        let (dir, path) = auth_file(
            r#"{"claude-code":{"type":"oauth","access":"a","refresh":"r","expires":1700000000},
                "openai-codex":{"type":"oauth","access":"","refresh":""},
                "deepseek":{"api_key":"k"}}"#,
        );
        let claude = oauth_block_status(OAuthProvider::Claude, Some(path.clone())).unwrap();
        assert!(claude.present && claude.refreshable);
        assert_eq!(claude.expires_ms, Some(1_700_000_000_000));
        let codex = oauth_block_status(OAuthProvider::Codex, Some(path.clone())).unwrap();
        assert!(!codex.present);
        assert_eq!(codex.expires_ms, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_is_signed_out_and_logout_preserves_other_blocks() {
        let missing = std::env::temp_dir().join("ocean-oauth-status-missing/auth.json");
        assert!(
            !oauth_block_status(OAuthProvider::Claude, Some(missing.clone()))
                .unwrap()
                .present
        );
        assert!(!logout(OAuthProvider::Claude, Some(missing.clone())).unwrap());
        assert!(!missing.exists(), "logout must not create an auth file");

        let (dir, path) = auth_file(
            r#"{"claude-code":{"type":"oauth","access":"a"},"deepseek":{"api_key":"k"},"x":1}"#,
        );
        assert!(logout(OAuthProvider::Claude, Some(path.clone())).unwrap());
        let root: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(root.get("claude-code").is_none());
        assert_eq!(root["deepseek"]["api_key"], "k");
        assert!(!logout(OAuthProvider::Claude, Some(path.clone())).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn labels_round_trip() {
        for provider in OAuthProvider::ALL {
            assert_eq!(OAuthProvider::from_label(provider.label()), Some(provider));
        }
        assert_eq!(OAuthProvider::from_label("gemini"), None);
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use std::future::Future;

    #[tokio::test]
    async fn aborting_waiter_keeps_blocking_publication_in_custody() {
        let fence = PublicationFence::default();
        let writer = fence.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let published = Arc::new(AtomicBool::new(false));
        let written = published.clone();
        let task = tokio::spawn(async move {
            writer
                .persist(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    written.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .await
        });
        entered_rx.await.unwrap();
        fence.cancel();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let settle = fence.settle();
        tokio::pin!(settle);
        // Poll explicitly so this proves an active custody wait, not scheduling.
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(settle.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(!published.load(Ordering::SeqCst));
        release_tx.send(()).unwrap();
        settle.await;
        assert!(published.load(Ordering::SeqCst));
        assert!(fence
            .persist(|| -> Result<()> { panic!("revoked writer must never execute") })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn queued_publication_is_refused_after_revocation() {
        let fence = PublicationFence::default();
        let guard = fence.publication.lock().await;
        let write = fence.persist(|| -> Result<()> { panic!("queued revoked writer executed") });
        tokio::pin!(write);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(write.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        fence.cancel();
        drop(guard);
        assert!(write.await.is_err());
        fence.settle().await;
    }
}
