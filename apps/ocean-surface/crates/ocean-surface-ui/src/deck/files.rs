//! Workspace file-tree helpers — the session-cwd tree contract shared by the
//! workspace pane (crate::workspace) and transcript file rows
//! (crate::components). Pure and unit-testable without WASM.

use crate::daemon::FsDirEntry;

// ---------------------------------------------------------------------------
// Pure helpers — unit-testable without WASM
// ---------------------------------------------------------------------------

/// Sort entries: directories first (git repos sort above non-git dirs),
/// then alphabetically within each group.
pub(crate) fn sort_entries(entries: &mut [FsDirEntry]) {
    entries.sort_by(|a, b| {
        // Repos surface first; the daemon populates `is_repo` (`git` is a
        // legacy field it never sends).
        let a_repo = a.is_repo;
        let b_repo = b.is_repo;
        b_repo.cmp(&a_repo).then_with(|| a.name.cmp(&b.name))
    });
}

/// Given a PathEvent path, return Some(parent_dir) for a re-list target.
pub(crate) fn refresh_target(root: &str, event_path: &str, _kind: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let event_path = event_path.trim_end_matches('/');
    if event_path == root {
        return None;
    }
    if let Some(slash) = event_path.rfind('/') {
        let parent = &event_path[..slash];
        if parent == root || parent.starts_with(root) {
            return Some(parent.to_string());
        }
    }
    None
}

/// Extract basename from a path.
pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Map a session cwd to a listable file-tree root. `None` for roots the
/// trees must not follow: an unset cwd (`""`), the filesystem root (`/`),
/// and the projectless-chat pin ([`crate::daemon::CHAT_WORKSPACE_ROOT`],
/// `/tmp`). The daemon denies fs listing outside `$HOME`, so following the
/// chat pin after "New chat" would strand an "access denied: /tmp is outside
/// home directory" error in the tree instead of the clean empty state
/// (QA-007).
pub(crate) fn browsable_root(cwd: &str) -> Option<&str> {
    let trimmed = cwd.trim();
    let normalized = if trimmed.len() > 1 {
        trimmed.trim_end_matches('/')
    } else {
        trimmed
    };
    if normalized.is_empty()
        || normalized == "/"
        || normalized == crate::daemon::CHAT_WORKSPACE_ROOT
    {
        None
    } else {
        Some(trimmed)
    }
}

/// True when a filename matches a secret-bearing pattern that the surface
/// file trees hide by default (the way editors treat dotenv files): dotenv
/// files (`.env`, `.env.*`), key material (`*.pem`, `*.key`), and SSH
/// identities (`id_rsa*`). Deliberately narrow — ordinary dotfiles people
/// navigate to (`.gitignore`, `.github`, …) stay visible (QA-004).
pub(crate) fn is_secret_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.starts_with("id_rsa")
}
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, git: bool, is_repo: bool) -> FsDirEntry {
        FsDirEntry {
            name: name.to_string(),
            path: format!("/root/{}", name),
            git,
            is_repo,
            git_branch: if git { Some("main".into()) } else { None },
        }
    }

    #[test]
    fn sort_puts_git_repos_first() {
        let mut entries = vec![
            entry("zzz", false, false),
            entry("aaa", false, false),
            entry("bbb", true, true),
            entry("ccc", true, true),
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["bbb", "ccc", "aaa", "zzz"]);
    }

    #[test]
    fn sort_stable_for_same_type() {
        let mut entries = vec![
            entry("delta", false, false),
            entry("alpha", false, false),
            entry("charlie", true, true),
            entry("bravo", true, true),
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["bravo", "charlie", "alpha", "delta"]);
    }

    #[test]
    fn refresh_target_root_event_returns_none() {
        let root = "/home/user/project";
        assert_eq!(refresh_target(root, root, "modified"), None);
    }

    #[test]
    fn refresh_target_file_in_subdir() {
        let root = "/home/user/project";
        assert_eq!(
            refresh_target(root, "/home/user/project/src/main.rs", "modified"),
            Some("/home/user/project/src".into())
        );
    }

    #[test]
    fn refresh_target_dir_event() {
        let root = "/home/user/project";
        assert_eq!(
            refresh_target(root, "/home/user/project/src/components", "created"),
            Some("/home/user/project/src".into())
        );
    }

    #[test]
    fn refresh_target_nested() {
        let root = "/root";
        assert_eq!(
            refresh_target(root, "/root/a/b/c/d/file.txt", "modified"),
            Some("/root/a/b/c/d".into())
        );
    }

    #[test]
    fn basename_simple() {
        assert_eq!(basename("/foo/bar/baz.txt"), "baz.txt");
        assert_eq!(basename("just_a_file"), "just_a_file");
        assert_eq!(basename("/root/"), "");
    }

    #[test]
    fn browsable_root_rejects_unset_and_fs_root() {
        assert_eq!(browsable_root(""), None);
        assert_eq!(browsable_root("   "), None);
        assert_eq!(browsable_root("/"), None);
    }

    #[test]
    fn browsable_root_rejects_chat_workspace_pin() {
        // "New chat" pins the session cwd to CHAT_WORKSPACE_ROOT (/tmp); the
        // daemon denies listing it, so the trees must not follow it (QA-007).
        assert_eq!(browsable_root(crate::daemon::CHAT_WORKSPACE_ROOT), None);
        assert_eq!(browsable_root("/tmp/"), None);
    }

    #[test]
    fn browsable_root_passes_real_project_roots() {
        assert_eq!(
            browsable_root("/Users/dev/project"),
            Some("/Users/dev/project")
        );
        // /tmp is only rejected exactly — subdirs of it are still listable
        // roots as far as this helper is concerned (the daemon still gates).
        assert_eq!(browsable_root("/tmp/scratch"), Some("/tmp/scratch"));
    }

    #[test]
    fn secret_files_hidden() {
        assert!(is_secret_file(".env"));
        assert!(is_secret_file(".env.local"));
        assert!(is_secret_file(".env.production"));
        assert!(is_secret_file(".ENV"));
        assert!(is_secret_file("server.pem"));
        assert!(is_secret_file("private.key"));
        assert!(is_secret_file("id_rsa"));
        assert!(is_secret_file("id_rsa.pub"));
    }

    #[test]
    fn ordinary_dotfiles_stay_visible() {
        assert!(!is_secret_file(".gitignore"));
        assert!(!is_secret_file(".github"));
        assert!(!is_secret_file(".envrc")); // direnv config, not a dotenv file
        assert!(!is_secret_file("environment.rs"));
        assert!(!is_secret_file("keyboard.rs"));
        assert!(!is_secret_file("monkey.txt"));
        assert!(!is_secret_file("Cargo.toml"));
    }
}
