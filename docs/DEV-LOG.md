# Ocean Bedrock Dev Log

Generated: 2026-06-11T03:41:35Z  
Repo: `/root/rt-nas/rt-nas`  
Service URL: <https://ocean-bedrock-production.up.railway.app>

> Freshness note: this is a chronological implementation log. Older counts, trees, and environment examples are historical unless explicitly restated in `docs/HANDOFF.md` or verified by the current check scripts.

## 2026-06-11 — Build/deploy baseline

### Project rename and framing

- Renamed the package/project from `rt-nas` to `ocean-bedrock`.
- Reframed the service as Ocean Bedrock: the cloud-box substrate for Ocean Longhouse support.
- Preserved canonical Ocean boundaries:
  - `ocean-daemon` is local runtime/body and execution authority.
  - `ocean-longhouse` is hive brain/coordination layer.
  - `ocean-bedrock` supports Longhouse, but does not redefine it.

Files:

```txt
package.json
README.md
docs/OCEAN-BEDROCK.md
docs/OCEAN-LONGHOUSE-DATA-PLANE.md
docs/OCEAN-LONGHOUSE.md
```

### HTTP server and file API

Built Node HTTP service in:

```txt
src/server.mjs
```

Core server features added:

```txt
GET    /
GET    /api
GET    /api/v1
GET    /health
GET    /api/v1/openapi.yaml
GET    /api/v1/info
GET    /api/v1/list
GET    /api/v1/tree
GET    /api/v1/stat
GET    /api/v1/file
PUT    /api/v1/file
DELETE /api/v1/file
POST   /api/v1/mkdir
POST   /api/v1/move
POST   /api/v1/copy
GET    /api/v1/search
GET    /api/v1/audit
```

Added landing route index after user hit `/` and got:

```json
{
  "ok": false,
  "error": "Route not found."
}
```

Fix shipped in Railway deployment:

```txt
54a961eb-4cf8-47cf-8d6b-791f1f324f61
```

Verified live:

```json
{
  "ok": true,
  "service": "ocean-bedrock",
  "instance": "ocean-bedrock",
  "version": "0.1.0",
  "note": "Most /api/v1 routes require Authorization: Bearer <token>."
}
```

### Auth, roles, scopes, locks

Built auth helper:

```txt
src/auth.mjs
```

Added token behaviors:

```txt
bearer token auth
x-ocean-bedrock-token fallback
path scopes
token issue/revoke
public token records without raw token values
```

Roles:

```txt
readonly     read
contributor  read/write/lock, no delete
readwrite    read/write/delete/lock
agent        read/write/delete/lock
admin        all plus admin token ops
```

Added locks:

```txt
GET    /api/v1/locks
POST   /api/v1/locks
DELETE /api/v1/locks/{lockId}
```

Security decision:

```txt
Default normal coworker role = contributor.
Contributor can write but cannot delete.
```

Validation:

```txt
operator-contributor token write succeeded
delete returned 403 as intended
```

### Ocean Ledger

Added ledger abstraction:

```txt
src/ledger.mjs
```

Ledger routes:

```txt
GET  /api/v1/ledger/events
POST /api/v1/ledger/events
GET  /api/v1/ledger/trace
POST /api/v1/ledger/snapshots
GET  /api/v1/ledger/verify
```

Store modes:

```txt
jsonl fallback/local mode
postgres production mode
```

Production uses:

```txt
OCEAN_LEDGER_STORE=postgres
```

Current live ledger event count from last check:

```txt
27
```

### Postgres migrations

Added migrations:

```txt
db/001_longhouse_core.sql
db/002_ocean_ledger.sql
```

Added helpers:

```txt
scripts/migrate.mjs
scripts/check-postgres.mjs
```

Applied migrations to Railway Postgres.

`npm run db:check` returned schema ready.

Verified tables include:

