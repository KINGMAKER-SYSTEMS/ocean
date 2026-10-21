time: [01:40pm EDT] [10-21-26]
agent: [Codex] [review129_mcp]
worktree: [codex/factory-candidate-review-repairs]
type: [bug report]
area: [frontend]

Repair PR138 finding 4339877903 under existing reconnect issue #136. A session
snapshot captured before live text/tool/component events could finish its
permission read later and overwrite those events within the same projection
epoch. Every admitted mutation of snapshot-owned metadata/transcript/pinned
content now advances the existing activity fence; lifecycle and permission
authority safeguards remain intact.

Initial hydration and SSE reconnect share activity-race recovery. Four immediate
attempts handle brief collisions, then one pending recovery waits for a 250ms
quiet window without issuing HTTP reads during continuous content churn. Newer
full reads or focus/SSE changes retire that waiter. Live content keeps rendering;
recovery is visible and converges when content pauses or the turn ends. No blind
delta replay or new daemon policy is introduced. Main-only SteeringDelivered
requires the corresponding activity fence during integration, owned separately.

Deterministic production-seam fixtures cover admitted text/thinking/tool/pinned
and metadata frames, rejected/suppressed frames, continuous-stream coalescing,
quiet convergence and retirement by a newer full read. Formatting and diff
whitespace checks pass. Targeted tests and independent review are pending at
this source checkpoint; no merge, deployment or live acceptance is claimed.
