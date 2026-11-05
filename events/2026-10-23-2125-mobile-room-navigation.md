time: [21:25] [23-10-26] America/New_York
agent: Codex
worktree: codex/mobile-rooms-entry; browser-recovery-ec61e413
type: bug report
area: frontend

Mobile Rooms used two room-title bars and let the center flex child grow past
the viewport: at 390x844 the composer began below y=1900. Keep the center
shrinkable, use one conversation header with the existing Back action leading,
and return lost focus through the existing room-list helper. Compact folder
context remains in Room details so the room name has space. Loading and invite
confirmation retain fallback navigation. No added controls or runtime requests.

Validation: release Trunk bundle, WASM Clippy with denied warnings, formatting
and diff checks; ten compact tests. Rendered 390x844 center now ends at y=844,
with composer y=782..844 and transcript scrolling inside. Back restores focus
to the rail option. Surface contract updated; root and child index unchanged
because ownership and structure did not change. PR #181 follow-up; not deployed.
