# mon-email-watcher — Agent Protocol (Mon Rovîa email route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the production
> `mon-rovia-watcher` Rust engine at
> `/Users/ericcromartie/Documents/Development/mon-rovia-email-watcher`
> (override via `$OCEAN_MON_EMAIL_WATCHER_ENGINE_DIR`), reached through the
> thin wrapper in `bin/courier`. This file is the operating protocol any agent
> loads before running the courier.

## Mission

Watch the Mon Rovîa management-client email flow: sweep Gmail
(`ECrom423@gmail.com`) for Mon Rovîa-related mail — especially Wasserman
(`teamwass.com`) booking threads from Boaz — extract actionable dates, upsert
Google Calendar markers on Eric's calendar, and prepare Slack-ready draft
copy. The engine already runs this daily at **9:00 AM** under launchd — this
courier is the **on-demand identity**: sweep now, or preview what the next
sweep would do.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll Gmail/Calendar API calls, date extraction, dedupe, or
Slack notifications. Use the wrapper:

- **`preview`** — the engine's own dry-run (`sweep-preview`): searches Gmail,
  extracts date candidates, and prints what WOULD be upserted. No calendar
  writes, no Slack, no state change. Safe to run any time.
- **`sweep`** — the real daily entry (`sweep`, exactly what launchd runs at
  9 AM): search → extract → idempotent calendar upserts → run report + site
  event feed → Slack notify. Without the webhook env, notifications land as
  **drafts** in `.mon_watcher/slack_drafts.jsonl`. Saves `state.json` so swept
  messages never re-fire.
- **`boaz-preview` / `boaz-sync`** — historical Wasserman/Boaz backfill
  (read-only preview / real calendar upserts over the full thread history).
  Rare, operator-initiated — deliberately not slash-registered; run only when
  Eric explicitly asks for a backfill, preview first.

Engine one-timers (`install-launchd`, `uninstall-launchd`, `gmail_oauth.py`)
are **deliberately not exposed** — the wrapper refuses the launchd verbs.
Schedule and OAuth setup happen in the engine dir with the operator, per the
engine's own AGENTS.md.

## Operating protocol (follow in order)

1. **Preview first when the operator's intent is "anything new?"** Run
   `bin/courier preview` and relay `Messages checked: N` plus the
   `date — title` candidate lines. Many requests end here — a preview answers
   "any new booking dates from Boaz?" without touching calendar, Slack, or
   state.
2. **Confirm before a real sweep.** A sweep writes markers onto Eric's real
   `primary` calendar and saves state. If the daily 9 AM launchd run is due
   imminently anyway, say so — an on-demand sweep is for "Eric needs this
   now", not routine.
3. **Sweep.** Run `bin/courier sweep`. The engine owns token refresh, Gmail
   search, dedupe against `state.json`, idempotent calendar upserts (keyed by
   its `SOURCE=mon_rovia_watcher` marker), state/report writes, and Slack
   drafting.
4. **Report.** Relay `Messages checked: N` / `Calendar markers upserted: M`
   and the `date — title` lines. Point Eric at
   `.mon_watcher/slack_drafts.jsonl` for the drafted announcement copy — the
   drafts are **not sent**; Eric reviews and sends.
5. **On failure, stop and report.** A non-zero exit means the sweep aborted —
   commonly an expired/blank OAuth token (re-auth happens in the engine dir
   via `gmail_oauth.py`, with the operator) or a Gmail/Calendar API error.
   Check `.mon_watcher/launchd.err.log` for the scheduled runs. Never "fix"
   state to make a run pass — see Hard rules.

## Hard rules

- **The engine stays in place and the launchd job stays live.** This package
  never moves, edits, rebuilds, or reinstalls the engine or its schedule, and
  never runs `install-launchd` / `uninstall-launchd`. On-demand invocation
  only.
- **Never hand-edit `.mon_watcher/` state.** `state.json` is the single source
  of which Gmail messages were already swept; the run reports, drafts, and
  `site_events.json` are engine-owned outputs. Corrupting state = duplicate
  markers and duplicate notifications. Read-only inspection is fine.
- **Preview before sweep.** An on-demand sweep touches Eric's real calendar —
  preview, confirm with the operator, then sweep.
- **Drafts never auto-send.** Never post `slack_drafts.jsonl` content to Slack
  yourself, never send email (the token carries `gmail.send` — do not use it),
  and never set `MON_ROVIA_SLACK_WEBHOOK_URL` to flip live posting without
  Eric's explicit say-so.
- **Respect the marker convention.** Calendar upserts are idempotent via the
  engine's `SOURCE=mon_rovia_watcher` marker. Never create or edit calendar
  events outside the engine — dedupe breaks.
- **Never hardcode tokens or secrets** in code or commits. OAuth material
  lives in the engine dir only (see below); nothing sensitive lives in this
  package.
- **Don't bypass the wrapper.** It pins the working directory to the engine
  dir — the binary resolves `watcher_config.json`, `token.json`, and
  `.mon_watcher/` relative to cwd, so a bare binary run from anywhere else
  spawns fresh, divergent state.

## Secrets & scopes

Nothing secret lives in this package. The OAuth files (`client_secret.json`,
`token.json`) stay in the **engine dir root** (gitignored there) — reference
them by location only; never print, copy, or commit their contents, and rotate
if exposed. The token authenticates `ECrom423@gmail.com` with scopes
`gmail.modify`, `gmail.send`, `calendar.events` against calendar `primary`.
Slack delivery is opt-in via `MON_ROVIA_SLACK_WEBHOOK_URL` (engine-side env;
default is drafts-only). Local overrides (e.g.
`OCEAN_MON_EMAIL_WATCHER_ENGINE_DIR`) go in `courier.env` (gitignored); see
`courier.env.example`.

## Quick reference

```bash
bin/courier preview        # dry-run: what would the next sweep upsert? (safe)
bin/courier sweep          # real sweep: calendar markers + Slack drafts (confirm first)
bin/courier boaz-preview   # dry-run the historical Boaz/Wasserman backfill
bin/courier help           # usage

# via the hub
python3 ../hub/router.py run /monmail-preview
python3 ../hub/router.py run /monmail

# engine-side observability (read-only)
ENGINE="${OCEAN_MON_EMAIL_WATCHER_ENGINE_DIR:-/Users/ericcromartie/Documents/Development/mon-rovia-email-watcher}"
cat  "$ENGINE/.mon_watcher/last_sweep.json"
tail -5  "$ENGINE/.mon_watcher/slack_drafts.jsonl"
tail -20 "$ENGINE/.mon_watcher/launchd.err.log"
```
