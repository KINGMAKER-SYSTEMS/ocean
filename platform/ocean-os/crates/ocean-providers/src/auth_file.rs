//! Shared custody for cooperating Ocean credential writers.
//! Hold the lease only for fresh read/merge/publication, never for network I/O.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

/// Maximum contention wait, including both the process mutex and file lock.
pub const AUTH_FILE_LOCK_WAIT: Duration = Duration::from_secs(5);

fn process_auth_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Owns one auth path until publication finishes. Drop releases the file lock
/// before the process mutex. The shared lock file is deliberately never removed.
pub struct AuthFileGuard {
    _file: File,
    _process: MutexGuard<'static, ()>,
    auth_file: PathBuf,
}

/// Acquire both custody locks or fail without reading or publishing credentials.
/// All cooperating writers must use the same configured auth-file location.
pub fn lock_auth_file(auth_file: &Path) -> io::Result<AuthFileGuard> {
    lock_with_wait(auth_file, AUTH_FILE_LOCK_WAIT)
}

fn lock_with_wait(auth_file: &Path, wait: Duration) -> io::Result<AuthFileGuard> {
    let deadline = Instant::now() + wait;
    let process = loop {
        match process_auth_lock().try_lock() {
            Ok(guard) => break guard,
            Err(TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(TryLockError::WouldBlock) => wait_for_custody(deadline)?,
        }
    };
    // Bind relative paths now; a later cwd change cannot redirect publication.
    let auth_file = if auth_file.is_absolute() {
        auth_file.to_path_buf()
    } else {
        std::env::current_dir()?.join(auth_file)
    };
    let parent = auth_file.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(parent.join(".auth.json.lock"))?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => break,
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                wait_for_custody(deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(AuthFileGuard {
        _file: file,
        _process: process,
        auth_file,
    })
}

fn wait_for_custody(deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "auth file custody timed out; no write attempted",
        ));
    }
    std::thread::sleep(remaining.min(Duration::from_millis(10)));
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "auth file custody timed out; no write attempted",
        ))
    } else {
        Ok(())
    }
}

/// Fixed diagnostics: credential bytes and arbitrary underlying error text are
/// never carried by a publication error.
#[derive(Debug)]
pub struct AuthFileWriteError {
    published: bool,
    kind: io::ErrorKind,
}

impl AuthFileWriteError {
    /// True means rename succeeded, but directory durability was not confirmed.
    /// Do not roll back a stale snapshot or claim the previous bytes remain.
    pub fn published(&self) -> bool {
        self.published
    }

    pub fn kind(&self) -> io::ErrorKind {
        self.kind
    }

    fn before_publication(error: io::Error) -> Self {
        Self {
            published: false,
            kind: error.kind(),
        }
    }
}

impl std::fmt::Display for AuthFileWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.published {
            f.write_str("auth file published; directory durability unconfirmed")
        } else {
            f.write_str("auth file publication failed before replacement")
        }
    }
}

impl std::error::Error for AuthFileWriteError {}

impl AuthFileGuard {
    /// Read only after acquiring custody, so a merge sees the latest commit.
    pub fn read(&self) -> io::Result<Vec<u8>> {
        std::fs::read(&self.auth_file)
    }

    /// Publish a fresh merge through a unique create-new sibling. Unix mode0600
    /// is set at creation, before any bytes. Pre-rename failures preserve the
    /// old file; post-rename directory-sync failure is explicitly distinguished.
    pub fn publish(&self, bytes: &[u8]) -> Result<(), AuthFileWriteError> {
        self.publish_with(bytes, |_| Ok(()), sync_parent)
    }

    fn publish_with(
        &self,
        bytes: &[u8],
        before_rename: impl FnOnce(&Path) -> io::Result<()>,
        sync_directory: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), AuthFileWriteError> {
        let mut temp =
            create_private_temp(&self.auth_file).map_err(AuthFileWriteError::before_publication)?;
        let result = (|| {
            let file = temp.file.as_mut().expect("new tempfile has its handle");
            file.write_all(bytes)?;
            file.sync_all()
        })();
        // Close before cleanup/rename, including on Windows failure paths.
        drop(temp.file.take());
        result.map_err(AuthFileWriteError::before_publication)?;
        before_rename(&temp.path).map_err(AuthFileWriteError::before_publication)?;
        std::fs::rename(&temp.path, &self.auth_file)
            .map_err(AuthFileWriteError::before_publication)?;
        sync_directory(&self.auth_file).map_err(|error| AuthFileWriteError {
            published: true,
            kind: error.kind(),
        })
    }
}

struct PrivateTemp {
    path: PathBuf,
    file: Option<File>,
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        drop(self.file.take());
        // Bounded best-effort cleanup of only the file this writer created.
        let _ = std::fs::remove_file(&self.path);
    }
}