```txt
longhouse.mounts
longhouse.objects
longhouse.object_versions
longhouse.embedding_chunks
longhouse.graph_nodes
longhouse.graph_edges
longhouse.ingest_jobs
longhouse.ledger_events
longhouse.context_snapshots
```

### Cloudflare resources

Provisioned Cloudflare data plane resources:

```txt
R2 bucket:        ocean-longhouse
Vectorize index: ocean-longhouse-context
KV namespace:    OCEAN_LONGHOUSE_CACHE
```

Current integration status:

```txt
R2 resource exists, but object adapter not wired.
Vectorize resource exists, but embedding/upsert not wired.
KV resource exists, but hot-cache behavior not wired.
```

### Coworker bootstrap and local ingest

Added scripts:

```txt
scripts/ocean-bootstrap.mjs
scripts/ocean-ingest-local.mjs
```

NPM scripts:

```txt
npm run ocean:bootstrap
npm run ocean:ingest
```

Bootstrap writes local config and source manifest.

Ingest uploads selected files and writes run manifests.

Validation:

```json
{
  "scanned": 1,
  "skipped": 0,
  "unchanged": 0,
  "changed": 1,
  "uploaded": 1,
  "bytesUploaded": 85,
  "errors": 0
}
```

Generated current Bedrock paths:

```txt
/context/ocean-bedrock/sources/operator-contributor-workflow-smoke.json
/context/ocean-bedrock/manifests/operator-contributor-workflow-smoke-1781141239331.json
/coworkers/operator-contributor/workflow-smoke/contrib-notes/README.md
```

### MCP wrapper

Added MCP wrapper:

```txt
scripts/ocean-bedrock-mcp.mjs
docs/MCP.md
```

NPM script:

```txt
npm run mcp
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

MCP resources:

```txt
ocean-bedrock://docs
ocean-bedrock://context
ocean-bedrock://coworkers
ocean-bedrock://sessions
ocean-bedrock://handoffs
```

Smoke validation:

```txt
initialize OK
tools/list OK, 10 tools
bedrock_list OK
bedrock_snapshot OK
```

### Metadata and ingest worker

Added metadata/worker module:

```txt
src/metadata.mjs
scripts/ocean-ingest-worker.mjs
```

NPM script:

```txt
npm run ocean:worker
```

Server now records object writes/deletes to Postgres and queues ingest jobs.

Worker pipeline:

```txt
claim queued ingest job
read file from persistent volume
skip non-text/too-large files
chunk text
insert chunks into longhouse.embedding_chunks
upsert file node into longhouse.graph_nodes
update object metadata
complete job
```

Production enabled with:

```txt
OCEAN_BEDROCK_WORKER_ENABLED=true
```

Validation:

```txt
/coworkers/operator-test/workflow-smoke/local-notes/indexed-smoke.md -> 1 chunk
/coworkers/operator-contributor/workflow-smoke/contrib-notes/README.md -> 1 chunk
```

Current chunk state:

```txt
embedding_provider: none
embedding_model:    text-chunk-v1
dimensions:         0
vectorized:         0
```

Important limitation:

```txt
This is a chunk index, not vector semantic search yet.
```

### Railway deployment

Railway auth was initially expired:

```txt
invalid_grant; run railway login again
```

After user indicated sign-in was complete, Railway work resumed.

Created/configured dedicated service:

```txt
service: ocean-bedrock
service ID: b117c870-d503-4cb3-8653-c094b50b6fbb
volume: ocean-bedrock-volume mounted at /data
public URL: https://ocean-bedrock-production.up.railway.app
```

Deployment history:

```txt
4116f466-af55-4e70-b3d1-0c2600e763fb initial service deploy
d04ef61f-3d47-4073-914e-4fedbd387a80 metadata worker deploy
5d31071a-0634-4c41-a261-c95bca5feba9 contributor role deploy
54a961eb-4cf8-47cf-8d6b-791f1f324f61 landing route deploy
```

Health verified:

```bash
curl https://ocean-bedrock-production.up.railway.app/health
```

Returned:

```json
{
  "ok": true,
  "instance": "ocean-bedrock",
  "version": "0.1.0"
}
```

### Tokens issued/tested

Local token files:

```txt
/root/.config/ocean-bedrock/railway-admin-token.txt
/root/.config/ocean-bedrock/operator-test-token.txt
/root/.config/ocean-bedrock/operator-contributor-token.txt
```

Token tests performed:

```txt
bootstrap admin token stored/set as Railway secret
operator-test-mcp token issued and smoke tested
temporary token create/revoke path verified
operator-contributor-mcp token issued
contributor write succeeded
contributor delete failed with 403
```

Do not commit tokens.

### Current live filesystem inventory

Admin tree snapshot, depth 5:

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
/handoffs
/sessions/README.md
/sessions/operator-contributor
/shared
/vault/README.md
```

