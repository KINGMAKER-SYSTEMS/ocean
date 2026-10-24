time: [20:17] [23-10-26]
agent: [claude code] [claude-fable-5-1]
worktree: [port/rooms-details-drawer-directory-picker] [~/dev/kingmaker-ocean-public/.claude/worktrees/agent-ab46f8262f286f2a0]
type: [feature-request] [port]
area: [frontend]

Hand-ported legacy Risingtides-dev/ocean-surface#240 ("Rooms: focus-first
workspace and daemon directory picker") onto the monorepo's split Rooms module
as the execution of `apps/ocean-surface/docs/OCEAN_ROOMS_DESIGN_DIRECTION.md`
§5 ("there is no permanent right rail"). The right rail
(`.rooms-workspace__right`) is now one fixed slide-over at every width: 380px,
closed by default, `--bg-raised` on `--shadow-lg` with no border, a `--scrim`
backdrop that closes like Escape, a 280ms slide with the reduced-motion kill
after the state selectors, a full-screen sheet at or below 650px; the former
rail modes (Members, Files, Search, Thread) are its content and only the
thread panel docks inline at or above 1280px (§7) through a `--thread`
modifier, so the 901-1080 docked-rail breakpoints are gone. The header gains
the design's members pill (Groups glyph plus the roster count) toggling the
drawer. The legacy picker landed as `rooms_workspace/working_directory.rs`
(pure `working_directory_options` / `directory_choice` over the shared
`daemon.projects` signal, project roots and worktrees, the Tauri "Choose
folder…" row through `host::pick_folder`, and an explicit "Other absolute
path…" field; no browser-side path judgement) and is mounted exactly twice:
the create form in `room_list.rs` (replacing PR F's select, `allow_empty=true`)
and the new drawer "Default folder" row (`rooms_workspace/workspace_binding.rs`,
design §5.2: root in mono or "none", the cwd sentence, a ghost Edit revealing
Bind / Unbind). Bind and Unbind are the surface's first callers of the daemon's
`PATCH /v1/rooms/persistent/{key}` (ocean#161): `Rooms::set_open_room_workspace`
sends `RoomWorkspacePatchBody` with `workspace_root` as a string or an
EXPLICIT `null` (the daemon reads the field absent-vs-null, so the body has no
`skip_serializing_if`), one in flight, reply admitted by `room_is_current`, a
2xx room replacing `open_room` and the rail row, refusals folded by
`classify_workspace_patch` into one sentence beside the control with the draft
kept (`invalid_workspace_root` → "workspace folder not found on this Ocean").
The proxy already forwards the route generically (`RoomsPersistentShape::Json`,
no operator key), so it is untouched. Contract boundaries kept verbatim from
the legacy PR: no resource-grant softening, no browser filesystem authority or
path validation (the daemon's 400 is the only verdict), unbound rooms still
refuse turns (`workspace_unavailable`, surfaced at the action). Where the
legacy PR and the design doc disagreed the design won and is recorded in the
AGENTS.md: no room-wide "agents cannot run" copy (S7 deletes it; the per-agent
line belongs to `inspect`), no 840px centred transcript/composer (main's
full-bleed ruling and the 72ch measure already stand; the docked composer card
already shipped on main as P5's "single elevated composer"), and the thread
panel stays inline at wide widths rather than becoming an overlay. Kept intact:
unread divider, workspace-root grouping and load-more, agent badge, close
control, the Files panel, Coding plans. Tests: picker and binding unit tests,
`workspace_patch_*` fold tests (string vs null body, 2xx-only room, refusal
sentences), `tests/room_details_drawer.rs` (slide-over geometry, reduced-motion
order, 650/1280 blocks, pill and scrim wiring, sole PATCH callers, CSS↔emitter
parity, tokens only), `tests/directory_selection.rs` (shared signal, two mounts,
no pre-validation tokens, daemon-host copy, banned softening and room-wide
warning phrases, aria linkage, reset epochs, keyboard order), the mobile
anti-zoom floor extended to the picker with the cascade-order check and its
self-test. `OCEAN_ROOMS_PRODUCT.md` gains the drawer and Default folder
sections and the PATCH row (the doc-truth guard now requires the route to be
sent); the Surface AGENTS.md Rooms contract replaces "this surface does not
call it". Validation (private CARGO_TARGET_DIR in the worktree): see the PR
body for the final counts of `cargo test -p ocean-surface-ui`, the wasm32
clippy under `-D warnings` and `cargo fmt --check`; not exercised live.
