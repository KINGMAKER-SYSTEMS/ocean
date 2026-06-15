# Ocean Bedrock Handoff

Generated: 2026-06-15T00:00:00Z  
Repo: `/root/rt-nas/rt-nas`  
Service: `ocean-bedrock`  
Public URL: <https://ocean-bedrock-production.up.railway.app>

## 1. Handoff summary

Ocean Bedrock has been built and deployed as the cloud-box substrate for Ocean Longhouse support.

Current baseline is live:

```txt
health endpoint:     live
Postgres ledger:     live
shared file API:     live  (Wave 2: routed via mount adapter)
scoped bearer auth:  live
MCP wrapper:         live
coworker bootstrap: live
ingest worker:       live for text chunking
semantic vectors:    not yet live
mount registry:      live — /api/v1/mounts (admin-gated), 7 seeded mounts
data-plane router:   live — all file ops route through StorageAdapter interface
```

The system is usable today for:

- scoped file storage,
- coworker folder ingest,
- agent/MCP read/write/search,
- ledger events and snapshots,
- text chunk indexing into Postgres.

It is **not yet** a full vector/semantic knowledge engine because embeddings and Vectorize upserts are not implemented.

## 2. Critical guardrails

Do not violate these:

1. Do not redefine Ocean Longhouse in Bedrock docs.
2. Canonical split:
   - `ocean-daemon` = local runtime/body/execution authority.
   - `ocean-longhouse` = hive brain/coordination layer.
   - `ocean-bedrock` = cloud-box support substrate.
3. Do not hallucinate Ocean docs. Use canonical docs when discussing Ocean:
   - `ocean-os/docs/LONGHOUSE.md`
   - `ocean-os/docs/LONGHOUSE_ORCHESTRATION.md`
   - relevant `ocean-surface` docs for Surface-specific claims.
4. Do not commit or paste raw secrets into docs.
5. Coworker ingest must remain opt-in folder selection.
6. Coworker/agent tokens should be scoped bearer/MCP tokens, not admin tokens.
7. Avoid ingesting secrets by default.

## 3. Deployment facts

```txt
Railway project:       rising-tides-agents
Railway service:       ocean-bedrock
Railway service ID:    b117c870-d503-4cb3-8653-c094b50b6fbb
Public URL:            https://ocean-bedrock-production.up.railway.app
Volume:                ocean-bedrock-volume
Volume mount:          /data
Node package:          ocean-bedrock@0.1.0
Runtime:               node src/server.mjs
```

Important deployment IDs:

```txt
4116f466-af55-4e70-b3d1-0c2600e763fb   initial successful service deploy
d04ef61f-3d47-4073-914e-4fedbd387a80   metadata/worker deploy
5d31071a-0634-4c41-a261-c95bca5feba9   contributor role deploy
54a961eb-4cf8-47cf-8d6b-791f1f324f61   landing route deploy
202428df-c989-4ec2-b730-6e0598633877   source/sync API deploy
db04d2f4-e87b-406b-80e5-c4a9c254272a   Ocean Context semantic/graph/toolbox deploy
```

Production env vars known to be set:

```txt
OCEAN_BEDROCK_ROOT=/data
OCEAN_BEDROCK_INSTANCE=ocean-bedrock
OCEAN_LEDGER_STORE=postgres
OCEAN_BEDROCK_WORKER_ENABLED=true
DATABASE_URL=<Railway Postgres URL>
OCEAN_BEDROCK_BOOTSTRAP_TOKEN=<secret>
CLOUDFLARE_ACCOUNT_ID=<set>
CLOUDFLARE_API_TOKEN=<secret>
OCEAN_VECTORIZE_INDEX=ocean-longhouse-context
OCEAN_EMBEDDING_MODEL=@cf/baai/bge-base-en-v1.5
OCEAN_EMBEDDING_DIMENSIONS=768
OCEAN_EMBEDDING_POOLING=cls
```

## 4. Token locations

Local token files on Lennox:

```txt
/root/.config/ocean-bedrock/railway-admin-token.txt
/root/.config/ocean-bedrock/operator-test-token.txt
/root/.config/ocean-bedrock/operator-contributor-token.txt
```

Do not commit these files. Do not copy the admin token into docs.

Normal operator/coworker testing should use:

```txt
/root/.config/ocean-bedrock/operator-contributor-token.txt
```

That token role is `contributor`:

```txt
can:    read, write, lock
cannot: delete, admin token ops
```

Scopes:

```txt
/coworkers/operator-contributor
/sessions/operator-contributor
/context/ocean-bedrock
```

## 5. Current repo state

Git state at handoff:

```txt
Baseline committed: a4e3133 Build and deploy Ocean Bedrock baseline
Source helper committed: 288cda2 feat(sources): add source adapter helper module
Source registry/local app committed: 639491b Add Ocean Bedrock source registry and local sync app
Source/sync HTTP API committed: b526d02 Add Ocean Bedrock source sync API
```

Important source-registry paths:

```txt
db/003_source_adapters.sql
src/sources.mjs
scripts/migrate.mjs
scripts/check-postgres.mjs
scripts/ocean-bootstrap.mjs
scripts/ocean-ingest-local.mjs
scripts/ocean-local-app.mjs
scripts/smoke-test.mjs
docs/LOCAL-GUI-APP.md
docs/WIKI.md
docs/HANDOFF.md
docs/DEV-LOG.md
```

Recommended next operator action:

```bash
# Source/sync API is committed, pushed, deployed, and live-smoked.
# Next: build embeddings/vector search or prepare a real coworker rollout checklist.
git status --short
```

## 6. Verification commands

Set env:

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"
```

Public route index:

```bash
curl "$OCEAN_BEDROCK_URL/"
```

Health:

```bash
curl "$OCEAN_BEDROCK_URL/health"
```

Authenticated info:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/info"
```

List scoped workspace:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/tree?path=/coworkers/operator-contributor&depth=5"
```

Write a file:

```bash
echo "handoff smoke" | curl -X PUT \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  --data-binary @- \
  "$OCEAN_BEDROCK_URL/api/v1/file?path=/coworkers/operator-contributor/handoff-smoke.md"
```

Search:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/search?q=handoff&path=/coworkers/operator-contributor&limit=10"
```

Delete should fail for contributor:

```bash
curl -i -X DELETE \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/file?path=/coworkers/operator-contributor/handoff-smoke.md"
```

Expected:

```txt
403 Token role "contributor" does not allow delete.
```