Contributor-visible scopes:

```txt
/coworkers/operator-contributor
/context/ocean-bedrock
/sessions/operator-contributor
```

### Source adapter precedent research

User asked for precedent for a source adapter table/config.

Created:

```txt
docs/SOURCE-ADAPTER-PRECEDENTS.md
```

Research covered:

```txt
Airbyte: connector spec + catalog + state
Singer/Meltano: config + catalog + state
Kafka Connect/Debezium: connector configs + offsets + schema history
Dagster: external resource config
LangChain/LlamaIndex: loaders/readers to Document + metadata
Unstructured: source connectors + standardized metadata
```

Recommended Ocean Bedrock tables:

```txt
longhouse.source_adapters
longhouse.source_instances
longhouse.source_streams
longhouse.source_sync_runs
longhouse.source_records
```

Recommended lineage:

```txt
source_instance -> source_stream -> source_record -> object -> chunks -> graph -> ledger
```

Status:

```txt
research complete
implemented in db/003_source_adapters.sql and src/sources.mjs
local_folder bootstrap/ingest wiring live
```

### Source adapter registry implementation

Added migration and helper layer:

```txt
db/003_source_adapters.sql
src/sources.mjs
```

Tables added/applied to Railway Postgres:

```txt
longhouse.source_adapters
longhouse.source_instances
longhouse.source_streams
longhouse.source_sync_runs
longhouse.source_records
```

Seeded adapters:

```txt
local_folder
github
telegram
slack
notion
linear
google_drive
r2
```

Updated scripts:

```txt
scripts/migrate.mjs              now applies db/003 by default
scripts/check-postgres.mjs       verifies source tables + 8 seeded adapters
scripts/ocean-bootstrap.mjs      can write source_instance/source_stream directly when run with DATABASE_URL
scripts/ocean-ingest-local.mjs   now uses server-side source/sync HTTP endpoints for lineage
scripts/smoke-test.mjs           keeps local smoke ephemeral, then read-only verifies source adapter tables when DATABASE_URL exists
```

Verification:

```txt
npm run db:migrate -- --yes   passed
npm run db:check              schemaReady=true, sourceAdaptersSeeded=8
npm run smoke                 passed
local_folder registry smoke   bootstrap enabled, ingest enabled, source_record linked to object_id
```

Operational note:

```txt
operator-contributor token was rotated after implementation testing; old token id tok_83b7ebc4e643 revoked, replacement stored in /root/.config/ocean-bedrock/operator-contributor-token.txt
```

### Wiki/handoff/dev-log docs

User asked for three markdown docs fully breaking it down.

Created:

```txt
docs/WIKI.md
docs/HANDOFF.md
docs/DEV-LOG.md
```

Intent:

```txt
docs/WIKI.md     stable operational/project wiki
docs/HANDOFF.md  next-agent/operator handoff packet
docs/DEV-LOG.md  chronological build and deployment log
```

### Local GUI companion app V0

Added a simple local GUI app:

```txt
scripts/ocean-local-app.mjs
docs/LOCAL-GUI-APP.md
package.json script: npm run ocean:app
```

Intent:

