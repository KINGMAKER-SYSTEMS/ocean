# Ocean Bedrock Wiki

Generated: 2026-06-11T03:41:35Z  
Service: `ocean-bedrock`  
Public URL: <https://ocean-bedrock-production.up.railway.app>

> Freshness note: this wiki is a generated/historical snapshot. Use `docs/HANDOFF.md` plus live verification commands for current deployment facts and counts.

## 1. What Ocean Bedrock is

Ocean Bedrock is the **cloud-box support substrate** for Ocean Longhouse.

It provides:

- authenticated shared files,
- scoped coworker/agent workspaces,
- local folder bootstrap and ingest,
- Ocean Ledger events/history,
- MCP tools for agents,
- Postgres metadata/index tables,
- background context chunking,
- a foundation for future source adapters and semantic search.

It does **not** redefine Ocean Longhouse.

Canonical split:

```txt
ocean-daemon      = local runtime/body and execution authority
ocean-longhouse   = hive brain / coordination layer
ocean-bedrock     = cloud-box support substrate for files, context, ledger, and MCP access
```

## 2. Current live status

The Railway deployment is live.

```txt
URL:       https://ocean-bedrock-production.up.railway.app
Project:   rising-tides-agents
Service:   ocean-bedrock
Version:   0.1.0
Store:     Railway volume + Postgres ledger/metadata
```

Public route index:

```bash
curl https://ocean-bedrock-production.up.railway.app/
```

Health:

```bash
curl https://ocean-bedrock-production.up.railway.app/health
```

Authenticated info:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  https://ocean-bedrock-production.up.railway.app/api/v1/info
```

## 3. What is currently on the shared filesystem

Historical global tree snapshot, depth 5. This is not guaranteed to match the current repo or live filesystem:

```txt
/context
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

The normal contributor token is scoped and cannot list `/`. It can see:

```txt
/coworkers/operator-contributor
/context/ocean-bedrock
/sessions/operator-contributor
```

## 4. Auth model

Ocean Bedrock uses bearer tokens.

```bash
Authorization: Bearer <token>
```

Token records live in the persistent Bedrock auth file on the service volume. Local copies for the operator are stored outside the repo:

```txt
/root/.config/ocean-bedrock/railway-admin-token.txt
/root/.config/ocean-bedrock/operator-test-token.txt
/root/.config/ocean-bedrock/operator-contributor-token.txt
```

Do **not** commit raw tokens.

### Roles

| Role | Permissions | Intended use |
| --- | --- | --- |
| `readonly` | read | observers, safe viewers |
| `contributor` | read, write, lock | normal coworker/agent contribution; no delete |
| `readwrite` | read, write, delete, lock | trusted operator or controlled automation |
| `agent` | read, write, delete, lock | trusted autonomous agent |
| `admin` | all plus token admin | service owner/operator only |

Default coworker role should be `contributor`.

### Scopes

A token can only access paths inside its scope list.

Example contributor scope:

```txt
/coworkers/operator-contributor
/sessions/operator-contributor
/context/ocean-bedrock
```

This is why listing `/` with that token returns `403`.

## 5. Core filesystem model

Default folders:

```txt
/docs       durable documentation
/context    reusable context, source manifests, knowledge-layer artifacts
/sessions   session-specific notes and scratch state
/handoffs   cross-agent handoff packets
/shared     general shared files
/vault      sensitive placeholder; not a secrets manager yet
/coworkers  coworker-specific uploaded/synced material
```

Files are addressed by POSIX-style virtual paths:

```txt
/coworkers/<person>/<device-or-run>/<folder-label>/file.md
/context/ocean-bedrock/sources/<source>.json
/sessions/<person-or-agent>/<date>/notes.md
```

Bedrock maps those virtual paths onto its persistent Railway volume rooted at `/data` in production.

## 6. HTTP API

Public, unauthenticated:

```txt
GET /                    landing route index
GET /api                 route index
GET /api/v1              route index
GET /health              health check
GET /api/v1/openapi.yaml OpenAPI document
```

Authenticated routes:

