time: [21:14] [23-10-26] America/New_York
agent: Codex
worktree: codex/room-open-status; browser-recovery-ec61e413
type: bug report
area: frontend

A selected Room previously hid its loading and failure status behind an empty
center pane. Render the existing status as a polite live status using the
existing centered text style. An unselected pane remains empty. No new requests,
retries, authority, or controls. Updated Surface guidance and removed a stray
merge-conflict marker there; root contract and child index unchanged.

Validation: release Trunk WASM build; WASM Clippy with denied warnings; formatting
and diff checks; two existing open_catch_up tests. Desktop loading and 390px
failure rendering checked with a loopback-only delayed 404 fixture that refuses
mutations. Screenshot: /tmp/ocean-room-open-status.png. No production deployment.
