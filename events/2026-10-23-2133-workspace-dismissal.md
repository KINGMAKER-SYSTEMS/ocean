time: [21:33] [23-10-26] America/New_York
agent: Codex
worktree: codex/workspace-dismissal; browser-recovery-ec61e413
type: bug report
area: frontend

The compact workspace could not dismiss on Escape or outside click, and its
collapsed off-screen controls remained in accessibility traversal. Add compact
backdrop dismissal and lowest-priority unhandled Escape dismissal, returning
focus to the existing header toggle. Keep collapsed content mounted but inert
and aria-hidden. The toggle now exposes its expanded/controls relationship.
Docked workspace and stored pane/tab state remain unchanged.

Validation: release Trunk bundle, WASM Clippy with denied warnings, formatting
and diff checks; one breakpoint/admission test plus thirteen Escape tests.
Rendered 390px checks verified both dismissal paths, focus return, collapsed
accessibility absence, and preservation of an unsent draft (cleared without
sending). At 1280px, Escape kept the dock open and the backdrop remained hidden.
Screenshot: /tmp/ocean-workspace-dismissed.png. Surface contract updated; root
and child index unchanged because ownership/structure are unchanged.
No deployment or runtime/provider changes.