```txt
GET    /api/v1/info
GET    /api/v1/list?path=/docs&depth=1
GET    /api/v1/tree?path=/docs&depth=5
GET    /api/v1/stat?path=/docs/file.md
GET    /api/v1/file?path=/docs/file.md&inline=1
PUT    /api/v1/file?path=/docs/file.md
DELETE /api/v1/file?path=/docs/file.md
POST   /api/v1/mkdir
POST   /api/v1/move
POST   /api/v1/copy
GET    /api/v1/search?q=term&path=/docs&limit=50
GET    /api/v1/ledger/events
POST   /api/v1/ledger/events
GET    /api/v1/ledger/trace?correlation_id=...
POST   /api/v1/ledger/snapshots
GET    /api/v1/ledger/verify
GET    /api/v1/semantic/search?q=...&path=...
GET    /api/v1/graph/nodes?path=...
GET    /api/v1/graph/neighborhood?path=...
POST   /api/v1/ocean-context/triage/daily
GET    /api/v1/toolbox/manifest
GET    /api/v1/sources/adapters
GET    /api/v1/sources/instances
POST   /api/v1/sources/instances
GET    /api/v1/sources/instances/{id}
PATCH  /api/v1/sources/instances/{id}
GET    /api/v1/sources/streams
POST   /api/v1/sources/streams
PATCH  /api/v1/sources/streams/{id}
GET    /api/v1/sync-runs/{id}
POST   /api/v1/sync-runs
POST   /api/v1/sync-runs/{id}/complete
POST   /api/v1/sync-runs/{id}/fail
POST   /api/v1/sync/local-folder/plan
POST   /api/v1/sync/local-folder/records:batch
POST   /api/v1/sync/local-folder/commit
GET    /api/v1/locks
POST   /api/v1/locks
DELETE /api/v1/locks/{lockId}
GET    /api/v1/tokens       admin only
POST   /api/v1/tokens       admin only
DELETE /api/v1/tokens/{id}  admin only
GET    /api/v1/audit
```

More detail:

```txt
docs/API.md
docs/openapi.yaml
```

## 7. MCP wrapper

The MCP wrapper lets MCP clients/agents operate Bedrock using the same bearer token.

Run:

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN='<scoped-token>'
npm run mcp
```

Current MCP tools:

```txt
bedrock_info
bedrock_list
bedrock_read
bedrock_write
bedrock_mkdir
bedrock_search
bedrock_semantic_search
bedrock_graph_neighborhood
bedrock_toolbox_manifest
bedrock_triage_daily
bedrock_lock
bedrock_unlock
bedrock_trace
bedrock_snapshot
```

Current MCP resources:

```txt
ocean-bedrock://docs
ocean-bedrock://context
ocean-bedrock://coworkers
ocean-bedrock://sessions
ocean-bedrock://handoffs
```

## 8. Coworker bootstrap and local ingest

Coworkers must explicitly choose local folders to feed into the knowledge layer.

Local GUI app:

```bash
npm run ocean:app
```

This starts a localhost companion UI where coworkers can paste a scoped token, choose folders, run sync manually, and choose an interval schedule while the app is open. Details: `docs/LOCAL-GUI-APP.md`.

Bootstrap CLI:

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN='<scoped-token>'
npm run ocean:bootstrap
```

Ingest selected local folders:

```bash
npm run ocean:ingest -- --dry-run
npm run ocean:ingest
```

What bootstrap writes:

```txt
~/.config/ocean-bedrock/config.json              local config
/context/ocean-bedrock/sources/<source>.json     shared source manifest
```

What ingest writes:

```txt
/coworkers/<name>/<device-or-run>/<folder-label>/...      uploaded files
/context/ocean-bedrock/manifests/<run>.json               ingest run manifest
Ocean Ledger events                                       ingest history
```

Default safety behavior:

- opt-in folder selection,
- extension allowlist,
- ignores `.git`, caches, dependency folders, and common noise,
- secrets are not intentionally ingested,
- raw tokens are redacted from shared manifests.

## 9. Current ingestion engine

The current ingestion engine is live, but it is **V1**.

When a file is written through HTTP/MCP/local ingest:

```txt
file write
  -> record object metadata in Postgres longhouse.objects
  -> enqueue index_object job
  -> background worker reads file from volume
  -> textual files are chunked into longhouse.embedding_chunks
  -> enqueue embed_object + extract_graph jobs
  -> embed_object calls Workers AI and upserts to Vectorize
  -> extract_graph creates file/directory/heading/topic/link/source lineage edges
  -> ledger/audit activity records the action
```

Historical verified index state from the original snapshot:

```txt
objects recorded:      19 files
indexed chunks:        17
graph file nodes:      17
ledger events:         89
ingest jobs done:      17
ingest jobs failed:    9 (historical rows later dispositioned; see docs/OCEAN-TRIAGE-OPS.md and docs/HANDOFF.md for current state)
source adapters:       8 seeded
source instances:      6
source streams:        6
source sync runs:      2 completed, 3 cancelled debug runs
source records:        2
```

Current semantic/graph model:

```txt
chunk staging model: text-chunk-v1
embedding provider: cloudflare-workers-ai when configured
embedding model:    @cf/baai/bge-base-en-v1.5
Vectorize index:    ocean-longhouse-context
graph extraction:   file, directory, heading, topic, link, source lineage
```

Meaning:

```txt
YES: file intake, metadata, jobs, worker, text chunking, Workers AI embeddings, Vectorize upserts, semantic search endpoint, lightweight graph extraction
LIMIT: entity extraction is heuristic; Vectorize queries are async/eventually consistent
```


## 10. Ocean Context, semantic search, graph, and triage

