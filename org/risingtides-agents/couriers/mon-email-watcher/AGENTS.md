# mon-email-watcher — agent instructions

Provider-agnostic Ocean courier over the **`mon-rovia-watcher` Rust engine**
(`/Users/ericcromartie/Documents/Development/mon-rovia-email-watcher`, override
via `$OCEAN_MON_EMAIL_WATCHER_ENGINE_DIR`). The engine is the only thing that
talks to Gmail/Calendar/Slack; the agent loop only orchestrates and confirms.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral and applies regardless of which model drives this agent under the
Ocean runtime.

Principles:
- The engine owns transport: OAuth token refresh, Gmail search, date
  extraction, idempotent calendar upserts, `.mon_watcher/` state, Slack
  drafting. Never hand-roll any of it.
- Lead with `bin/courier preview` (engine `sweep-preview`) — zero writes.
  Confirm with the operator before a real `sweep` (real calendar, saved state).
- The engine's daily 9 AM launchd job stays untouched; the launchd verbs are
  refused by the wrapper. Slack drafts are never auto-sent.
- OAuth files (`client_secret.json`, `token.json`) live in the engine dir only.
  Nothing secret in this package, env-var names only, nothing committed.

Self-test (no token needed — exercises the wrapper's not-built error path):
```bash
OCEAN_MON_EMAIL_WATCHER_ENGINE_DIR=/tmp bin/courier preview   # expect "engine binary not built"
```
