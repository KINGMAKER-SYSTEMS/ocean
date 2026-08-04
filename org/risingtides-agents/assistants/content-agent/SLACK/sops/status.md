# SOP — status / queue lookup (content-agent / Slack)

The intake → reason → reply skeleton for "what's the status / what's queued / did X
finish." Driven over the content-lab API (`tools.toml [api.content_lab] queue_status`
+ `video_status`).

## Intake
"Is anything still generating?", "what's the queue look like?", "did job 1234 finish?"
Resolve whether the operator wants the **whole pipeline status** or a **single job**.

## Reason
- A single job → `video_status/{job_id}`, answer inline in one line.
- The whole queue/pipeline → `queue_status`; if it's a long board, render a canvas
  (house canvas SOP); if it's a quick count, keep it inline.

## Act
1. Single job: `GET {CONTENT_LAB_API_URL}/api/video/status/{job_id}`.
2. Whole pipeline: `GET {CONTENT_LAB_API_URL}/api/status`.
   `Authorization` from `CONTENT_LAB_API_KEY` on both.

## Reply
- **Front-load the answer:** "✅ done — link" / "⏳ still generating, ~2 min" /
  "⚠️ failed — reason." Detail after, only if it earns its place.
- For a multi-row queue board, render a canvas + a one-line in-thread pointer.
- No raw log dumps; fence a short excerpt only if it helps.

## Skeleton (deferred to live-wiring)
The poll/report loop is wired during build-order step 4; this SOP is the contract the
agent reasons against. Read status from the API — never guess (spec §8).
