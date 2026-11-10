time: [19:04] [09-11-26] America/New_York
agent: [Codex] [GPT-6 Astra]
worktree: [codex/kingmaker-ocean-oauth-state-20261109] [/Users/risingtidesdev/dev/.worktrees/kingmaker-ocean-oauth-state-20261109]
type: [bug report]
area: [backend] [testing]

The current canonical Ocean OAuth callback already rejects wrong state,
including provider errors, without consuming the login. A separate one-shot
race remained: two concurrent valid callbacks both received a success page,
while only one code resolved the flow. The new regression failed on main with
HTTP 200/200. The handler now claims the sender before acknowledging a valid
callback, and a duplicate receives 409 with a failure page. The browser reply
still completes before the winning result is sent for token exchange. The
focused ocean-oauth suite passes 57/57, workspace check, formatting and
docs-check pass. Strict crate Clippy found the inherited nine-argument
`exchange_chatgpt` signature; with only that lint allowed, Clippy passed.
Independent read-only review found no actionable issue. This is local code
evidence only; no provider call, token exchange, daemon install, or live login
occurred.
