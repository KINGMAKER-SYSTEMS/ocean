# tt-uploader — Agent Protocol (TikTok auto-uploader ops window)

> An Ocean courier. The *hands* are `bin/courier`, a thin read-only wrapper over
> the live TikTok auto-uploader engine at the `Auto Uploader (TT)` repo — a
> Playwright + stealth uploader with a self-healing orchestrator (`src/`) and a
> deployed FastAPI + Socket.IO backend (`backend/`, `app.main:socket_app`,
> local `:8000` or Railway), pulling videos from Google Drive and posting to
> TikTok Studio for Rising Tides' owned pages. This file is the operating
> protocol any agent loads before running the courier. The engine's own
> `AGENTS.md` / `README.md` / `MORNING-CHECKLIST.md` (in the engine repo) are
> the authoritative runbooks.

## Mission

Give the operator a **read-first ops window** into the uploader: is the backend
alive, what did the supervisor last decide, how many posts landed vs the daily
target per account, what's approved and runnable in the queue, what actually
posted, and is the source pipeline ready. **Visibility, not triggering.** The
courier inspects; the engine uploads. Nothing here starts an upload, edits a
manifest, or touches an account.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll TikTok/Drive/proxy calls or edits to the engine's files.
Use the harness — every verb maps to a verified surface:

- **`health`** — `GET /health` on the backend API (quick liveness, no DB).
- **`proxy`** — `GET /api/system/proxy/status`, the backend's read-only proxy
  status route. **Never** the POST `/api/system/proxy/test` route.
- **`accounts` / `uploads`** — `GET /api/accounts` and `GET /api/uploads`
  (list views only).
- **`status`** — the orchestrator supervisor's latest decision snapshot
  (`logs/auto_supervisor.json`: action/hold, posted_total, device_ready,
  source_ready, and the blocking detail string).
- **`heartbeat`** — posted vs `daily_target`, per-account posted/runnable
  counts (`logs/heartbeat/codex-status.json`).
- **`escalations`** — escalated blockers awaiting a human
  (`logs/heartbeat/codex-escalation.json`).
- **`queue`** — the approved runnable manifest (`logs/approved_manifest.json`;
  schema in the engine's `logs/approved_manifest.SCHEMA.md`). Add
  `--candidates` for the candidate manifest.
- **`posts`** — recent entries from the post ledger (`logs/post_log.json`).
- **`source`** — iCloud source readiness (`logs/icloud_source_status.json`).
- **`dry-run`** — last dry-run state (`logs/dry_run_status.json`).

Not exposed on purpose: anything that uploads, retries, or mutates —
`python -m src.orchestrator.scheduler`, `sandbox/dry_run.py`, the backend's
POST/PUT/DELETE routes, cookie refresh scripts, proxy tests. Those belong to
the engine and its own supervision, not to this package.

## Operating protocol (follow in order)

1. **Status before anything — always.** Open with `bin/courier status` and
   `bin/courier heartbeat`; they read the orchestrator's live status files and
   work even when the backend API is down. Add `bin/courier health` when the
   operator cares about the backend service itself. If the backend is
   unreachable, say so and stop — restarting uvicorn/Railway is an engine-repo
   operation, not something this courier does silently.
2. **Inspect, then answer.** For queue questions run `queue` /
   `queue --candidates`; for "what went out" run `posts --limit N`; for "why is
   it holding" pair `status` with `source` and `escalations`. Report what the
   engine says — don't infer state from memory.
3. **Read the supervisor's own words.** `status.detail` and the escalation
   entries carry the engine's exact blocking reasons (missing captions, source
   seed contract failures, device not ready). Relay them verbatim; don't
   paraphrase a red gate into something softer.
4. **Freshness matters.** Every status file carries an `at` timestamp. If a
   snapshot is old (hours/days), flag the staleness explicitly rather than
   presenting it as current state.
5. **Report.** Posted vs daily target, per-account counts, queue depth, and
   anything red (holds, escalations, source failures, proxy errors) — plainly,
   with the engine's own wording.

## Hard rules

- **NEVER trigger uploads from this package — v1 is read-only, no exceptions.**
  No orchestrator/scheduler runs, no sandbox dry-runs, no POST/PUT/DELETE to
  the backend, no "just this once" upload. Uploads stay entirely behind the
  engine's own self-healing orchestrator.
- **Never touch cookies or proxy config.** `config/cookies/` is
  Fernet-encrypted and stays sealed — never read, decrypt, copy, or relay it
  (same for `config/ig_sessions/` and `config/proxies.json`). Cookie refresh
  and proxy failover are the engine's jobs.
- **Stealth parameters are the engine's domain.** Residential proxies, UA and
  viewport rotation, human-behavior simulation are load-bearing anti-detection
  — never tune, disable, or "improve" them from here. Engine changes get
  validated against the engine's sandbox, never live.
- **Slack alerts are the engine's channel.** The engine alerts on its own
  successes/failures via its webhook. This courier reports to the operator in
  the conversation — it never posts alerts of its own.
- **Engine repo stays untouched.** Both layers — `src/` engine and `backend/`
  deployed app — are read-only from here. No `.env` edits, no manifest or
  ledger hand-edits (the orchestrator owns those writes), no migrations, no
  deploys.
- **Never hardcode tokens or secrets.** The engine loads its own `.env`; the
  courier neither reads nor relays creds.
- **Don't bypass the harness.** `bin/courier` owns the engine path (spaces and
  parens — always quoted), the API base URL, and the read-only verb surface.
  Orchestrate + confirm; don't curl backend routes or open logs/ files ad hoc.

## Secrets & scopes

The courier itself needs **no secrets** — localhost GETs are unauthenticated
and the status files are plain JSON. The engine's creds live in the ENGINE
repo's `.env` and are referenced by env var name only, never by value:
`PROXY_USERNAME` / `PROXY_PASSWORD` (Decodo/Smartproxy residential proxies),
`COOKIE_ENCRYPTION_KEY` (Fernet key for the cookie store — never read the
store itself), `GOOGLE_DRIVE_CREDENTIALS_PATH` / `GOOGLE_DRIVE_TOKEN_PATH`,
`SLACK_WEBHOOK_URL`, `NOTION_API_KEY`, `SENTRY_DSN`. Local courier config
(engine path override, API URL) goes in `courier.env` (gitignored); see
`courier.env.example`. `OCEAN_TT_UPLOADER_ENGINE_DIR` defaults to
`/Users/ericcromartie/Documents/Development/Auto Uploader (TT)` — the path
contains spaces and parentheses, so quote it everywhere.

## Quick reference

```bash
bin/courier status                 # supervisor decision snapshot (action/hold + why)
bin/courier heartbeat              # posted vs daily target, per-account counts
bin/courier escalations            # blockers escalated to a human
bin/courier queue                  # approved runnable manifest
bin/courier queue --candidates     # candidate manifest
bin/courier posts --limit 10       # recent post-ledger entries
bin/courier source                 # iCloud source readiness
bin/courier dry-run                # last dry-run state
bin/courier health                 # backend API liveness (:8000 / Railway)
bin/courier proxy                  # proxy status (read-only route)
bin/courier accounts               # GET /api/accounts
bin/courier uploads                # GET /api/uploads
```