```txt
coworker opens local app -> pastes scoped token once -> chooses folders -> chooses manual/scheduled sync
```

Current V0 behavior:

```txt
binds to 127.0.0.1:8765
opens browser automatically
stores ~/.config/ocean-bedrock/bootstrap.json with 0600 permissions
supports native folder picker where available
runs existing ocean-ingest-local.mjs for sync
supports manual sync and interval schedule while app is open
shows selected folders, run output, and recent activity
```

Limitations:

```txt
not packaged as a double-click desktop app yet
schedules only run while app process is open
source/sync lineage now depends on the server having DATABASE_URL configured
non-local integrations are placeholders in the UI
```

## Current tests/run evidence

### Local smoke

```bash
npm run smoke
```

Passed after landing route edit:

```txt
ocean-bedrock smoke test passed
```

### Node syntax check

```bash
node --check src/server.mjs
```

Passed.

### Public routes

Verified:

```txt
GET /        ok true
GET /api     ok true
GET /api/v1  ok true
GET /health  ok true
```

### DB counts from last check

```json
{
  "objects": [{ "kind": "file", "count": 19 }],
  "chunks": [{ "count": 17 }],
  "jobs": [
    { "status": "done", "count": 17 },
    { "status": "failed", "count": 9 }
  ],
  "ledgerEvents": [{ "count": 89 }],
  "graphNodes": [{ "node_type": "file", "count": 17 }],
  "sourceAdapters": [{ "count": 8 }],
  "sourceInstances": [{ "count": 6 }],
  "sourceStreams": [{ "count": 6 }],
  "sourceSyncRuns": [
    { "status": "cancelled", "count": 3 },
    { "status": "completed", "count": 2 }
  ],
  "sourceRecords": [{ "count": 2 }]
}
```

### Chunk/vector status

```json
{
  "embedding_provider": "none",
  "embedding_model": "text-chunk-v1",
  "dimensions": 0,
  "count": 17,
  "vectorized": 0
}
```

## Design decisions logged

### Use contributor role by default

Reason:

- coworkers need to add material,
- delete is higher-risk,
- contributor can read/write/lock but cannot delete.

### Keep source ingest opt-in

Reason:

- coworker devices may contain secrets/noise,
- explicit selection is safer,
- manifests make lineage auditable.

### Use Postgres for metadata/ledger production source

Reason:

- queryable,
- durable,
- supports future source adapters and semantic index state,
- JSONL remains fallback/local mode.

### Treat Vectorize/KV as rebuildable indexes

Reason:

- source bytes + Postgres metadata are canonical,
- vector/cache indexes can be regenerated.

### Do not put raw tokens in docs

Reason:

- markdown files may be committed or shared,
- token files stay outside repo.

## 2026-06-12 — Server-side source/sync API

Added server-owned source registry and sync-run endpoints so local/coworker devices no longer need direct Postgres credentials.

New API routes:

```txt
GET    /api/v1/sources/adapters
POST   /api/v1/sources/instances
GET    /api/v1/sources/instances
GET    /api/v1/sources/instances/{id}
PATCH  /api/v1/sources/instances/{id}
POST   /api/v1/sources/streams
GET    /api/v1/sources/streams
PATCH  /api/v1/sources/streams/{id}
POST   /api/v1/sync-runs
GET    /api/v1/sync-runs/{id}
POST   /api/v1/sync-runs/{id}/complete
POST   /api/v1/sync-runs/{id}/fail
POST   /api/v1/sync/local-folder/plan
POST   /api/v1/sync/local-folder/records:batch
POST   /api/v1/sync/local-folder/commit
```

Updated local ingest:

- `scripts/ocean-ingest-local.mjs` now plans lineage through `/api/v1/sync/local-folder/plan`.
- Uploaded file records are batched through `/api/v1/sync/local-folder/records:batch`.
- Final stats are committed through `/api/v1/sync/local-folder/commit`.
- Coworker devices only need the Bedrock URL and scoped token; `DATABASE_URL` stays on the server.

