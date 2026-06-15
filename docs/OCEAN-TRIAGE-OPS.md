# Ocean Context Triage — Ops Runbook

## Canonical boundary note

- `ocean-daemon` — local runtime/body/execution authority.
- `ocean-longhouse` — hive brain/coordination layer per `ocean-os/docs/LONGHOUSE.md`.
- `ocean-bedrock` — cloud-box support substrate. Hosts the triage ritual, operator tooling, and context/ledger infrastructure. **Not** the quorum engine.

## What triage does

`runOceanContextTriage` (`src/ocean-context.mjs`) runs a DB snapshot, verifies the ledger hash-chain, checks semantic health, and writes:

1. A dated report to `/context/ocean-bedrock/triage/YYYY-MM-DD.md`.
2. A slot-stamped alert digest to `/context/ocean-bedrock/triage/_alerts/YYYY-MM-DD-HH.md` (only when actionable findings exist, i.e. severity `high` or `medium`).
3. An `ocean_context.triage.completed` ledger event with `source_sequence = triage:<day>:<slot>`.

## Cron scheduler (Railway)

Service: `ocean-bedrock-triage-cron`  
Project: `rising-tides-agents`  
Schedule: `0 6,18 * * *` UTC (06:00 and 18:00 daily)  
Start command: `npm run ocean:triage`  
Exit behavior: script POSTs once and exits — Railway cron does not keep it alive.

### Provision the cron service

```bash
# From repo root, using the Railway skill / MCP.
# 1. Create the service in the production environment with the cron schedule.
# 2. Set environment variables (NO secrets in git):
#    OCEAN_BEDROCK_URL=https://ocean-bedrock-production.up.railway.app
#    OCEAN_BEDROCK_TOKEN=<scoped-contributor-token>
# 3. Deploy (Railway will run on the schedule immediately).
```

Required token scopes:
- role: `contributor`
- scopes: `write`, `/context/ocean-bedrock`
- clearance: `CONFIDENTIAL`

Token is set via a Railway service variable — never committed to the repo.

### Verify a cron run

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"

# Check today's report
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/file?path=/context/ocean-bedrock/triage/$(date -u +%Y-%m-%d).md"

# Check this slot's alert digest (if any actionable findings)
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/file?path=/context/ocean-bedrock/triage/_alerts/$(date -u +%Y-%m-%d-%H).md"
```

### Run triage manually

```bash
# Uses contributor token from file by default
npm run ocean:triage

# Force re-run even if this hour-slot already ran
npm run ocean:triage -- --force

# Override the report path
npm run ocean:triage -- --report-path /context/ocean-bedrock/triage/manual-run.md
```

### Rollback / disable

To disable without deleting: pause the cron schedule in Railway (set an empty schedule or toggle the service off).  
To revoke the token: issue a new token, update the Railway variable, redeploy. The old token hash-comparison will fail on next run.

## Idempotency

Each scheduled slot is keyed `triage:<YYYY-MM-DD>:<HH>` in the ledger. A re-fire within the same UTC hour returns `skipped: already-ran-this-slot` without doing any work. `--force` bypasses this guard.

The two daily slots (06:00 and 18:00) each produce their own event. The report file is shared per day (`YYYY-MM-DD.md`) and the 18:00 run overwrites it in-place; the alert digest is slot-stamped (`YYYY-MM-DD-06.md` and `YYYY-MM-DD-18.md`) so both coexist.

Railway cron's built-in overlap-skip (prior Active execution blocks the next) provides a second layer of protection against concurrent runs.

## Alert surfaces and thresholds

### Primary surfaces

Alert digest is written only when `high` or `medium` findings exist.

Active:
- Digest file: `/context/ocean-bedrock/triage/_alerts/<day>-<slot>.md`
- Telegram: `OCEAN_TRIAGE_TELEGRAM_BOT_TOKEN` + `OCEAN_TRIAGE_TELEGRAM_CHAT_ID` are set on the `ocean-bedrock` service.

Deferred/off by default:
- `OCEAN_TRIAGE_WEBHOOK_URL` → generic POST

### Configurable thresholds

| Env var | Default | Meaning |
|---|---|---|
| `OCEAN_TRIAGE_FAILED_ALERT` | `0` | Emit a finding when `failedJobs > N` |
| `OCEAN_TRIAGE_QUEUE_ALERT` | `50` | Emit a finding when `queuedJobs > N` |
| `OCEAN_TRIAGE_STALE_JOB_DAYS` | `3` | Days before a failed job is considered stale |

Example: set `OCEAN_TRIAGE_FAILED_ALERT=100` to silence the 9 historical failed-job finding once they are accepted as known-but-not-yet-cleaned.

## Failed ingest job tooling (`npm run ocean:jobs`)

Requires `DATABASE_URL` — run via Railway or `railway run` from the project root.

```bash
# List all failed jobs
npm run ocean:jobs
npm run ocean:jobs -- list

