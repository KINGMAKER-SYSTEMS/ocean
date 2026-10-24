time: [22:45] [10-22-26]
agent: [claude code] [claude-opus-5-5]
worktree: [port/os-536-observatory-g5-extractor] [.claude/worktrees/agent-a540f587495459fce]
type: [testing] [port]
area: [testing] [backend]

Ported Risingtides-dev/ocean-os#536 (factory FQ-005A, test-only). The extractor-level `observatory_auth_rejects_every_credential_failure_as_401` in `platform/ocean-os/crates/ocean-daemon/src/observatory_auth.rs` asserted only the 401 status; every credential-failure case (malformed `Authorization-Observer` cookie, non-Bearer header, missing credential, wrong-instance token) now goes through `IntoResponse` via a new `assert_unauthorized_response` helper that pins the Gate 1 manifest §7.4 no-store header set, the exact §7.1 body, and `ObservatoryUnauthorized::for_request`'s no-`ObservatoryServices` fallback (cursor 0, instance id from the auth state), a branch the router-level test never reaches. No production change. Applied cleanly apart from the legacy events.md hunk. Validation: `cargo test -p ocean-daemon observatory_auth` 7 passed; `cargo fmt --check -p ocean-daemon` clean. `cargo clippy -p ocean-daemon --all-targets -- -D warnings` fails under local clippy 0.1.97 on pre-existing lints in untouched code (ocean-oauth `too_many_arguments`, ocean-agent `map_or`, daemon `room_attachments.rs`, `room_summary.rs`, `model_catalog.rs`); nothing in `observatory_auth.rs`.
