# Ocean PR #193 timeout overflow review repair

- worktree: `codex/kingmaker-ocean-m2c-port-20261109` (`8df70a06` plus this review repair)
- time: 2026-11-09 16:28 EST
- agent: Ocean coding agent
- type: defect repair / review
- area: `platform/ocean-os/crates/ocean-runtime`

A defect-first review found that gated direct-argv resolution formed its shared probe deadline with `Instant::now() + Duration::from_millis(timeout_ms)`. The tool schema accepts the complete JSON `u64` range, so a caller-provided `u64::MAX` timeout could overflow the instant addition and panic the runtime task before command execution.

The resolver now preserves one shared budget by subtracting elapsed time before each probe. This retains timeout and cancellation ownership without constructing an absolute instant. The integration regression executes a login-PATH shebang helper with `timeout_ms = u64::MAX`; it would panic before this repair.

Validation:

- `cargo test -p ocean-runtime --test m2b_command_minimization` — PASS
- `cargo test -p ocean-runtime` — PASS
- `cargo clippy -p ocean-runtime --all-targets -- -D warnings` — PASS
- `cargo check --workspace` — PASS (only the two existing `ocean-agent` dead-code warnings)
- `cargo fmt --all -- --check` — PASS
- `git diff --check` — PASS