# Inspect one job (full row + payload)
npm run ocean:jobs -- inspect <id>

# Retry a single job (status → queued, worker picks it up)
npm run ocean:jobs -- retry <id>
npm run ocean:jobs -- retry <id> --reset-attempts   # also resets attempt counter

# Retry all failed jobs — dry-run by default, add --yes to mutate
npm run ocean:jobs -- retry --all
npm run ocean:jobs -- retry --all --reset-attempts --yes

# Abandon (cancel) a single job
npm run ocean:jobs -- abandon <id> --reason "file deleted, no longer needed"

# Abandon all — dry-run by default, add --yes to mutate
npm run ocean:jobs -- abandon --all --reason "historical debug runs, not worth retrying" --yes
```

Each retry/abandon appends a ledger event (`ocean_context.ingest.job_requeued` or `ocean_context.ingest.job_abandoned`) for audit.

Abandon does **not** delete the underlying object from `longhouse.objects` or disk — it only marks the job row `cancelled`.

### Disposition of the 9 historical failed rows

Completed 2026-06-15.

Disposition:
- All 9 historical failed `index_object` jobs were stale `/docs/smoke/hello.md` file-not-found rows from early smoke runs.
- They were marked `cancelled`/abandoned rather than retried because the underlying file no longer exists.
- Forced triage afterward reported `failedJobs=0`, `staleFailedJobs=0`, `queuedJobs=0`, and `pendingChunks=0`.

If new failed jobs appear:

1. `npm run ocean:jobs -- list` — record each job's `virtual_path` and `error`.
2. For transient errors (fetch retry, embed timeout): `npm run ocean:jobs -- retry <id> --reset-attempts`.
3. For file-not-found / stale debug paths: `npm run ocean:jobs -- abandon <id> --reason "<explanation>"`.
4. After dispositioned: verify `failedJobs = 0` via the next triage run's `dbCounts`.

## Daily operator loop

1. Check today's alert digest: `GET /context/ocean-bedrock/triage/_alerts/<today>-<slot>.md`.
2. If `failedJobs > 0`: run `npm run ocean:jobs -- list` and triage each row.
3. If `staleFailedJobs > 0`: retry transient ones, abandon dead paths.
4. If `pendingChunks > 0`: run `npm run ocean:semantic:backfill`.
5. If `failedSyncRuns > 0`: inspect source sync runs via the `/api/v1/sources/*` endpoints.

## In-server fallback (not default)

An in-server `setInterval` approach is documented here as a fallback for environments where a separate cron service is unavailable, but is **OFF by default**.

If enabled (`OCEAN_TRIAGE_SCHEDULER=1`), it must take a `pg_advisory_lock` (`pg_try_advisory_lock`) around the run to prevent double-fire on restart/redeploy in a multi-process or multi-replica scenario. This approach is NOT recommended for Railway's single-replica deployment; prefer the dedicated cron service.