Verified locally and live:

```txt
node --check src/server.mjs
node --check src/sources.mjs
node --check scripts/ocean-ingest-local.mjs
npm run smoke
npm run db:check
Railway deploy: 202428df-c989-4ec2-b730-6e0598633877 SUCCESS
live /health: ok
live /api/v1/sources/adapters: 8 adapters
live operator-contributor local ingest: sync_run_id 86ed8bdd-d108-4340-a65c-6d6af4831cd8 completed, source_record linked to object_id
```

## 2026-06-12 — Ocean Context semantic/graph/toolbox staging

Added the first Ocean Context wave:

- Cloudflare Workers AI embedding client (`src/semantic.mjs`) using `@cf/baai/bge-base-en-v1.5`.
- Cloudflare Vectorize upsert/query support for `ocean-longhouse-context`.
- `/api/v1/semantic/search` endpoint with lexical chunk fallback.
- `embed_object` ingest job support after `index_object`.
- Lightweight graph extraction (`src/graph.mjs`) and `/api/v1/graph/*` endpoints.
- Ocean Context daily triage (`src/ocean-context.mjs`) and `npm run ocean:triage`.
- `/api/v1/toolbox/manifest` for staging coworker computer auth, MCP, skills, and local companion setup.
- MCP tools for semantic search, graph neighborhood, toolbox manifest, and daily triage.

New docs:

```txt
docs/OCEAN-CONTEXT.md
docs/OCEAN-TOOLBOX-BOOTSTRAP.md
```

Cloudflare/live smoke:

```txt
Workers AI embedding dims: 768
Vectorize upsert mutation accepted
Vectorize query returned the smoke vector after async index propagation
Railway deploy: db04d2f4-e87b-406b-80e5-c4a9c254272a SUCCESS
live /api/v1/info semantic.enabled: true
live /api/v1/semantic/search: vectorize results returned
live /api/v1/graph/neighborhood: nodes/edges returned
live /api/v1/ocean-context/triage/daily: /context/ocean-bedrock/triage/2026-06-12.md
DB after triage: chunks=31 vectorized=31 pending=0 graphNodes=59 graphEdges=57
```

## 2026-06-15 — Triage cleanup + Telegram alert wiring

Operational cleanup after the triage scheduler wave:

- Historical failed ingest jobs are resolved: 9 stale `/docs/smoke/hello.md` `index_object` rows were abandoned/cancelled.
- Forced live triage wrote `/context/ocean-bedrock/triage/2026-06-15.md` and reported no immediate issues.
- Telegram alert variables are set on `ocean-bedrock`; one-time verification send returned Telegram API 200.
- `ocean-bedrock` redeployed successfully after env wiring: `af2a2949-337f-494c-af95-17285e166c43`.
- Latest source deploy after mount containment fix/docs: `d9984f8a-3d0f-4c98-8da3-3387b2ea32e1`.

Live verification:

```txt
failedJobs=0
staleFailedJobs=0
queuedJobs=0
pendingChunks=0
semantic chunks=55 vectorized=55 pending=0
graphNodes=84 graphEdges=82
```

## Known open gaps

```txt
no R2 object bytes adapter
no GitHub/Telegram/Slack/Notion/Linear adapter runners yet
no automatic local watcher daemon
no encrypted vault/secrets manager
source registry is V1 local_folder HTTP wiring only
entity/relationship extraction is still heuristic
```

## Next development sequence

Recommended sequence:

1. Continue Wave 2/3 hardening: use the mount router as the stable data-plane boundary.
2. Add real source runners, starting with GitHub and Telegram.
3. Prepare real coworker rollout checklist and run a small real folder dry-run.
4. Package/download local companion GUI and one-time invite flow.
5. Add richer graph/entity extraction and graph/semantic result fusion.
6. Add R2 adapter or decide volume-first remains acceptable for V1.
7. Perform security review before real coworker rollout.

