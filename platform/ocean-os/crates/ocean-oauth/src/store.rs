//! Atomic read-merge-write of Ocean's auth JSON.
//!
//! The auth file is a single JSON object keyed by provider. Unrelated blocks
//! (deepseek, kimi, …) MUST be preserved. Every fresh read/merge/publication holds
//! the shared Providers custody lease; unique create-new temporary files are
//! restricted to 0600 at creation on Unix.

use std::path::Path;

use anyhow::{bail, Context, Result};
use ocean_providers::AuthFileGuard;
use serde_json::{json, Value};

/// Merge `block` under `key`, preserving every other key, and write atomically.
pub(crate) fn merge_and_write(auth_file: &Path, key: &str, block: Value) -> Result<()> {
    let guard = ocean_providers::lock_auth_file(auth_file).context("auth file custody failed")?;
    let mut root = read_root(&guard)?;
    let map = root
        .as_object_mut()
        .context("auth file root is not a JSON object")?;
    map.insert(key.to_string(), block);
    let serialized = serde_json::to_string_pretty(&root)?;
    guard.publish(serialized.as_bytes())?;
    Ok(())
}

/// Remove the block under `key`, preserving every other key. Returns whether a
/// block was present. A missing file is "nothing to remove", not an error, and
/// is left missing rather than created.
pub(crate) fn remove_and_write(auth_file: &Path, key: &str) -> Result<bool> {
    let guard = ocean_providers::lock_auth_file(auth_file).context("auth file custody failed")?;
    let mut root = read_root(&guard)?;
    let map = root
        .as_object_mut()
        .context("auth file root is not a JSON object")?;
    if map.remove(key).is_none() {
        return Ok(false);
    }
    let serialized = serde_json::to_string_pretty(&root)?;
    guard.publish(serialized.as_bytes())?;
    Ok(true)
}

/// Read one block without writing anything. `None` when the file or the key is
/// absent. Read after acquiring custody, so a status read never observes a
/// mid-merge snapshot.
pub(crate) fn read_block(auth_file: &Path, key: &str) -> Result<Option<Value>> {
    let guard = ocean_providers::lock_auth_file(auth_file).context("auth file custody failed")?;
    Ok(read_root(&guard)?.get(key).cloned())
}

/// Read the existing root object, or an empty object when the file is missing
/// or blank.
fn read_root(guard: &AuthFileGuard) -> Result<Value> {
    match guard.read() {
        Ok(bytes) => {
            if bytes.iter().all(|b| b.is_ascii_whitespace()) {
                return Ok(json!({}));
            }
            let value: Value =
                serde_json::from_slice(&bytes).context("auth file is not valid JSON")?;
            if !value.is_object() {
                bail!("auth file root is not a JSON object");
            }
            Ok(value)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(err) => Err(err).context("failed to read auth file"),
    }
}

#[cfg(test)]
mod tests {
    use super::merge_and_write;
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// RAII temp dir; cleaned up even on panic.
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fresh() -> (TempDir, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ocean-oauth-store-{}-{n}", std::process::id()));
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
    fn preserves_unrelated_keys_and_replaces_same_key_block() {
        let (_guard, auth) = fresh();
        std::fs::write(
            &auth,
            r#"{"deepseek":{"k":"v"},"claude-code":{"old":true}}"#,
        )
        .unwrap();
        merge_and_write(
            &auth,
            "claude-code",
            json!({"type":"oauth","access":"a","refresh":"r","expires":1}),
        )
        .unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        // Unrelated key preserved byte-for-value.
        assert_eq!(v["deepseek"]["k"], "v");
        // Same-key block fully replaced — no leftover "old".
        assert_eq!(v["claude-code"]["type"], "oauth");
        assert_eq!(v["claude-code"]["access"], "a");
        assert!(
            v["claude-code"].get("old").is_none(),
            "old block not replaced"
        );
    }

    #[test]
    fn creates_parent_dirs_and_file_when_absent() {
        let (_guard, auth) = fresh();
        // Remove the dir so the file's parent doesn't exist yet.
        let parent = auth.parent().unwrap();
        std::fs::remove_dir_all(parent).unwrap();
        assert!(!parent.exists());

        merge_and_write(
            &auth,
            "claude-code",
            json!({"type":"oauth","access":"a","refresh":"r","expires":7}),
        )
        .unwrap();

        assert!(auth.exists(), "auth file should have been created");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(v["claude-code"]["expires"], 7);
        #[cfg(unix)]
        assert!(
            mode_is_0600(&auth),
            "expected 0600 permissions on auth file"
        );
    }

    #[test]
    fn whitespace_only_file_is_treated_as_empty() {
        let (_guard, auth) = fresh();
        std::fs::write(&auth, "   \n  ").unwrap(); // blank -> treated as {}
        merge_and_write(
            &auth,
            "openai-codex",
            json!({"type":"oauth","access":"a","refresh":"r","expires":9}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(v["openai-codex"]["access"], "a");
    }

    #[test]
    fn invalid_json_errors_without_clobbering() {
        let (_guard, auth) = fresh();
        let garbage = "{not valid json";
        std::fs::write(&auth, garbage).unwrap();
        let err = merge_and_write(&auth, "claude-code", json!({"type":"oauth"})).unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "err: {err}");
        // Original bytes untouched.
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), garbage);
    }

    #[test]
    fn output_is_pretty_printed() {
        let (_guard, auth) = fresh();
        merge_and_write(
            &auth,
            "claude-code",
            json!({"type":"oauth","access":"a","refresh":"r","expires":1}),
        )
        .unwrap();
        let contents = std::fs::read_to_string(&auth).unwrap();
        assert!(
            contents.contains("\n  \"claude-code\""),
            "not pretty printed"
        );
    }

    #[test]
    fn concurrent_provider_writers_preserve_every_committed_block() {
        let (_guard, auth) = fresh();
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let auth = auth.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    merge_and_write(&auth, &format!("fixture-{index}"), json!({"present": true}))
                        .unwrap();
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let root: Value = serde_json::from_slice(&std::fs::read(&auth).unwrap()).unwrap();
        assert!(root.as_object().unwrap().len() == 8);
        assert!((0..8).all(|index| root[format!("fixture-{index}")]["present"] == true));
        #[cfg(unix)]
        assert!(mode_is_0600(&auth));
    }
}
