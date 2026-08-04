# SOP — generate a video (content-agent / Slack)

The intake → reason → reply skeleton for a generate request. Driven over the
content-lab API (`tools.toml [api.content_lab] generate_video` + `video_status`).

## Intake
An operator asks for a video in a thread/DM, e.g. "make me a 9:16 clip of X for the
Atlantic sound." Resolve from the turn:
- the **prompt** / creative intent (ask one clarifying question only if you truly
  can't proceed — don't quiz),
- any **constraints** the operator stated (aspect ratio, length, sound/campaign).

## Reason
- Map intent → the `generate_video` endpoint payload. If the request is ambiguous in
  a way that would waste a generation, ask the one thing you need; otherwise proceed
  with the obvious read and state your assumption.
- Decide deliverability: a single clip → deliver inline; a batch → plan a canvas
  gallery (see `gallery.md`).

## Act
1. `POST {CONTENT_LAB_API_URL}/api/video/generate` with the prompt + constraints,
   `Authorization` from `CONTENT_LAB_API_KEY`. Capture the returned `job_id`.
2. **Acknowledge immediately in-thread:** "👀 kicking off — I'll drop it here when it
   lands." Include a rough ETA if the response carries one.
3. Poll `GET /api/video/status/{job_id}` until `done`/`failed` (back off; don't
   hammer — the transport/limits rules apply).

## Reply
- **On success:** post the result in the **same thread** — upload the file via the
  transport, or post the gallery/R2 link with a one-line caption. ✅
- **On failure:** report plainly what failed and the next step; don't dump raw logs
  — fence a short excerpt if useful. ⚠️
- **Never** auto-repost or delete a prior result (append-only, spec §9).

## Skeleton (deferred to live-wiring)
The actual generate→poll loop is wired in the bridge/agent during step 4 of the
build order; this SOP is the contract the agent reasons against. The escape-hatch
path (drive Replicate/R2 directly via `bash`/`fetch`) is used only when the API
can't express the request.
