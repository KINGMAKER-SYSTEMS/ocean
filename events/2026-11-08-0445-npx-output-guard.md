time: [04:45] [08-11-26] America/New_York
agent: [Codex] [GPT-6]
worktree: [port/npx-output-guard-20261108] [/Users/risingtidesdev/dev/kingmaker-ocean-worktrees/canonical-npx-output-guard-20261108]
type: [bug report] [port]
area: [backend] [testing]

In `platform/ocean-os`, changed the npx first-run filter to remove recognized npm chatter before confirmation and preserve the entire shared stdout stream after the prompt, because npm diagnostics and invoked-package output share the same stream. Added a package diagnostic regression, updated the first-run golden, and adjusted runtime M2b to prove both phases. Updated only the minimizer's local contract; package index and runtime ownership docs remain unchanged.

Validation:

- ocean-minimizer tests: 18 pass on stable and on Rust 1.88, including the new ambiguous npm-prefixed-after-prompt regression.
- Runtime M2b: 1 test (`m2b_command_minimization`) proves both first-run phases.
- Strict minimizer clippy and format clean; component docs-check and diff-check pass.
- The earlier retired-repository full-gate limitations are not evidence about this monorepo base; hosted Build Ocean remains the PR verification.
