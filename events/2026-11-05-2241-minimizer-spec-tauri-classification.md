time:  [22:41] [05-11-26]
agent: [zcode cli] [glm-5.3]
worktree: [kmf/fq-k24] [~/dev/kingmaker-ocean-worktrees/kmf-fq-k24-r2239]
type:  [bug report]
area:  [writing]

Factory FQ-K24 follow-on to #179, docs-only. The minimizer M2 design spec
(`platform/ocean-os/docs/specs/2026-07-16-ocean-minimizer-command-capture-runtime-integration-design.md`)
still said at its §3 policy-table note that `surface-tauri` "retains the
daemon's current CLI compatibility fallback ... until cross-repository policy
changes". That condition has fired: #179 (squash `e6dc81c6`) maps
`Some("surface-tauri")` to `HarnessProfile::Web` in
`platform/ocean-os/crates/ocean-daemon/src/harness_profile.rs:68`, pinned by
`surface_tauri_desktop_gets_the_web_profile_not_the_cli_fallback` at :156.
Since M2 is "implementation not started" (spec :5), an M2 implementer reading
§3 could mis-file `surface-tauri` into the fallback test row. Amended exactly
that one sentence to record the landed decision (PR #179, `e6dc81c6`) and that
the `surface-tauri` M2 gate follows the Web profile row, not the CLI fallback
row; :285 (unknown/missing clients retaining CLI fallback) stays as-is because
it remains true. Precedent: #179 itself refreshed the stale omp-port-map W0
row. Caveat: the spec carries an authoring-baseline stamp (`5b9e23a8`,
2026-07-16); the operator may drop this change if dated design specs are ruled
immutable (stated in the PR body). Validation: `cargo xtask docs-check` PASS
(30 packages, 164 active Markdown files, 191 local links) and
`git diff --check` clean from `platform/ocean-os` in the worktree.
