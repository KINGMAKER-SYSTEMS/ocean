time: [21:52] [23-10-26] America/New_York
agent: Codex
worktree: codex/room-drawer-focus
type: bug report
area: frontend

Opening the compact room list now focuses its selected/first room instead of Add room. Deferred callbacks refuse disposed or superseded entry state. Nonempty refreshes preserve the listbox and stable room-id options; live name/mute projections update without replacing focused rows.

Validation: release Trunk build, WASM Clippy with denied warnings, format and diff checks, 28 room-list tests. Browser at 390x700 verified first/selected focus, Arrow/Home/End without selection, Escape return focus, and Create/Join input focus. A read-only localhost fixture changed a room name, mute state and timestamp during polling; focused row survived and label/styling updated. No room creation, invitation redemption or message send. Independent review, merge and deployment remain pending.

Devlog: Surface AGENTS.md updated; root/index unchanged because ownership and hierarchy did not change.
