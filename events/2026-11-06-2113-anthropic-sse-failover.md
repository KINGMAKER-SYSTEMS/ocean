time:  [21:13] [06-11-26]
agent: [codex desktop] [gpt-6]
worktree: [codex/ocean-anthropic-sse-failover-20261106] [dev/.worktrees/ocean-canonical-sse-failover-20261106]
type:  [bug report]
area:  [backend]

Anthropic can send an availability error inside an HTTP-200 SSE stream. The
protocol previously emitted a terminal message error for every such frame,
discarding the status class used by clean-round retry and pre-output provider
failover. Map rate-limit, API, timeout, and overloaded frames to typed
429/500/504/529 provider errors; keep caller and unknown errors terminal.
Document the boundary in the protocol owner contract.

Validation: the local HTTP/SSE regression failed on the old behavior and passed
after the fix. `cargo test -p ocean-protocol --locked` passed 192 unit and 5
integration tests; `cargo check --workspace --locked`, `cargo fmt --all --
--check`, and `git diff --check` passed. Independent review found no code
blocker. This is source-only; no provider call, live posting, or deployment.
