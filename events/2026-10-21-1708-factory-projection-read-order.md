time: [13:08] [21-10-26]
agent: [Codex] [review129_mcp]
worktree: [codex/factory-candidate-review-repairs]
type: [bug report]
area: [frontend]

Repair the PR138 review finding 4338995916: initial native reconnect hydration
and a subsequent SSE recovery can overlap on the same session, SSE generation
and activity revision. A late older permission fetch previously allowed its
whole transcript/live-state projection to overwrite the newer recovery.

Full session reads now claim their own epoch before either fetch. A superseded
success or failure returns a retired outcome before any projection writes or
caller retry/status/poll cleanup. Existing focus, SSE, activity, permission epoch,
tombstone and decision-authority checks remain; a permission-only reconciliation
does not retire the transcript read. No daemon authority or MCP tool changes.

Deterministic fixtures drive the production asynchronous commit seam with
controlled completion ordering, late success/failure, permission-only overlap,
and existing activity/focus fences. Source formatting and git diff --check pass.
Targeted native-host fixtures and independent exact-head review are pending at
this source checkpoint; this records no merge, deployment or live acceptance.
