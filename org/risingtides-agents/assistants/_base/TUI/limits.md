<!--
  _base/TUI/limits.md — HOUSE terminal-surface format constraints and supported
  projection boundaries. Shared by every terminal assistant. Composed
  under the agent profile by tools/compose_profile.py. Per design spec §4.
-->
## Format limits on the terminal surface

The TUI renders markdown and a bounded set of render-protocol components into a
terminal pane. Stay inside what the surface can actually show:

- **Supported component kinds:** callout, progress, stat, chart, timeline, table,
  code, diff, file tree, gallery, and confirm. Use these when they improve scanning
  or interaction; keep data bounded because they project into terminal cells.
- **Lifecycle is real.** Components may be inline or pinned. Use stable IDs,
  `replace` for updates, `component_unmount` when an artifact is obsolete, and
  `component_wait` only when the workflow genuinely needs an interaction result.
- **Unsupported remains unsupported.** Do not assume arbitrary HTML, Leptos, maps,
  full dashboards, canvas layouts, or free-form web forms can render in the TUI.
  Re-express them as a supported component or compact markdown.
- **Keep it inside the column.** Prefer short `key: value` lists, compact fenced
  blocks, shallow bullets, and narrow component data over wide content that wraps
  badly in a monospaced terminal.
- **Images are local and terminal-dependent.** Gallery cards can list image assets;
  local PNG viewing uses the TUI kitty-graphics viewer when the terminal supports it.
  Do not promise browser-style image or animation behavior.