fn create_private_temp(auth_file: &Path) -> io::Result<PrivateTemp> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = auth_file.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = parent.join(format!(".auth.json.tmp-{}-{n}-{nanos}", std::process::id()));
        match create_private_temp_at(path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            result => return result,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "auth tempfile allocation exhausted",
    ))
}

fn create_private_temp_at(path: PathBuf) -> io::Result<PrivateTemp> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    Ok(PrivateTemp {
        path,
        file: Some(file),
    })
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    // Directory fsync is not portable. Preserve the existing non-Unix boundary.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_temps(parent: &Path) -> bool {
        std::fs::read_dir(parent).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".auth.json.tmp-")
        })
    }

    #[cfg(unix)]
    fn private(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777 == 0o600
    }

    #[test]
    fn auth_file_process_contention_times_out_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let guard = lock_auth_file(&path).unwrap();
        guard.publish(b"{}").unwrap();
        let before = guard.read().unwrap();
        let result = std::thread::spawn(move || {
            lock_with_wait(&path, Duration::from_millis(40))
                .err()
                .map(|error| error.kind())
        })
        .join()
        .unwrap();
        assert_eq!(result, Some(io::ErrorKind::TimedOut));
        assert!(guard.read().unwrap() == before);
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn auth_file_lock_open_failure_never_falls_back_to_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::create_dir(dir.path().join(".auth.json.lock")).unwrap();
        assert!(lock_auth_file(&path).is_err());
        assert!(std::fs::read(&path).unwrap() == b"{}");
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn auth_file_collision_never_truncates_or_removes_existing_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".auth.json.tmp-collision");
        std::fs::write(&path, b"existing fixture").unwrap();
        let error = create_private_temp_at(path.clone()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(std::fs::read(&path).unwrap() == b"existing fixture");
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_new_temp_is_private_before_any_content() {
        let dir = tempfile::tempdir().unwrap();
        let temp = create_private_temp(&dir.path().join("auth.json")).unwrap();
        assert!(private(&temp.path));
        assert!(std::fs::metadata(&temp.path).unwrap().len() == 0);
        drop(temp);
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn auth_file_prepublication_failure_preserves_bytes_mode_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let guard = lock_auth_file(&path).unwrap();
        guard.publish(b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let before_mode = std::fs::metadata(&path).unwrap().permissions();
        let error = guard
            .publish_with(
                b"{\"synthetic\":true}",
                |temp| {
                    #[cfg(unix)]
                    assert!(private(temp));
                    #[cfg(not(unix))]
                    let _ = temp;
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                },
                sync_parent,
            )
            .unwrap_err();
        assert!(!error.published());
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(guard.read().unwrap() == b"{}");
        assert!(std::fs::metadata(&path).unwrap().permissions() == before_mode);
        assert!(no_temps(dir.path()));
        assert!(dir.path().join(".auth.json.lock").exists());
    }

    #[test]
    fn auth_file_postrename_failure_reports_publication_without_stale_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let guard = lock_auth_file(&path).unwrap();
        guard.publish(b"{}").unwrap();
        let error = guard
            .publish_with(
                b"{\"synthetic\":true}",
                |_| Ok(()),
                |_| Err(io::Error::from(io::ErrorKind::Other)),
            )
            .unwrap_err();
        assert!(error.published());
        assert!(guard.read().unwrap() == b"{\"synthetic\":true}");
        #[cfg(unix)]
        assert!(private(&path));
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn auth_file_cross_process_contention_and_release() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let guard = lock_auth_file(&path).unwrap();
        guard.publish(b"{}").unwrap();
        let child = |mode: &str| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auth_file::tests::auth_file_lock_subprocess_child",
                    "--test-threads=1",
                ])
                .env("OCEAN_CUSTODY_TEST_PATH", &path)
                .env("OCEAN_CUSTODY_TEST_MODE", mode)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        assert!(child("blocked"));
        assert!(guard.read().unwrap() == b"{}");
        drop(guard);
        assert!(child("released"));
        assert!(std::fs::read(&path).unwrap() == b"{\"child\":true}");
        assert!(dir.path().join(".auth.json.lock").exists());
    }

    #[test]
    fn auth_file_lock_subprocess_child() {
        let Some(path) = std::env::var_os("OCEAN_CUSTODY_TEST_PATH") else {
            return;
        };
        let path = PathBuf::from(path);
        if std::env::var("OCEAN_CUSTODY_TEST_MODE").unwrap() == "blocked" {
            let error = lock_with_wait(&path, Duration::from_millis(100))
                .err()
                .expect("child must not obtain held custody");
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        } else {
            let guard = lock_auth_file(&path).unwrap();
            guard.publish(b"{\"child\":true}").unwrap();
        }
    }
}
