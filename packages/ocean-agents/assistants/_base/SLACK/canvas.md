<!--
  _base/SLACK/canvas.md — HOUSE Slack canvas-rendering SOPs. Shared by every Slack
  assistant. Composed under the agent profile by tools/compose_profile.py.
  Per design spec §4 + §8 (canvas rendering capability).
-->
## When to use a Slack Canvas (rich rendering SOP)

Render into a **Slack canvas** instead of a message when the content is too big or
too structured to read comfortably inline:

- **Reach for a canvas when:** the output is a **gallery** of generated media, a
  **status/queue board**, a multi-row table, a long structured summary, or anything
  the operator will want to revisit, scroll, or share. Messages are for the
  conversation; canvases are for the artifact.
- **Keep it inline when:** it's a direct answer, a short status, a confirmation, a
  link or two, or a quick back-and-forth. Don't canvas a one-liner.
- **Pairing:** when you create or update a canvas, **post a short message in-thread
  too** — one line of context + the canvas reference — so the thread stays readable
  ("Updated the gallery canvas 👆 — 6 new clips."). Never drop a canvas silently.
- **Drive canvas + message I/O through the agent's transport/tools, not by hand.**
  The transport (`couriers/transport/slack.py`) owns rate limits, retries, file
  upload, and canvas create/update — it's the surface mechanic that does the actual
  Slack I/O. Drive it. Don't re-implement Slack I/O by hand.
- **Append over overwrite.** Prefer updating/extending an existing canvas for an
  ongoing task over blowing it away — mirror the pipeline's "append-only is safer"
  rule so you never destroy a board someone is mid-review on.
