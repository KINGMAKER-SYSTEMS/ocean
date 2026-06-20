# Ocean Bedrock Handoff

Generated: 2026-06-17  
Repo: `/root/rt-nas/rt-nas`  
Service: `ocean-bedrock`  
Public URL: <https://ocean-bedrock-production.up.railway.app>

## 1. What is live

```txt
health endpoint:       live
Postgres ledger:       live
scoped bearer auth:    live
file API:              live — routed through StorageAdapter
local mount:           live — managed-local → /data/files
R2 mounts:             live — 6 buckets, read/write/delete/move/copy
mount registry API:    live — GET /api/v1/mounts (admin-gated)
MCP wrapper:           live — 10 tools
coworker bootstrap:    live
ingest worker:         live — text chunk → embed → graph pipeline
semantic vectors:      live — Workers AI bge-base-en-v1.5 + Cloudflare Vectorize
semantic search:       live — GET /api/v1/semantic/search
graph extraction:      live — nodes: 108, edges: 106
scheduled triage:      live — ocean-bedrock-triage-cron, 06:00 + 18:00 UTC
Telegram alerts:       live — medium/high triage findings
source adapter tables: live — 8 adapters seeded
server-side runners:   live — local_folder runner wired; other adapters stubbed
```

Ocean Bedrock is **not** the full Ocean Longhouse quorum/runtime. It is the cloud-box support substrate: storage, ingest, semantic/graph context, ledger traces, triage, and operator tooling.

## 2. Critical guardrails

1. Do not redefine Ocean Longhouse in Bedrock docs.
2. Canonical split:
   - `ocean-daemon` = local runtime/body/execution authority.
   - `ocean-longhouse` = hive brain/coordination layer.
   - `ocean-bedrock` = cloud-box support substrate.
3. Do not hallucinate Ocean docs. Use canonical docs:
   - `ocean-os/docs/LONGHOUSE.md`
   - `ocean-os/docs/LONGHOUSE_ORCHESTRATION.md`
   - relevant `ocean-surface` docs for Surface-specific claims.
4. Do not commit or paste raw secrets into docs.
5. Coworker ingest must remain opt-in folder selection.
6. Coworker/agent tokens should be scoped bearer/MCP tokens, not admin tokens.
7. Avoid ingesting secrets by default.
8. `DATABASE_URL` is Railway-internal (`postgres.railway.internal:5432`) — does not resolve off-Railway.
9. Token files at `/root/.config/ocean-bedrock/` are gitignored; never commit or log them.
10. Default to the contributor token for testing, not the admin token.

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

Deployment history (key IDs):

```txt
4116f466   initial successful service deploy
d04ef61f   metadata/worker deploy
5d31071a   contributor role deploy
54a961eb   landing route deploy
202428df   source/sync API deploy
db04d2f4   Ocean Context semantic/graph/toolbox deploy
cf1c3e9b   Wave 1 triage scheduler + failed-job tooling
bc7f1ca2   Wave 2 data-plane mount router
af2a2949   Telegram triage env enable
d9984f8a   mount containment fix + triage alert docs
           (commit 738818b — latest before R2 adapter)
367b346    R2 object adapter
c0ec37fe   Epoch 01 P1 coworker onboarding deploy — current Railway deployment
```

Production env vars:

```txt
OCEAN_BEDROCK_ROOT=/data
OCEAN_BEDROCK_INSTANCE=ocean-bedrock
OCEAN_LEDGER_STORE=postgres
OCEAN_BEDROCK_WORKER_ENABLED=true
DATABASE_URL=<Railway Postgres URL — internal only>
OCEAN_BEDROCK_BOOTSTRAP_TOKEN=<secret>
CLOUDFLARE_ACCOUNT_ID=<set>
CLOUDFLARE_API_TOKEN=<secret — has R2 object read/write/delete>
OCEAN_VECTORIZE_INDEX=ocean-longhouse-context
OCEAN_EMBEDDING_MODEL=@cf/baai/bge-base-en-v1.5
OCEAN_EMBEDDING_DIMENSIONS=768
OCEAN_EMBEDDING_POOLING=cls
OCEAN_TRIAGE_TELEGRAM_BOT_TOKEN=<secret>
OCEAN_TRIAGE_TELEGRAM_CHAT_ID=<set>
```

## 4. Token locations

