# SOP — gallery lookup & rendering (content-agent / Slack)

The intake → reason → reply skeleton for "show me the gallery / recent clips."
Driven over the content-lab API (`tools.toml [api.content_lab] gallery_list`).

## Intake
"What's in the gallery?", "show me the last few clips", "pull the X campaign clips."
Resolve any filter the operator named (campaign, sound, date range, count).

## Reason
- A gallery is **structured, revisitable content** → it belongs in a **Slack canvas**
  (house canvas SOP), not a wall of inline links.
- A single "did clip X land?" is a one-liner → keep it inline.

## Act
1. `GET {CONTENT_LAB_API_URL}/api/gallery` with any filter as query params,
   `Authorization` from `CONTENT_LAB_API_KEY`.
2. Build a canvas: a clean list/grid of clips — title, link (`<url|label>`), and the
   key metadata (campaign/sound/status). **Append** to an existing gallery canvas for
   an ongoing task rather than overwriting it.
3. Create/update the canvas through the transport (`couriers/transport/slack.py`
   `create_canvas`), never by hand.

## Reply
- Post the canvas, then a **one-line in-thread pointer**: "Updated the gallery canvas
  👆 — 6 new clips." Never drop a canvas silently.
- If the gallery is empty or the filter matched nothing, say so in one line inline —
  don't render an empty canvas.

## Skeleton (deferred to live-wiring)
Canvas assembly + transport call are wired during build-order step 4; this SOP is the
contract. No deletes of existing canvases (append-only, spec §9).
