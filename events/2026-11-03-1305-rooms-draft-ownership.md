time: [13:05] [03-11-26] America/New_York
agent: Codex [GPT-6]
worktree: fix/rooms-draft-ownership; base 4996b5ecd2b88c037ff80571a30f14e7cba91e95
type: bug report
area: frontend

Room composer text now follows daemon origin and room key. Switching rooms or
losing the selected identity keeps each unsent draft; pending send confirmation
is stored per room and clears only that room's unchanged text. Added A/B roundtrip,
origin/key isolation, identity-loss, pending-send navigation and one-time clear
coverage. The Surface Rooms contract now records the draft ownership rule and
focused test command.

Validation:

- `cargo fmt --manifest-path apps/ocean-surface/crates/ocean-surface-ui/Cargo.toml -- --check` passed.
- All 11 composer tests passed with `cargo test --manifest-path apps/ocean-surface/crates/ocean-surface-ui/Cargo.toml rooms_workspace::composer::tests`.
- The pending-send regression failed with an intentionally unscoped clear predicate, then passed with the room guard restored.
- WASM `cargo check --target wasm32-unknown-unknown` passed.
- Release `trunk build --release` passed into `/tmp/ocean-room-draft-dist`; the output WASM magic is `0061736d`.
- `git diff --check` passed. Build targets and output stayed under `/tmp`.

No room messages, provider calls, deployment, secret access, or runtime control.
Open PRs #105 (consent) and #179 (daemon web harness) were checked and are out
of scope. Other active room worktrees were inspected; none owned this composer
change. Root devlog and child indexes are unchanged because ownership and tree
structure did not change.