## Quick command appendix

Set live env:

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"
```

Health:

```bash
curl "$OCEAN_BEDROCK_URL/health"
```

Info:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" "$OCEAN_BEDROCK_URL/api/v1/info"
```

Tree:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/tree?path=/coworkers/operator-contributor&depth=5"
```

MCP:

```bash
npm run mcp
```

Bootstrap:

```bash
npm run ocean:bootstrap
```

Ingest dry run:

```bash
npm run ocean:ingest -- --dry-run
```

Ingest:

```bash
npm run ocean:ingest
```

DB check:

```bash
npm run db:check
```

Smoke:

```bash
npm run smoke
```

## 2026-07-08 — Onboarding E2E real; OceanOrb + design system extracted

- Finished and hardened the real-scan ceremony conductor
  (`scripts/ocean-ceremony-scan.mjs`): true de-duplicated source counts, sync
  POST gated on confirmed human ack, honest timeout narration (ack/sync frames
  dropped when no human acked). Live E2E verified against a throwaway app
  instance: full arc streamed, gate genuinely blocked, sync skipped on timeout.
- Extracted the OceanOrb (water-surface sphere, voice states, logo absorption)
  to `design/orb/ocean-orb.js` as the canonical reusable module; onboarding
  mounts it via a thin facade. Design tokens + self-hosted InterVariable +
  glass UI landed under `design/`; `assets/integrations/omp.svg` added so all
  four detected sources have absorption marks.
- Rebuilt the operator panel at `ui/app/index.html` (root route) after the
  cleanup wave deleted the never-committed inline GUI; ceremony now lives only
  at `/onboarding`. Panel verified in-browser against real config (sources,
  preview state, ack gate, schedule; zero console errors, Inter live).

## 2026-07-16 — Gate-2 S1A: federated room membership + authz (feat/s1a-federated-rooms)

- `db/006_federated_rooms.sql`: `longhouse.room_members` with composite
  same-room/same-principal FK, actor-shape CHECKs, one-active-owner /
  one-active-human / registration-key partial uniques, and a DEFERRABLE
  INITIALLY DEFERRED owner-guard trigger (active agents must reference an
  active user owner; removal cannot orphan). Transaction-wrapped, idempotent
  on second apply.
- `src/rooms.mjs`: room store — atomic register (correlation claim marked
  `federation_kind=room`), self-join with in-transaction consumed-invite
  provenance + tombstone lock (no revoked-token resurrection), batch agent
  registration keyed by daemon-supplied registration_key (all-or-nothing,
  201 all-new / 200 retry-mixed, changed-content 409), locked in-transaction
  removal authorization, fail-closed safe projections.
- `src/server.mjs`: single-shared-Postgres F3 gate (resolved options/env,
  URLs never logged, split state → 503), routes R-1..R-7 with strict JSON
  object bodies, room-owner invite branch (derived contributor role + exact
  scope), and R-8 ledger gating: canonical-pair writes with owned
  actor_member_id, descendant-path detection, per-row global-read
  suppression (admin does not bypass), gate ordered before generic scope so
  the frozen mismatch 400 wins.
- Tests: 61 DB-free (validators, projections, suppression helper, file/JSONL
  HTTP 503/400 pins) plus `scripts/rooms-postgres-smoke.mjs` — quadruple-
  gated disposable-Postgres protocol gate (94 checks green, reviewer-executed
  on a fresh disposable PG14 `_test` database): exact-status races,
  SQLSTATE-verified direct-SQL constraint pins, split-Postgres F3,
  create-only cleanup with verbatim abort on any preexisting `longhouse`
  schema.
- Reviewed continuously by codex against the frozen
  `gate2-s1a-freeze.md` v4 (ocean-surface `.stitchpad/artifacts/`); store
  gate approved at b9f9df0, route/R-8 gate through c2a9363, PG protocol gate
  through e4cb564; ocean independent trace APPROVE with no findings.
