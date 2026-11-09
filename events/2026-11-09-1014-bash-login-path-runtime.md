time: [10:14] [09-11-26] America/New_York
agent: [Codex] [GPT-6]
worktree: [codex/kingmaker-ocean-m2c-port-20261109] [/Users/risingtidesdev/dev/.worktrees/kingmaker-ocean-m2c-port-20261109]
type: [bug report] [refactor]
area: [backend] [testing]

Direct argv execution now receives the agreeing login-shell probes' post-profile PATH, allowing shebang helpers such as `/usr/bin/env` targets to resolve. Relative and empty PATH entries are normalized against the profile cwd because the direct child runs from the turn cwd; if that translation cannot be encoded faithfully, resolution fails closed. Added regression coverage for login-only helpers, profile-cwd relative and empty PATH, and colon-containing profile cwd. Updated the runtime contract.

Validation:

- `cargo test -p ocean-runtime --test m2b_command_minimization`: passed.
- `cargo test -p ocean-runtime`: passed.
- `cargo clippy -p ocean-runtime --all-targets -- -D warnings`: passed.
- `cargo check --workspace`: passed; existing `ocean-agent` dead-code warnings remain.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

This entry accompanies the PR #193 follow-up. Merge and deployment remain separate gates.
