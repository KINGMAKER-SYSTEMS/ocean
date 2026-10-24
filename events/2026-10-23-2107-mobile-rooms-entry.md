time: [21:07] [23-10-26] America/New_York
agent: [Codex desktop]
worktree: [codex/mobile-rooms-entry]
type: [refactor]
area: [frontend] [design] [testing]

Removed the blank mobile Rooms entry. At the existing 650px breakpoint, no selected room and no pending invitation renders the room list full-width in normal flow; the unused center, mobile nav and backdrop are hidden. Selecting a room restores the existing conversation/drawer layout; pending invitation confirmation remains visible. No automatic room choice, join, post or permission change.

Validation: 10 compact-layout tests and 3 invitation tests passed; WASM Clippy with denied warnings, format, whitespace and locked release Trunk build passed. Actual daemon-backed browser at 390px verified initial list, selecting an existing room, returning via Back, staging a synthetic invitation without redemption, and cancelling back to the list. Desktop viewport restored after verification. Screenshot: /tmp/ocean-mobile-entry.png. Surface contract updated; parent and child indexes unchanged. Production remains on ec61e413; this local result is not deployment or coworker acceptance.