```txt
/root/.config/ocean-bedrock/railway-admin-token.txt      — admin, all paths
/root/.config/ocean-bedrock/operator-test-token.txt      — test role
/root/.config/ocean-bedrock/operator-contributor-token.txt — use this for testing
```

Contributor token scopes:

```txt
paths:   /coworkers/operator-contributor, /sessions/operator-contributor, /context/ocean-bedrock
can:     read, write, lock
cannot:  delete, admin token ops
```

## 5. Storage mounts

Live mounts (from `GET /api/v1/mounts`):

```txt
type    id                           prefix
local   managed-local                /               → /data/files
r2      r2-content-posting-lab       /assets/content-posting-lab
r2      r2-pi-video-gallery          /assets/video-gallery
r2      r2-pi-backups                /archive/pi-backups
r2      r2-sales-pitch-pdfs          /sales/pitch-pdfs
r2      r2-financial-invoices        /finance/invoices
r2      r2-rt-html-previews          /previews/html
```

R2 adapter (`src/mounts/adapters/r2.mjs`) uses Cloudflare REST API with the existing `CLOUDFLARE_API_TOKEN` bearer auth — no S3 credentials, no new npm packages.

Implementation notes:
- `stat()` — list endpoint with `prefix=<key>&limit=2`; CF REST does not support HEAD
- `list()` — `delimiter=/` for depth=1 (immediate children); cursor pagination up to 20 pages
- `read()` — returns `{stream: res.body}` (Web ReadableStream); server pipes via `Readable.fromWeb()`
- `write()` — streams upload via `Readable.toWeb(createReadStream(tmpPath))` with `duplex: 'half'`
- `mkdir()` — no-op (R2 has no real directories)
- `delete({recursive})` — walks cursor pages; Promise.all concurrent (revisit if bulk-deleting large prefixes)
- `diskRoot = null` → text search returns 501

## 6. Database state

Schema tables (all present in `longhouse` schema):

```txt
mounts, ledger_events, context_snapshots
source_adapters, source_instances, source_streams, source_sync_runs, source_records
embedding_chunks, graph_nodes, graph_edges, ingest_jobs
```

Current counts (as of 2026-06-17):

```txt
embedding chunks:   90 (90 vectorized, 0 pending)
graph nodes:        108
graph edges:        106
ingest jobs done:   76
ingest jobs cancelled: 9 (stale smoke runs, already dispositioned)
source adapters:    8 seeded
```

## 7. Ingest pipeline

Path: file write → `ingest_jobs` queue → worker loop → chunk → embed → graph extract

Worker runs in the same process as the HTTP server (`OCEAN_BEDROCK_WORKER_ENABLED=true`). Jobs are picked up in order: `chunk_object` → `embed_object` → `extract_graph`. All three are required for a path to appear in semantic/graph search.

Semantic search: `GET /api/v1/semantic/search?q=<query>&limit=<n>&threshold=<0-1>`

Graph neighborhood: `GET /api/v1/graph/neighborhood?path=<path>&depth=<n>`

## 8. Scheduled triage

Service: `ocean-bedrock-triage-cron` (separate Railway service, same project)  
Schedule: `0 6,18 * * *` UTC (06:00 and 18:00)  
Script: `npm run ocean:triage`  
Idempotency key: `triage:<YYYY-MM-DD>:<am|pm>`

Findings stored at: `/context/ocean-bedrock/triage/_alerts/<date>-<slot>.md`  
Thresholds: `OCEAN_TRIAGE_FAILED_ALERT` (default 1) / `OCEAN_TRIAGE_QUEUE_ALERT` (default 50)  
Telegram alerts: active for medium/high severity; webhook remains off.

Failed job tooling: `npm run ocean:jobs`  
Full runbook: `docs/OCEAN-TRIAGE-OPS.md`

