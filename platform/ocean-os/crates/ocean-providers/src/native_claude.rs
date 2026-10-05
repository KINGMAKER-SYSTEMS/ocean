//! Read-only native Claude login fallback. Never imports or refreshes credentials.
use std::{fs::File, io::Read, path::PathBuf};

use crate::{CredentialKind, CredentialSource, ProviderEnv, ResolvedCredential, SecretString};

const MAX_BYTES: u64 = 32 * 1024;

fn token(bytes: &[u8], now_secs: i64) -> Option<SecretString> {
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let oauth = json.get("claudeAiOauth")?;
    if oauth.get("expiresAt")?.as_i64()? / 1000 <= now_secs
        || !oauth
            .get("scopes")?
            .as_array()?
            .iter()
            .any(|scope| scope.as_str() == Some("user:inference"))
    {
        return None;
    }
    SecretString::new(oauth.get("accessToken")?.as_str()?)
}

pub(crate) fn resolve(env: &ProviderEnv) -> Option<ResolvedCredential> {
    let custom = env.get("CLAUDE_CONFIG_DIR");
    if custom.is_some_and(|directory| directory.trim().is_empty()) {
        return None;
    }
    #[cfg(target_os = "macos")]
    if custom.is_none()
        && env.get("HOME") == std::env::var("HOME").ok().as_deref()
        && env.get("HOME").is_some()
    {
        let mut command = std::process::Command::new("/usr/bin/security");
        command.args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ]);
        if let Some(secret) = bounded_output(command, std::time::Duration::from_secs(2))
            .and_then(|bytes| token(&bytes, crate::unix_epoch_secs()))
        {
            return Some(ResolvedCredential {
                secret,
                source: CredentialSource::ClaudeCodeKeychain,
                kind: CredentialKind::OAuthBearer,
            });
        }
    }
    // A custom CLI config selects only its own file; never borrow the default account.
    let directory = custom.map(PathBuf::from).or_else(|| {
        env.get("HOME")
            .map(|home| PathBuf::from(home).join(".claude"))
    })?;
    let path = directory.join(".credentials.json");
    let mut bytes = Vec::new();
    File::open(&path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(ResolvedCredential {
        secret: token(&bytes, crate::unix_epoch_secs())?,
        source: CredentialSource::ClaudeCodeCliAuthFile {
            path: path.display().to_string(),
        },
        kind: CredentialKind::OAuthBearer,
    })
}

#[cfg(any(target_os = "macos", test))]
fn bounded_output(
    mut command: std::process::Command,
    deadline: std::time::Duration,
) -> Option<Vec<u8>> {
    use std::{process::Stdio, sync::mpsc, time::Instant};
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    // The trusted security executable owns this pipe. At most MAX_BYTES+1 are retained.
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let start = Instant::now();
    let result = receiver.recv_timeout(deadline).ok().and_then(Result::ok);
    let bytes = match result {
        Some(bytes) if bytes.len() as u64 <= MAX_BYTES => bytes,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success().then_some(bytes),
            Ok(None) if start.elapsed() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(expires: i64, scopes: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"claudeAiOauth": {"accessToken":"synthetic-token", "expiresAt":expires,"scopes":scopes}})).unwrap()
    }

    #[test]
    fn native_token_requires_expiry_and_inference_scope() {
        assert!(token(
            &fixture(200_000, serde_json::json!(["user:inference"])),
            100
        )
        .is_some());
        assert!(token(
            &fixture(100_000, serde_json::json!(["user:inference"])),
            100
        )
        .is_none());
        assert!(token(&fixture(200_000, serde_json::json!(["user:profile"])), 100).is_none());
        assert!(token(b"{}", 100).is_none());
        assert!(token(b"invalid", 100).is_none());
        assert!(token(&vec![b' '; MAX_BYTES as usize + 1], 100).is_none());
    }

    #[test]
    fn custom_native_file_preserves_precedence_and_api_isolation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".credentials.json"),
            fixture(i64::MAX, serde_json::json!(["user:inference"])),
        )
        .unwrap();
        let mut env = ProviderEnv {
            vars: Default::default(),
            auth_file: None,
            codex_auth_file: None,
        };
        env.vars
            .insert("CLAUDE_CONFIG_DIR".into(), dir.path().display().to_string());
        let native = crate::resolve_credential(&env, &crate::ProviderId::ClaudeCode)
            .unwrap()
            .unwrap();
        assert!(matches!(
            native.source,
            CredentialSource::ClaudeCodeCliAuthFile { .. }
        ));
        assert!(
            crate::resolve_credential(&env, &crate::ProviderId::Anthropic)
                .unwrap()
                .is_none()
        );
        env.vars
            .insert("CLAUDE_CODE_ACCESS_TOKEN".into(), "explicit-token".into());
        let explicit = crate::resolve_credential(&env, &crate::ProviderId::ClaudeCode)
            .unwrap()
            .unwrap();
        assert!(matches!(explicit.source, CredentialSource::Env { .. }));
        env.vars.remove("CLAUDE_CODE_ACCESS_TOKEN");
        let ocean_path = dir.path().join("auth.json");
        env.auth_file = Some(ocean_path.clone());
        std::fs::write(&ocean_path, r#"{"claude-code":{"type":"oauth","access":"ocean-token","expires":9223372036854775807}}"#).unwrap();
        assert!(matches!(
            crate::resolve_credential(&env, &crate::ProviderId::ClaudeCode)
                .unwrap()
                .unwrap()
                .source,
            CredentialSource::OceanAuthFile { .. }
        ));
        std::fs::write(
            &ocean_path,
            r#"{"claude-code":{"type":"oauth","access":"expired","expires":1}}"#,
        )
        .unwrap();
        assert!(matches!(
            crate::resolve_credential(&env, &crate::ProviderId::ClaudeCode)
                .unwrap()
                .unwrap()
                .source,
            CredentialSource::ClaudeCodeCliAuthFile { .. }
        ));
        std::fs::write(&ocean_path, "invalid").unwrap();
        assert!(crate::resolve_credential(&env, &crate::ProviderId::ClaudeCode).is_err());
        env.vars.clear();
        env.vars.insert(
            "CLAUDE_CONFIG_DIR".into(),
            dir.path().join("missing").display().to_string(),
        );
        assert!(resolve(&env).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_rejects_failed_and_oversized_output() {
        let mut valid = std::process::Command::new("/usr/bin/printf");
        valid.arg("synthetic");
        assert_eq!(
            bounded_output(valid, std::time::Duration::from_secs(1)),
            Some(b"synthetic".to_vec())
        );
        let failed = std::process::Command::new("/usr/bin/false");
        assert!(bounded_output(failed, std::time::Duration::from_secs(1)).is_none());
        let mut oversized = std::process::Command::new("/usr/bin/printf");
        oversized.arg("%040000d").arg("0");
        assert!(bounded_output(oversized, std::time::Duration::from_secs(1)).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_deadline_is_bounded() {
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("10");
        let start = std::time::Instant::now();
        assert!(bounded_output(command, std::time::Duration::from_millis(30)).is_none());
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }
}
