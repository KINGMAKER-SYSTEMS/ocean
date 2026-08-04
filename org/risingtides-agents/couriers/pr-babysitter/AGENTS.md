# pr-babysitter — agent instructions

Provider-agnostic Ocean courier on the **GitHub route**. The standalone engine
at `$OCEAN_PR_BABYSITTER_ENGINE_DIR` (default
`/Users/ericcromartie/Documents/Development/pr-babysitter-agent`) is the only
thing that touches GitHub; the agent loop only orchestrates and reports.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral and applies regardless of which model drives this agent under the
Ocean runtime.

Principles:
- The engine owns the pass: lock, `gh` PR listing, the Codex triage/fix loop,
  checks-wait, merge gate, logs. Never hand-roll `gh` or `codex` calls.
- One pass = `bin/courier run`. "already running" → report and stop; **never
  remove the lock dir** (`/tmp/pr-babysitter-agent.lock`).
- Hard prohibitions: never force-push, never close PRs, never delete branches,
  never rotate secrets.
- The engine and its production cron stay untouched. Behavior changes belong
  in the engine's `PR_BABYSITTER_PROMPT.md`, not in this package.
- No tokens here — GitHub auth is the engine's `gh` login; local config goes
  in `courier.env` (gitignored, see `courier.env.example`).

Self-test (read-only, starts nothing): `bin/courier where`
