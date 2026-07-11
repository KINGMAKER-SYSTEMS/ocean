# mon-distro — Agent Protocol (Mon Rovia evergreen distro)

> An Ocean courier. The *hands* are `bin/courier`, a thin wrapper over the live
> Rust engine at the Postiz Posting Agent repo (`mon-posting-agent` — axum
> service on `127.0.0.1:3000` + scheduler loop, both under launchd, posting
> Mon Rovîa Reels to Facebook via Postiz on a rotation queue with a cursor in
> `data/queue.json`). This file is the operating protocol any agent loads
> before running the courier. The engine's own runbook is authoritative:
> `OPERATIONS.md` in the engine repo.

## Mission

Give the operator a **read-first ops window** into the Mon Rovia distro engine:
is it alive, is the queue healthy, what posts next, when, and did Postiz creds
resolve. The courier inspects; the engine posts. Nothing here schedules, edits
the queue, or flips a latch on its own.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll HTTP calls to Postiz or edits to `data/queue.json`. Use
the harness:

- **`health` / `status`** — hit the live launchd service on `:3000`
  (localhost, no token). `status` is the full readiness/gates/scheduler view.
- **`queue`** — engine `check`: validation counts (active / removed / postable /
  duplicate captions / issues). Exits non-zero on errors.
- **`preview` / `upcoming`** — next postable items off the rotation cursor, and
  the upcoming slot times they map to.
- **`readiness`** — the launch-gate report (captions, Mon keyword, review
  approvals, queue integrity).
- **`activity`** — recent events from `data/logs/activity.jsonl`.
- **`verify-postiz`** — read-only Postiz connectivity check; resolves the
  enabled Facebook integration. **Never posts.**
- **`post-now`** — pass-through to the engine's `post-now`, which is dry-run /
  draft / schedule-in unless `--enable-posting` is given. The wrapper refuses
  `--enable-posting` outright unless `OCEAN_MON_DISTRO_ALLOW_POSTING=1`.

Not exposed on purpose: `serve` / `schedule` (launchd owns those processes —
never race it), and the queue-mutating subcommands (captions, approval,
remove, promote, …) — queue edits go through the engine's dashboard/app flows.

## Operating protocol (follow in order)

1. **Status before action — always.** Open with `bin/courier health` and
   `bin/courier status`. If the service is down, stop and report; the fix is
   `launchctl kickstart` per the engine's `OPERATIONS.md`, not something this
   courier does silently.
2. **Inspect, then answer.** For queue questions run `queue`, `readiness`,
   `preview`, `upcoming`, `activity` as needed (add `--json` when you want to
   parse). Report what the engine says — don't infer state from memory.
3. **NEVER flip a latch without explicit operator instruction.** `DRY_RUN` and
   `POSTING_LOCKED` live in the engine's `.env` and block live posting even
   when everything else passes. Do not edit them, do not suggest editing them
   as a casual fix, do not set `OCEAN_MON_DISTRO_ALLOW_POSTING`. Only Eric
   says when a latch moves — read the instruction back before acting.
4. **`post-now` is a validation tool first.** Safe modes, in order of
   preference: `--dry-run` (plan only, no Postiz call), `--draft` (Postiz
   draft, never publishes), `--schedule-in <min>` (cancellable near-future
   post). A live post additionally needs `--enable-posting` AND the wrapper
   latch AND the engine latches to agree — confirm the exact item
   (`--drive-file-id`, caption) with the operator before any real call.
5. **Report.** Relay counts and gate results plainly; flag anything red
   (missing captions, missing Mon keyword, needs_review items, cursor stalls,
   Postiz failures) with the engine's own wording.

## Hard rules

- **This courier never posts on its own.** Posting stays behind the engine's
  latches; the wrapper adds its own refusal on top.
- **Engine, launchd jobs, and queue state stay untouched.** No `.env` edits,
  no hand-edits to `data/queue.json` (writes are serialized/versioned — the
  server owns them), no `launchctl` mutations from this package.
- **Never hardcode tokens or secrets.** The engine binary loads its own
  `.env` from the engine repo; the courier neither reads nor relays creds.
- **Don't bypass the harness.** `bin/courier` owns cwd (the binary expects
  repo root), path injection, and the posting latch. Orchestrate + confirm;
  don't exec the binary directly with ad-hoc paths.
- **Ignore the retired Node engine.** The README's `npm start` / `npm run
  live` describe the rollback, not production. Rust is live.

## Secrets & scopes

The courier itself needs **no secrets**. Postiz/Facebook creds
(`POSTIZ_API_KEY`, `FACEBOOK_INTEGRATION_ID`, …) live in the engine repo's
`.env`, loaded by the binary from its own working dir. Local courier config
(engine path override, API URL, the posting latch) goes in `courier.env`
(gitignored); see `courier.env.example`. `OCEAN_MON_DISTRO_ENGINE_DIR`
defaults to `/Users/ericcromartie/Documents/Development/Postiz Posting Agent`
— the path contains a space, so quote it everywhere.

## Quick reference

```bash
bin/courier health                       # is the service alive
bin/courier status                       # readiness / gates / scheduler state
bin/courier queue --json                 # queue validation counts
bin/courier readiness                    # launch-gate report
bin/courier preview --slots 5            # next postable items
bin/courier upcoming --slots 5           # upcoming slot times
bin/courier activity --limit 20          # recent activity events
bin/courier verify-postiz                # Postiz creds + integration (read-only)
bin/courier post-now --drive-file-id <id> --dry-run   # plan only, no Postiz call
```
