time:  [22:40] [07-11-26]
agent: [zcode cli] [glm-5.3]
worktree: [kmf/fq-k27] [~/dev/kingmaker-ocean-worktrees/kmf-fq-k27-r2235]
type:  [docs]
area:  [writing]

Reconciled the event-resume item in `platform/ocean-os/ROADMAP.md` after the
SSE-cursor merge (factory FQ-K27, follow-on to #189, e6771669). The single open
item combined a repair clause and a contract-test clause; the repair clause is
now satisfied at every hop, so the item is split into a closed repair line and
an open contract-test remainder. Repair evidence on main 5698dfad: the Surface
proxy forwards a valid `Last-Event-ID` on `/v1/agent/events`
(`apps/ocean-surface/crates/ocean-surface-proxy/src/main.rs`, with the
`agent_events_resume_forwards_last_event_id_to_daemon` loopback test); the
daemon parses the header and replays through its bounded event ring
(`platform/ocean-os/crates/ocean-daemon/src/main.rs` `parse_last_event_id` /
`subscribe_with_replay`, ring in `crates/ocean-daemon/src/bus.rs`); the
observatory and control hops pre-existed; and the Surface product feed is
session-scoped with no `?all=1`. The remaining open item is scoped to the
contract test only: existing coverage is per-layer with mocked counterparts,
and nothing exercises the resume path cross-layer. The "do not use
`/v1/agent/events?all=1` as the product feed" constraint is preserved verbatim.
Caveat recorded in the PR body: the Surface UI deliberately re-opens fresh
EventSources with no stale cursor anchor (projection recovery rather than
cursor replay on app-managed re-opens), so an operator reading "end-to-end" as
requiring UI cursor adoption may prefer the item fully open.

Validation: `cargo xtask docs-check` passed from `platform/ocean-os/`; grep
found no other ROADMAP or spec line claiming the agent resume path is
unforwarded.
