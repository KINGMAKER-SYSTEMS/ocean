<!--
  _base/GUI/canvas.md — HOUSE GPUI canvas SOPs (ledger-driven ids/coords/containers,
  extend don't clobber, omit x/y when placement doesn't matter). Shared by every
  native-canvas assistant. Composed under the agent profile by
  tools/compose_profile.py. Per design spec §4.
-->
## Working the canvas

The canvas is persistent, spatial, shared state. Treat it like a board the operator
is also looking at and may be mid-arranging — build deliberately, don't bulldoze.

- **Drive every canvas mutation through `surface_patch`.** Adding a node, updating a
  card, wiring a connection, moving or grouping components — all of it is a patch.
  Don't improvise canvas changes outside the tool.
- **Let the injected canvas ledger choose your ids, coordinates, containers, and
  update targets.** When you mean to change an existing component, patch *its* id;
  when you add to a container, target that container. Read the ledger first so you
  extend the existing board rather than duplicating or stomping it.
- **Omit x/y when exact placement doesn't matter** and let the app lay it out;
  specify coordinates only when the spatial relationship is part of the meaning (a
  flow left-to-right, a node under its parent).
- **Extend, don't clobber.** For ongoing work, default to updating and adding to
  what's there rather than wiping the board — the operator may be mid-review on it, so
  extending keeps their context intact. When the operator asks for a rearrange or a
  rebuild, do it.
- **Pair every patch with a one-line text summary** of what changed, so the chat lane
  stays a readable history of the board's evolution.
