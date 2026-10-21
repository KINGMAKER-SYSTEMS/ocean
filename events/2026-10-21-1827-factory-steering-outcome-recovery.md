time: [02:27pm] [10-21-26]
agent: [Codex] [runtime implementation]
worktree: [codex/factory-desktop-integration]
type: [bug report]: Issue #148 steering outcome recovery
area: [backend] [frontend] [testing]

Repairs the missed delivery/finish gap in PR147's source-reviewed integration
8cd4a343b891eb1776aa5a6dd484d78a0a7e9c88. Client steering identities are allocated
before submission and bound to the exact originating turn. Existing text-only
callers remain compatible. Duplicate identities never enqueue twice; mismatched
text/turn refuses, active receipt capacity refuses new identities instead of
evicting dedup evidence.

Existing inbox custody owns pending, in-flight, delivered and unconsumed outcomes.
The persistence owner publishes delivery only after the exact ordered checkpoint
saves durably, acknowledging no later runtime batch. Closed-turn evidence is
bounded and boot-local; absence, eviction, restart and failed persistence are
unknown, never proof to resend. Transcript reads bracket receipt snapshots and
recheck after daemon live-state enrichment; bounded churn returns typed503.
Ended proof excludes only the matching old Stop target, without claiming success.

Surface reconciles only admitted full projections. Delivered removes pending
without duplicating transcript; unconsumed queues once; ambiguous stays unresolved.
Delayed ambiguous SSE after hydration triggers authoritative refresh. Lost/late
ACKs, identical text, repeated recovery and stale focus cannot replay an intent.
Shared recovery owns bounded coherence retries and visible failure.

Pre-freeze targeted observations: runtime steering 8/8, agent steering 4/4,
initial Surface steering filter 4/4 passed before final review corrections.
Final-head targeted tests and real Trunk bundle remain release-owner gates;
results will be recorded outside frozen source and in PR evidence. No provider
credentials, live daemon/service changes, install, publication or deployment.
This is source work, not maintainer approval or live acceptance.

Devlog pass updated the nearest runtime, agent, core, daemon and Surface contracts.
Parent indexes are unchanged: no new module, route, package or ownership boundary.
All existing events and main's legacy events.md remain byte-identical.
