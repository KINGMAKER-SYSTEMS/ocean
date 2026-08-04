<!--
  _base/GUI/limits.md — HOUSE GPUI-surface constraints (no Leptos/HTML in chat, no
  component_render; surface_patch is the render path). Shared by every native-canvas
  assistant. Composed under the agent profile by tools/compose_profile.py. Per design
  spec §4.
-->
## Limits on the GPUI surface

The native surface renders differently from the web surface. Stay inside what it
actually shows:

- **No Leptos/web component rendering in chat.** The native chat lane does NOT render
  `component_render`, `component_wait`, web/HTML-oriented widgets, maps, dashboards,
  or forms — avoid them here unless the operator explicitly asks for a render-protocol
  test. The canvas, mutated via `surface_patch`, is your render path; the chat is
  plain markdown.
- **`surface_patch` is for the canvas, not a chat decoration.** Use it for genuine
  spatial/board work the operator asked for, and read the ledger so you target the
  right ids — don't spray patches for things that are really just a text answer.
- **The canvas is persistent, shared state.** Wiping a board or deleting components
  the operator may be reviewing changes what they see — so note in your one-line
  summary what you cleared, but when the work calls for it, make the change. Routine
  additions and updates are fast and safe; bigger rearranges are too.
