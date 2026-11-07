time:  [03:35] [07-11-26]
agent: [codex]
worktree: [codex/ocean-agent-sse-resume-20261106] [base dc2af791]
type:  [bug report]
area:  [backend]

The Surface proxy no longer strips the SSE resume cursor on `/v1/agent/events`.
`proxy_events` in `apps/ocean-surface/crates/ocean-surface-proxy/src/main.rs`
now forwards a valid `Last-Event-ID` request header to the daemon GET
(visible-ASCII values only, matching the established in-file forwarding
pattern; invalid values are dropped and the daemon treats the connection as
fresh). The query string is forwarded verbatim, the upstream request still
carries no cookie, and `sse_stream_response` is untouched so daemon frame ids
reach the browser for the next reconnect. With the cursor passed through, a
browser `EventSource` reconnect now reaches the daemon's existing
replay/reset ring-buffer logic instead of silently restarting the stream.
Added loopback regression test `agent_events_resume_forwards_last_event_id_to_daemon`
(echo daemon asserts forwarded `Last-Event-ID`, verbatim query, and cookie
absence) and recorded the session-stream reconnect rule in
`apps/ocean-surface/AGENTS.md`.

Validation:

- `cargo test --locked -p ocean-surface-proxy` from `apps/ocean-surface`: 73 passed, 0 failed, including the new `agent_events_resume_forwards_last_event_id_to_daemon` loopback test.
- `cargo check --locked -p ocean-surface-proxy` clean.
- PR #189 CI on head 501e800c: Build Surface pass, Build scope pass; Build Ocean skipping (Surface-only scope).

Transport prerequisite only for client rehydrate (issue #136 adjacency); no
client-side rehydration in this change. Frozen `events.md` and prior
`events/` files untouched.