## 7. MCP verification

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"
npm run mcp
```

Already smoke-tested successfully:

```txt
initialize OK
tools/list OK, 10 tools
bedrock_list OK
bedrock_snapshot OK
```

MCP tools:

```txt
bedrock_info
bedrock_list
bedrock_read
bedrock_write
bedrock_mkdir
bedrock_search
bedrock_lock
bedrock_unlock
bedrock_trace
bedrock_snapshot
```

## 8. Database verification

Schema check:

```bash
npm run db:check
```

Expected:

```json
{
  "schemaReady": true,
  "tables": {
    "mounts": "longhouse.mounts",
    "ledgerEvents": "longhouse.ledger_events",
    "contextSnapshots": "longhouse.context_snapshots",
    "sourceAdapters": "longhouse.source_adapters",
    "sourceInstances": "longhouse.source_instances",
    "sourceStreams": "longhouse.source_streams",
    "sourceSyncRuns": "longhouse.source_sync_runs",
    "sourceRecords": "longhouse.source_records"
  },
  "sourceAdaptersSeeded": 8
}
```

Current DB state from last check:

```txt
objects:       19 files
chunks:        17
jobs done:     17
jobs failed:   9
ledgerEvents: 89
graphNodes:   17 file nodes
sourceAdapters: 8 seeded
sourceInstances: 6
sourceStreams: 6
sourceSyncRuns: 2 completed, 3 cancelled debug runs
sourceRecords: 2
```

Chunk status:

```txt
embedding_provider=none
embedding_model=text-chunk-v1
dimensions=0
vectorized=0
```

This confirms chunk indexing exists, but semantic vector search does not.

## 9. Current data inventory

Global tree snapshot:

```txt
/context/README.md
/context/ocean-bedrock/manifests/operator-contributor-workflow-smoke-1781141239331.json
/context/ocean-bedrock/manifests/operator-test-workflow-smoke-1781140704660.json
/context/ocean-bedrock/sources/operator-contributor-workflow-smoke.json
/context/ocean-bedrock/sources/operator-test-workflow-smoke.json
/coworkers/operator-contributor/security/delete-check.md
/coworkers/operator-contributor/workflow-smoke/contrib-notes/README.md
/coworkers/operator-test/workflow-smoke/local-notes/README.md
/coworkers/operator-test/workflow-smoke/local-notes/context.json
/coworkers/operator-test/workflow-smoke/local-notes/indexed-smoke.md
/docs/README.md
/sessions/README.md
/vault/README.md
```

Empty/present directories:

```txt
/handoffs
/shared
/sessions/operator-contributor
```

## 10. Source adapter registry status

Research doc created:

```txt
docs/SOURCE-ADAPTER-PRECEDENTS.md
```

Precedents reviewed:

```txt
Airbyte: connector spec + catalog + state
Singer/Meltano: config + catalog + state
Kafka Connect/Debezium: connector config + offsets + schema history
Dagster: configurable external resources
LangChain/LlamaIndex: loaders/readers -> document + metadata
Unstructured: source connectors + standardized metadata
```

Implemented live tables:

```txt
longhouse.source_adapters
longhouse.source_instances
longhouse.source_streams
longhouse.source_sync_runs
longhouse.source_records
```

Implementation status:

```txt
db/003_source_adapters.sql applied to Railway Postgres
8 adapter definitions seeded
src/sources.mjs helper module added
server exposes /api/v1/sources/* and /api/v1/sync/* lineage endpoints
scripts/ocean-bootstrap.mjs writes/redacts source manifests and can directly seed registry when run by operator
scripts/ocean-ingest-local.mjs writes source_sync_run/source_record lineage through HTTP, no local DATABASE_URL required
```

Verified smoke:

```txt
local checks: node --check, npm run smoke, npm run db:check
live health: https://ocean-bedrock-production.up.railway.app/health ok
live adapters endpoint: 8 adapters returned
operator-contributor local_folder ingest through HTTP source/sync endpoints: enabled
source_record created and linked to longhouse.objects.id
latest live smoke sync_run_id: 86ed8bdd-d108-4340-a65c-6d6af4831cd8
latest live smoke source_instance_id: dfa0db73-61aa-405c-bfcd-10584ac478f7
```

## 11. Next work queue

### Priority 0 — Wave 2 data-plane mount router (SHIPPED)

Mount registry + StorageAdapter interface + path router + admin `/api/v1/mounts` API. All file operations (stat, list, read, write, delete, move, copy, mkdir, search) now route through a duck-typed `StorageAdapter`. Local adapter is `LocalStorageAdapter`; R2/other mounts return a `createUnimplementedAdapter` stub (HTTP 501). No-DB fallback uses a synthetic `managed-local` mount so `npm run smoke` stays green.

Deployment ID: `bc7f1ca2-2eff-46af-ba63-d3eab80c5832`

Verified live:
- `GET /api/v1/mounts` returns 7 mounts (6 R2 + managed-local)
- `GET /api/v1/stat?path=/` routes through `LocalStorageAdapter`
- `GET /api/v1/list?path=/` returns directory entries via adapter
- `npm run smoke` green

New files: `src/mounts/adapter.mjs`, `src/mounts/adapters/local.mjs`, `src/mounts/registry.mjs`, `src/mounts/router.mjs`

### Priority 1 — Automate daily Ocean Context triage (SHIPPED)

Dedicated Railway cron service `ocean-bedrock-triage-cron` runs `npm run ocean:triage` twice daily (`0 6,18 * * *` UTC). Triage is idempotent per slot (key `triage:<day>:<slot>`). Findings route to a slot-stamped alert digest at `/context/ocean-bedrock/triage/_alerts/<day>-<slot>.md`. Thresholds configurable via `OCEAN_TRIAGE_FAILED_ALERT` / `OCEAN_TRIAGE_QUEUE_ALERT`. Failed-job tooling live: `npm run ocean:jobs`. See `docs/OCEAN-TRIAGE-OPS.md` for full runbook.

Remaining: triage the 9 historical failed ingest jobs (list → retry or abandon using `npm run ocean:jobs`). Telegram/webhook notification channels are stubbed OFF; promote to active when ready.

### Priority 2 — Real coworker rollout prep

A V0 local GUI app exists:

```bash
npm run ocean:app
```

Before issuing real coworker tokens:

- confirm path scopes,
- document install instructions,
- package the GUI/download flow,
- add one-time invite exchange instead of raw token paste,
- run a small real folder dry-run,
- inspect skipped files,
- verify secrets are ignored,
- issue contributor-only tokens,
- confirm delete is blocked.

### Priority 3 — Richer graph/entity extraction

Current graph extraction is heuristic. Next add:

- people/project/client/task entities,
- stronger source-record to ledger correlation,
- graph-backed context packs for agents,
- graph/semantic result fusion.

### Priority 4 — R2 object adapter

Existing Cloudflare R2 bucket:

```txt
ocean-longhouse
```

Need:

```txt
R2 S3 access keys
Railway env vars
adapter for canonical object bytes
backfill/migration strategy from volume to R2 if desired
```

## 12. Railway deploy runbook

Use Railway skill practices. From repo root:

```bash
export RAILWAY_CALLER="skill:use-railway@1.2.1"
export RAILWAY_AGENT_SESSION="railway-skill-$(date -u +%Y%m%d)-ocean-bedrock"
env -u RAILWAY_SERVICE_ID -u RAILWAY_SERVICE_NAME \
  railway up --service ocean-bedrock --environment production --detach \
  -m "Deploy Ocean Bedrock changes"
```

Check status:

```bash
env -u RAILWAY_SERVICE_ID -u RAILWAY_SERVICE_NAME \
  railway service status --service ocean-bedrock --json
```

After deploy:

```bash
curl -fsS https://ocean-bedrock-production.up.railway.app/health
```

## 13. Known issues

1. Semantic search + Vectorize are live, but graph/entity extraction is still heuristic.
2. R2 bucket exists but object adapter/keys are not wired.
3. 9 failed ingest jobs exist from earlier worker attempts; inspect before production cleanup.
4. Token registry is volume JSON, not Postgres; okay for one Railway replica, not ideal for multi-replica.
5. `/vault` is just a folder placeholder, not an encrypted secrets vault.
6. `/api/v1/search` is still plain file text search; semantic search lives at `/api/v1/semantic/search`.
7. Current graph extracts file/directory/heading/topic/link/source-lineage nodes, but not rich business entities yet.
8. Source registry currently has V1 local_folder wiring only; GitHub/Telegram/Slack/Notion/Linear/Drive/R2 adapter runners are not implemented yet.
9. Daily Ocean Context triage: dedicated Railway cron service deployed (`0 6,18 * * *` UTC). Failed-job tooling live (`npm run ocean:jobs`). Historical failed ingest_jobs rows likely still present — run `npm run ocean:jobs -- list` and retry/abandon, then verify `failedJobs=0` via next triage report.

## 14. Acceptance criteria for next handoff

The next handoff should include:

- commit hash,
- latest Railway deployment ID,
- current live health output,
- DB counts,
- exact migration status,
- whether embeddings/vectorize are live,
- source adapter tables status,
- any issued coworker tokens by name/scope, not raw token values.
