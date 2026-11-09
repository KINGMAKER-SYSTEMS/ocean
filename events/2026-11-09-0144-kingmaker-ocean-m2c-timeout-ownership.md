time: [01:44] [09-11-26] America/New_York
agent: [Codex] [GPT-6]
worktree: [codex/kingmaker-ocean-m2c-port-20261109] [/Users/risingtidesdev/dev/.worktrees/kingmaker-ocean-m2c-port-20261109]
type: [bug report] [refactor]
area: [backend] [testing]

Bounded M2c login-PATH probes to one caller timeout budget shared with direct execution. Each probe now uses the BashTool process-group guard and kill-on-drop behavior; timeout and cancellation clean up ordinary descendants. Added regressions for a hanging login profile and cancellation while its profile child is active. Updated the runtime contract to document timeout and cleanup ownership.

Validation:

- Red/green: the old resolver took 4.28s to fail the profile timeout bound with `timeout_ms: 200`; the updated resolver passes the bound and verifies the marker is never written.
- Focused M2b command-minimization integration test passed with timeout and cancellation cleanup regressions.
- `cargo test -p ocean-runtime`: passed, including 148 unit tests and all integration suites.
- `cargo clippy -p ocean-runtime --all-targets -- -D warnings`: passed.
- `cargo check --workspace`: passed; existing `ocean-agent` dead-code warnings remain.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Residual platform boundary: Unix process groups kill ordinary descendants. Non-Unix builds retain direct-child cleanup only, matching the existing runtime contract.