## 9. MCP wrapper

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"
npm run mcp
```

Tools (10):

```txt
bedrock_info, bedrock_list, bedrock_read, bedrock_write, bedrock_mkdir,
bedrock_search, bedrock_lock, bedrock_unlock, bedrock_trace, bedrock_snapshot
```

## 10. Source adapter registry

8 adapters seeded. Server-side runner dispatch is live (`src/sources/runners/`):
- `local_folder` — fully implemented
- `github`, `telegram`, `slack`, `notion`, `linear`, `google_drive`, `r2` — stubbed (return 501 / not-implemented)

Source API:

```txt
GET  /api/v1/sources/adapters
GET  /api/v1/sources/instances
GET  /api/v1/sources/instances/:id
GET  /api/v1/sync/runs
GET  /api/v1/sync/runs/:id
GET  /api/v1/sync/records/:runId
POST /api/v1/sync/run (triggers a local_folder sync run)
```

## 11. Verification commands

```bash
export BASE='https://ocean-bedrock-production.up.railway.app'
export TOKEN="$(cat /root/.config/ocean-bedrock/operator-contributor-token.txt)"
export ADMIN="$(cat /root/.config/ocean-bedrock/railway-admin-token.txt)"

# Health
curl "$BASE/health"

# Mounts (admin)
curl -H "x-ocean-bedrock-token: $ADMIN" "$BASE/api/v1/mounts"

# Stat an R2 path
curl -H "x-ocean-bedrock-token: $ADMIN" \
  "$BASE/api/v1/stat?path=/assets/content-posting-lab"

# List an R2 directory
curl -H "x-ocean-bedrock-token: $ADMIN" \
  "$BASE/api/v1/list?path=/assets/content-posting-lab"

# Write a file (contributor)
echo "test" | curl -X PUT \
  -H "x-ocean-bedrock-token: $TOKEN" --data-binary @- \
  "$BASE/api/v1/file?path=/coworkers/operator-contributor/test.md"

# Semantic search
curl -H "x-ocean-bedrock-token: $TOKEN" \
  "$BASE/api/v1/semantic/search?q=triage&limit=5"

# DB check (local, requires Railway-internal DATABASE_URL)
node scripts/check-postgres.mjs
```

## 12. Deploy runbook

```bash
# From repo root
railway up --service ocean-bedrock

# Verify
curl -fsS https://ocean-bedrock-production.up.railway.app/health
```

Smoke test (local, no Railway required):

```bash
npm run smoke
```

## 13. Known issues

1. Recursive R2 delete uses concurrent `Promise.all` with no per-request status check — fine for current use, revisit for bulk-delete on large prefixes.
2. Token registry is volume JSON (`/data/.ocean-bedrock/tokens.json`), not Postgres — fine for one Railway replica, not multi-replica safe.
3. `/vault` is a folder placeholder, not an encrypted secrets vault.
4. `/api/v1/search` is plain file text search; semantic search lives at `/api/v1/semantic/search`. R2 paths return 501 for text search (`diskRoot = null`).
5. Graph extraction is heuristic (file/dir/heading/topic/link/source-lineage nodes); rich business entities (people, projects, clients) not yet extracted.
6. Source adapter runners for github/telegram/slack/notion/linear/google_drive/r2 are stubbed at 501.
7. Coworker real rollout prep is partially complete: one-time invite exchange, shared secret exclusions, dry-run skipped/sensitive reporting, and local GUI preview are live. Remaining: packaged GUI, hard acknowledgement gate before first sync, Postgres-backed token/invite registry, and broader rollout docs polish.

## 14. Next work queue

### Priority 1 — Real coworker rollout prep

Before issuing tokens to real coworkers:

- Confirm path scopes and document install instructions.
- Package the local GUI/download flow (`npm run ocean:app`).
- Use the one-time invite exchange instead of raw token paste.
- Run a real folder dry-run; inspect skipped files and verify secrets are ignored.
- Issue contributor-only tokens, confirm delete is blocked.

### Priority 2 — Source adapter runners

Wire real sync runners for the other 7 adapter types (github, telegram, slack, notion, linear, google_drive, r2). The runner dispatch layer is already in place in `src/sources/runners/`.

### Priority 3 — Richer graph/entity extraction

Add people/project/client/task entity extraction. Wire graph-backed context packs for agents. Fuse graph + semantic results in search.

### Priority 4 — Vault

Replace `/vault` folder placeholder with an actual encrypted secrets vault (age/sops or a KV-backed store).

## 15. Acceptance criteria for next handoff

Include in the next handoff:

- commit hash and Railway deployment ID,
- current `curl /health` output,
- `node scripts/check-postgres.mjs` output,
- exact migration status,
- any new issued coworker tokens by name/scope (not raw values),
- whether source adapter runners beyond local_folder are wired.