Ocean Context is the actionable layer over files, chunks, vectors, graph edges, source records, and ledger events. See `docs/OCEAN-CONTEXT.md`.

Current endpoints:

```txt
GET  /api/v1/semantic/search?q=...&path=/context
GET  /api/v1/graph/nodes?path=/context
GET  /api/v1/graph/neighborhood?path=/context/file.md&depth=2
POST /api/v1/ocean-context/triage/daily
GET  /api/v1/toolbox/manifest
```

Daily triage writes:

```txt
/context/ocean-bedrock/triage/YYYY-MM-DD.md
```

It also emits `ocean_context.triage.completed` into Ocean Ledger.

## 11. Source adapter registry

The precedent research is documented in:

```txt
docs/SOURCE-ADAPTER-PRECEDENTS.md
```

The recommended model is:

```txt
source_adapters      adapter type definitions: local_folder, github, slack, notion, linear
source_instances     configured source: Alice folder, GitHub repo, Slack workspace
source_streams       selected folder/channel/repo/page/database inside a source
source_sync_runs     execution history, counters, errors, manifest path
source_records       source object catalog for lineage/dedupe/delete detection
```

Lineage target:

```txt
source_instance -> source_stream -> source_record -> object -> chunks -> graph -> ledger
```

Current source adapter registry status:

```txt
schema:          live in Postgres via db/003_source_adapters.sql
seeded adapters: local_folder, github, telegram, slack, notion, linear, google_drive, r2
helper module:   src/sources.mjs
HTTP API:        /api/v1/sources/* and /api/v1/sync/*
local bootstrap: writes/redacts source manifests and can directly seed registry when run by operator
local ingest:    uses server-side source/sync endpoints to create sync_run/source_record rows
smoke status:    local smoke + db:check pass; live source record check should run after deploy
```

Current sources actually implemented:

```txt
local_folder via ocean-bootstrap + ocean-ingest-local + server-side source registry lineage
direct HTTP/MCP writes
```

Planned adapters:

```txt
GitHub
Telegram
Slack
Notion
Linear
Google Drive
R2
```

## 12. Postgres tables currently used

Known live tables include:

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
longhouse.source_adapters
longhouse.source_instances
longhouse.source_streams
longhouse.source_sync_runs
longhouse.source_records
```

The exact migrations currently in repo:

```txt
db/001_longhouse_core.sql
db/002_ocean_ledger.sql
db/003_source_adapters.sql
```

## 12. Environment variables

Production-important variables:

```txt
OCEAN_BEDROCK_ROOT=/data
OCEAN_BEDROCK_INSTANCE=ocean-bedrock
OCEAN_LEDGER_STORE=postgres
OCEAN_BEDROCK_WORKER_ENABLED=true
DATABASE_URL=<Railway Postgres URL>
OCEAN_BEDROCK_BOOTSTRAP_TOKEN=<admin bootstrap token>
```

Other variables:

```txt
PORT / OCEAN_BEDROCK_PORT
HOST / OCEAN_BEDROCK_HOST
OCEAN_BEDROCK_AUTH_FILE
OCEAN_LEDGER_FILE
OCEAN_BEDROCK_MAX_UPLOAD
OCEAN_BEDROCK_CORS_ORIGIN
OCEAN_BEDROCK_INDEX_MAX_BYTES
OCEAN_BEDROCK_CHUNK_CHARS
OCEAN_BEDROCK_CHUNK_OVERLAP
```

## 13. Common operator commands

Smoke test locally:

```bash
npm run smoke
```

Check database schema:

```bash
npm run db:check
```

Apply migrations intentionally:

```bash
npm run db:migrate -- --yes
```

Create token locally:

```bash
npm run token:create -- --name alice --role contributor \
  --scope /coworkers/alice \
  --scope /sessions/alice \
  --scope /context/ocean-bedrock
```

Create token over API, admin only:

```bash
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/tokens" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"alice","role":"contributor","scopes":["/coworkers/alice","/sessions/alice","/context/ocean-bedrock"]}'
```

## 14. Security posture

Current posture is internal-use, progressive-autonomy.

Rules:

- do not commit secrets,
- do not put raw tokens in docs,
- use contributor tokens for normal coworkers,
- reserve admin tokens for operators,
- scope tokens to narrow paths,
- avoid delete permission by default,
- keep `/vault` as a placeholder until stronger secret handling exists,
- treat Vectorize/KV/cache as rebuildable indexes,
- treat Postgres + source bytes/manifests as authoritative.

## 15. Roadmap

Immediate next work:

1. Add Cloudflare Workers AI embeddings.
2. Upsert chunks into `ocean-longhouse-context` Vectorize.
3. Add semantic search endpoint and MCP tool.
4. Add R2 adapter for canonical object bytes.
5. Add GitHub and Telegram adapters first using the source registry tables.
6. Add Notion/Linear/Slack after secret handling and adapter-specific source policies are stable.
7. Add final coworker security review and install guide.
