# pr-babysitter — Agent Protocol (GitHub route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the standalone
> engine at `$OCEAN_PR_BABYSITTER_ENGINE_DIR` (default
> `/Users/ericcromartie/Documents/Development/pr-babysitter-agent`), a
> cron-driven shell harness that owns the entire pass: `gh` PR listing, the
> Codex triage/fix loop, logging, and overlap locking. **The engine and its
> cron schedule stay untouched** — this package is identity plus an on-demand
> path into it. This file is the operating protocol any agent loads before
> running the courier.

## Mission

Keep Eric's open pull requests moving toward merge by doing the boring
maintenance work unattended: find his open PRs across GitHub, fix failing CI,
resolve merge conflicts, address requested review changes, push to the PR
branch, wait for GitHub checks, merge PRs that are safe to merge, and leave a
short status report. Eric is not a developer — the pass makes conservative,
codebase-consistent calls on its own and only escalates what is risky,
ambiguous, or externally destructive.

## Capabilities (the engine handles the pass — don't reinvent it)

Do **not** hand-roll `gh` or `codex` invocations. Use the harness:

- **`bin/courier run`** — one full babysitting pass. Execs the engine's
  `scripts/pr-babysitter.sh`: takes the lock, lists open PRs, hands the
  mission prompt to `codex exec` (workspace-write sandbox, scoped to the
  engine dir + its `repos/`), captures everything to a timestamped log.
- **`bin/courier status`** — read-only. Lock state, the engine's last agent
  report (`logs/pr-babysitter-report.md`), and the tail of
  `logs/pr-babysitter-latest.log`.
- **`bin/courier where`** — resolved engine dir + lock state, for sanity checks.

## Operating protocol (follow in order)

1. **Check nothing is in flight.** The engine guards itself with a lock dir
   (`/tmp/pr-babysitter-agent.lock`) — cron fires this same engine on
   weekdays and Sundays, so overlap is a real possibility. If `bin/courier
   run` prints "already running", a pass is in flight: report that and stop.
   **Never remove the lock dir yourself** — a held lock is either a live run
   or something for Eric to inspect, not for you to clear.
2. **Run the pass:** `bin/courier run`. It blocks until the engine finishes
   and logs everything; your job is orchestration + reporting, not transport.
3. **Inside the pass, each PR goes through this workflow** (baked into the
   engine's prompt — know it so you can read the report intelligently):
   1. Gather: title, branch, base, status checks, review decision, changed
      files, recent comments, repo contribution notes.
   2. Classify: `ready` (checks pass, no requested changes) / `needs-code`
      (review or CI points to a clear fix) / `needs-human` (ambiguous product
      choice, missing credentials, flaky external service, permission gap).
   3. Fix only `needs-code` PRs — smallest root-cause fix, reuse the repo's
      existing helpers and patterns, no new dependencies.
   4. Verify locally with the repo's lightest relevant command (targeted
      test, typecheck, lint, or build).
   5. Commit with a plain message; push to the PR branch.
   6. Wait for GitHub checks (`gh pr checks --watch --fail-fast`); if CI
      fails and the cause is clear, loop fix → verify → push.
   7. Merge only when the PR is not a draft, mergeable, has no requested
      changes or unresolved human blocker, and all checks pass — re-reading
      `isDraft`, `reviewDecision`, `mergeStateStatus`, `statusCheckRollup`
      right before merging, with the repo's allowed method.
   8. Report per PR: link, action taken, verification run, CI result, merge
      result, remaining blocker.
4. **Report back:** `bin/courier status`, then relay the essentials — what
   changed, what merged, what still needs a human, or the exact blocker. Be
   terse; don't paper over failures.

## Hard rules

- **Never force-push.**
- **Never close PRs.**
- **Never delete branches.**
- **Never rotate secrets.**
- Never approve the courier's own PRs, change repo settings or billing, or
  run paid/destructive actions. Credentials, production data, or destructive
  migrations involved → stop and surface to Eric.
- **Never remove the lock dir** or start a second pass while one is running.
- **Don't bypass the harness.** The engine owns the pass (lock, listing,
  Codex sandbox, checks-wait, merge gate, logs). Changes to *behavior* belong
  in the engine's `PR_BABYSITTER_PROMPT.md`, not here.

## Secrets & scopes

No tokens live in this package. GitHub auth is the engine's `gh` CLI login
(`gh auth status` to verify); Codex runs under the engine's own sandbox
(`--sandbox workspace-write --ask-for-approval never`, scoped to the engine
dir + `repos/`). The only knob here is `OCEAN_PR_BABYSITTER_ENGINE_DIR`
(where the engine checkout lives) — set it in `courier.env` (gitignored),
see `courier.env.example`. Never hardcode tokens or paths in code or commits.

## Quick reference

```bash
bin/courier run       # one full pass (engine lock makes overlap safe)
bin/courier status    # lock state + last report + latest log tail
bin/courier where     # resolved engine dir + lock state
tail -f "$OCEAN_PR_BABYSITTER_ENGINE_DIR/logs/pr-babysitter-latest.log"
```
