time: [00:42 UTC] [24-10-26]
agent: [claude code] [claude-fable-5-1]
worktree: [port/tauri-web-harness] [~/dev/kingmaker-ocean-public/.claude/worktrees/agent-a3f2d2e0750ca23b6]
type: [bug-report] [port]
area: [backend]

Ported legacy Risingtides-dev/ocean-os#525 ("fix(daemon): map surface-tauri to
the Web harness profile") onto the monorepo as an OPEN pull request that merges
only on the operator's yes; it is the "Decide the Tauri effective harness
classification" item `platform/ocean-os/ROADMAP.md` lists, carried over so the
decision can be taken in the canonical repo. Today Surface's Tauri shell sends
`client_type: "surface-tauri"` (`apps/ocean-surface/crates/ocean-surface-ui/src/daemon.rs`),
`ocean-agent` already gives it the web surface prompt (`system_prompt.rs`), but
`HarnessProfile::from_client_type` in
`platform/ocean-os/crates/ocean-daemon/src/harness_profile.rs` does not map
it, so Tauri turns run the CLI fallback with hashline-tagged reads and
`hashline_edit` on. The branch adds `surface-tauri` to the Web arm (the profile
`surface-web` and `surface-extension` already get, since all three host the one
shared Leptos/WASM Surface; Web differs from the CLI fallback only in
`hashline_edits: false`, artifact spill and code intelligence stay on), adds the
legacy test `surface_tauri_desktop_gets_the_web_profile_not_the_cli_fallback`,
adds `surface-tauri` to the table test, and moves the unknown-types list to the
abandoned GPUI desktop's `surface-gpui`. Docs follow the legacy PR where the
file exists here: `docs/ARCHITECTURE.md` (the Tauri paragraph and the
non-claims line), the daemon `AGENTS.md` `harness_profile.rs` contract, and the
`docs/specs/2026-07-03-omp-port-map.md` W0 row. The ROADMAP item is rewritten
to the proposed Web wording but stays an open `[ ]` item naming this PR as the
pending decision (merging it is the yes; remove the item then). Skipped because
they do not exist in the monorepo: `docs/contracts/README.md` and
`docs/contracts/session-wire.json` (no session-wire contract pin here, so the
`known` client-type list has nothing to update). Validation from
`platform/ocean-os` with a private `CARGO_TARGET_DIR` in the worktree (deleted
after): `cargo test -p ocean-daemon` 1075 passed, 0 failed (1074 on main plus
the new test); `cargo test -p ocean-daemon harness_profile` 5 passed;
`cargo fmt --check` clean; warn-mode `cargo clippy -p ocean-daemon --tests`
reports 4 pre-existing warnings in `room_attachments.rs`, `model_catalog.rs`
and `room_summary.rs` and none in `harness_profile.rs`; `cargo xtask docs-check`
PASS (30 packages, 164 Markdown files, 191 links). Mutation: unmapping
`surface-tauri` turns `known_client_types_map_table_driven` and the new test
red (the legacy's third red was its contract pin, absent here). The legacy PR
is closed with a pointer to this one; its branch is left in place.
