time: [22:00] [23-10-26] America/New_York
agent: Codex
worktree: codex/mobile-rooms-entry
type: refactor
area: frontend

Addressed PR #181's Codex review finding by moving selecting-state rail geometry and related visibility rules into the canonical Layout & depth compact media block. Verified declarations and all other stylesheet content unchanged apart from whitespace/location.

Validation: release Trunk build; nine compact tests; diff check. Rendered 320px and 390px rails fill their viewport; 1280px desktop retains the 252px rail. Opening a room keeps composer bottom at 690px in a 700px viewport; Back returns to the list. Screenshot /tmp/ocean-mobile-layout-review.png.

Devlog pass: root and Surface AGENTS.md/indexes intentionally unchanged because their existing canonical-layout rule already owns this correction. No merge or deployment; fresh review remains required.
