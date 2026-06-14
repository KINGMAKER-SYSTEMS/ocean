# Ocean — Next Waves (sequential firing order)

Generated 2026-06-14T02:19:17.838Z. Four independent wave specs, **fired sequentially** in the order below.
Each is a machine-readable `workflows/<id>.workflow.json` in the same house format as `ocean-bedrock-week-one.workflow.json` (canonical boundaries + model roster + dependency-ordered phases with tasks & acceptance).

Grounded against **verified live prod state (2026-06-14)**: semantic search is already live & populated; ledger on Postgres; triage manual (last ran 06-12, no scheduler); 9 failed ingest jobs; Longhouse router unbuilt (single `managed-local` mount); 8 adapters seeded but only `local_folder` has a runner; token registry is volume-JSON.

## Firing order

| # | Wave | Depends on | Phases | File targets | Spec |
|---|---|---|---|---|---|
| 1 | **Ocean Triage Scheduler + Ingest Job Cleanup** | — | 7 | 11 | `workflows/ocean-triage-scheduler.workflow.json` |
| 2 | **Ocean Longhouse Data-Plane Mount Router** | — | 8 | 10 | `workflows/ocean-longhouse-router.workflow.json` |
| 3 | **Ocean Source-Adapter Runners (GitHub + Telegram First)** | ocean-longhouse-router | 7 | 12 | `workflows/ocean-source-runners.workflow.json` |
| 4 | **Ocean Coworker Rollout Prep** | ocean-longhouse-router | 8 | 15 | `workflows/ocean-coworker-rollout.workflow.json` |

## Waves

### 1. Ocean Triage Scheduler + Ingest Job Cleanup

`ocean-triage-scheduler` — depends on: _none_

Make the daily Ocean Context triage run automatically (today it is manual and last ran 2026-06-12), route triage findings somewhere an operator actually sees instead of only a ledger event plus a /context report file, and give operators tooling to inspect, retry, or abandon the 9 historical failed longhouse.ingest_jobs rows so the ingest queue is healthy. Hardens HANDOFF Priority 1 without redefining canonical Ocean boundaries: Bedrock remains the cloud-box support substrate that runs the triage ritual and exposes operator tooling; it is not the quorum engine.

- 7 phases · 11 file targets · 8 DoD criteria · 5 open decisions · 6 risks

### 2. Ocean Longhouse Data-Plane Mount Router

`ocean-longhouse-router` — depends on: _none_

Build the documented Ocean Longhouse mount registry + StorageAdapter interface + path router + admin /api/v1/mounts API, and refactor the live file operations off the single hardcoded local-disk root (state.filesRoot / diskPathFor) so storage becomes pluggable. This makes the already-seeded longhouse.mounts table the source of truth for routing and unblocks future R2/s3/github/webdav adapters and source runners, without breaking the live single-mount prod during rollout. Scope is the local-adapter behavior-preserving refactor + registry/router/admin API only; external adapters and DB-backed locks are explicitly deferred.

- 8 phases · 10 file targets · 8 DoD criteria · 6 open decisions · 6 risks

### 3. Ocean Source-Adapter Runners (GitHub + Telegram First)

`ocean-source-runners` — depends on: `ocean-longhouse-router`

Replace facade source adapters with real server-side fetch/ingest runners, starting with GitHub and Telegram per docs/SOURCE-ADAPTER-PRECEDENTS.md. Each runner fetches remote content, dedupes by content hash, writes objects through the storage router (built in ocean-longhouse-router), records the existing source_instance -> source_stream -> source_sync_run -> source_record lineage plus Ocean Ledger events, persists incremental cursors in longhouse.ledger_source_cursors, and only advertises capabilities (delete_detection/webhook/incremental) that are actually implemented. Stops longhouse.source_adapters from advertising GitHub/Telegram/Slack/Notion/Linear/Drive/R2 capabilities that have no runner behind them.

- 7 phases · 12 file targets · 8 DoD criteria · 6 open decisions · 7 risks

### 4. Ocean Coworker Rollout Prep

`ocean-coworker-rollout` — depends on: `ocean-longhouse-router`

Take the V0 local sync GUI (scripts/ocean-local-app.mjs) and raw-token-paste onboarding to a safe, repeatable coworker rollout: replace raw-token-paste with a one-time server-issued invite code redeemed for a scoped token; add default secret-exclusion globs so credentials/keys never sync; give the dry-run a real skipped/sensitive-file review surface before first upload; issue contributor-only scoped tokens (delete already blocked) and verify the scope/clearance gates; harden the token registry from volume-JSON (src/auth.mjs + data/.ocean-bedrock/tokens.json) to a Postgres-backed table (db/004) so multi-replica is possible; package or document the GUI install; and ship a written operator onboarding + revoke/rotate runbook.

- 8 phases · 15 file targets · 7 DoD criteria · 6 open decisions · 7 risks

## Execution notes

- **Sequential:** do not start wave N+1 until wave N's `launchDefinitionOfDone` is met.
- Each wave opens with a phase-0 source-of-truth guardrail and closes with a security/launch-review phase.
- Deploy via the `railway up` runbook (HANDOFF sec 12). `npm run smoke` must stay green across every wave.
