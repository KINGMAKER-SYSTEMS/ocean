time: [13:50] [21-10-26]
agent: [Codex] [factory integration builder]
worktree: [codex/factory-desktop-integration]
type: [bug report] [handoff]
area: [frontend] [testing]

Integrated reviewed PR #138 head 651d25dfda1c9a267fe94154b81835963f05f1cb
into PR #147 from d5a24a008f8dae5267c3d397699eceaf0ac2a09e through an
actual merge, retaining source ancestry. The merge applied cleanly.

Closed the main-only SteeringDelivered variant of issue #136: the stream
dispatches admitted/deduplicated steering before the ordinary event reducer,
so its transcript append now advances the same projection activity fence.
Existing admission, duplicate filtering, pending-row matching and steering
queue behavior remain unchanged. A deterministic delayed-read regression
exercises the production recovery seam for local and other-client delivery:
a stale snapshot cannot erase delivered text, Stop, queued prompts or unrelated
pending steers, and a fresh authoritative retry converges.

Surface devlog records this bypass and fence contract. Legacy events.md remains
byte-identical to canonical main, and all prior integration and repair event
files remain byte-identical. No ownership/index boundaries changed. Source
whitespace checks passed before freeze; targeted integrated projection/recovery/
steering checks and a real current Trunk bundle will run after this commit under
one bounded slot. Independent final-head review and fresh required checks remain
release gates. No push, canonical merge, install, deployment, real credentials
or live provider/coworker acceptance is claimed.
