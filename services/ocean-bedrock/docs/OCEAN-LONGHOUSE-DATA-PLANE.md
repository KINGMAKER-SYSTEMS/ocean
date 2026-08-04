# Ocean Longhouse Data Plane

This is the Cloudflare-first data architecture for Ocean Longhouse.

## Chosen foundation

| Layer | V1 choice | Why |
| --- | --- | --- |
| Object bytes | Cloudflare R2 + local managed storage | Cheap object storage, no egress surprises, easy bucket-per-domain organization |
| Metadata/control plane | Railway Postgres | Already connected; relational source of truth for mounts, files, Ocean Ledger, graph, jobs, audit |
| Vector search | Cloudflare Vectorize primary; optional pgvector fallback | Cloudflare-first serving path; Postgres can remain the canonical chunk catalog |
| Embeddings | Cloudflare Workers AI first; pluggable OpenAI/Replicate later | Keeps inference near Cloudflare storage and avoids handing provider keys to agents |
| Graph | Postgres node/edge tables first | Good enough for entity/project/session graphs without adding Neo4j too early |
| KV cache | Cloudflare KV for edge/global hot cache; Postgres `kv_cache` for durable fallback; in-memory LRU in daemon | Fast path at edge, canonical invalidation in Postgres |
| Locks/concurrency | Longhouse DB now; Cloudflare Durable Objects later for edge locking | Reliable V1 on daemon, edge-native later |
| Background jobs | Railway daemon loop now; Cloudflare Queues later | Daemon is easy because Railway/Postgres already exist |

## Existing Cloudflare buckets

Current account buckets discovered:

```txt
content-posting-lab
financial-dashboard-invoices
pi-backups
pi-video-gallery
rt-html-previews
rt-sales-pitch-pdfs
```

Suggested virtual mounts:

```txt
/assets/content-posting-lab  -> R2 content-posting-lab
/previews/html               -> R2 rt-html-previews
/assets/video-gallery        -> R2 pi-video-gallery
/sales/pitch-pdfs            -> R2 rt-sales-pitch-pdfs
/finance/invoices            -> R2 financial-dashboard-invoices, readonly by default
/archive/pi-backups          -> R2 pi-backups, readonly by default
```

Dedicated core bucket provisioned:

```txt
ocean-longhouse
```

Use it for canonical Longhouse-managed object storage instead of mixing core state into app-specific buckets.

## Data ownership

Postgres is the authority for:

- mount registry
- virtual path metadata
- object manifests and replicas
- Ocean Ledger events/correlations/context snapshots
- session registry
- graph nodes and edges
- embedding chunk catalog
- ingest job queue
- audit events
- durable cache fallback

R2 is the authority for:

- file/object bytes for R2-mounted paths
- large artifacts
- binary assets
- snapshots/archives

Vectorize is the authority for:

- live vector nearest-neighbor index

But Postgres remains the authority for:

- which chunks exist
- which model produced each embedding
- which vectorize index/id owns the vector
- how to rebuild the vector index

## Ocean Ledger ingest pipeline

```txt
external source event / agent action / file operation
        |
        v
append Ocean Ledger event
        |
        +--> correlate by id/path/source/actor/time
        +--> create context snapshot when needed
        +--> update graph edges
        +--> enqueue embedding/index work for textual payloads
```

## File ingest pipeline

```txt
write file / sync external object
        |
        v
record object metadata in Postgres
        |
        v
enqueue ingest_jobs.index_object
        |
        v
extract text/metadata
        |
        +--> update graph nodes/edges
        |
        +--> chunk text
                |
                v
            embed chunks
                |
                +--> upsert to Cloudflare Vectorize
                +--> store chunk catalog in Postgres
                +--> optionally store vector in pgvector
```

## Retrieval pipeline

For an agent query:

1. Auth token determines scope.
2. Check KV/in-memory cache for recent query or path metadata.
3. Use Vectorize for semantic nearest-neighbor search.
4. Use Postgres graph traversal for related entities/sessions/docs.
5. Read source files from R2/local/GitHub mount.
6. Return answer context with citations to Longhouse virtual paths.

## Graph model

Use Postgres first:

```txt
node types:
- file
- folder
- session
- agent
- project
- customer
- document
- concept
- credential_handle
- task

edge predicates:
- contains
- authored_by
- mentions
- depends_on
- derived_from
- handoff_to
- uses_secret_handle
- belongs_to_project
- supersedes
- mirrors
```

This gives us a useful knowledge graph without running a separate graph database.

## KV cache classes

| Namespace | Examples | TTL |
| --- | --- | --- |
| `path_stat` | stat/list metadata | 30s-5m |
| `search_result` | repeated semantic/text search responses | 1m-15m |
| `token_scope` | token id -> scopes/role | 30s-5m |
| `mount_route` | virtual path prefix -> mount | 5m-1h |
| `signed_url` | generated R2 signed URL metadata | until URL expiry |
| `session_snapshot` | active session summaries | 30s-5m |

Cache invalidation rule: writes/deletes/moves emit audit events and invalidate path-prefix cache keys.

## Cloudflare resources

Provisioned resources:

1. R2 bucket: `ocean-longhouse`
2. Vectorize index: `ocean-longhouse-context`
   - embedding preset: `@cf/baai/bge-base-en-v1.5`
   - dimensions: 768
   - metric: cosine
3. KV namespace: `OCEAN_LONGHOUSE_CACHE`
   - id: `2e7a00f20a3848a2be22a35a16b321ea`

Still optional/to add later:
4. Optional Queues:
   - `ocean-longhouse-ingest`
   - `ocean-longhouse-replicate`
5. Optional Worker front door:
   - auth, read cache, signed URLs, MCP endpoint
6. Railway daemon:
   - connected to Postgres
   - R2 credentials
   - embedding/indexing workers

## Environment variables

Daemon/Railway:

```txt
DATABASE_URL=postgres://...
OCEAN_BEDROCK_ROOT=/data or Railway volume path
LONGHOUSE_PUBLIC_URL=https://...
LONGHOUSE_PRIMARY_BUCKET=ocean-longhouse
CLOUDFLARE_ACCOUNT_ID=...
CLOUDFLARE_API_TOKEN=...
R2_ENDPOINT=https://<accountid>.r2.cloudflarestorage.com
R2_ACCESS_KEY_ID=...
R2_SECRET_ACCESS_KEY=...
LONGHOUSE_VECTORIZE_INDEX=ocean-longhouse-context
LONGHOUSE_EMBEDDING_PROVIDER=cloudflare
LONGHOUSE_EMBEDDING_MODEL=@cf/baai/bge-base-en-v1.5
```

Worker/edge later:

```txt
KV binding: OCEAN_LONGHOUSE_CACHE
R2 binding: OCEAN_LONGHOUSE
Vectorize binding: OCEAN_CONTEXT
Hyperdrive binding: LONGHOUSE_POSTGRES
```

## Database migration

Initial schema drafts:

```txt
db/001_longhouse_core.sql
db/002_ocean_ledger.sql
```

Do not apply blindly. First confirm which Railway Postgres instance should own Longhouse state.

## Decision: pgvector vs Vectorize

Recommended V1:

- Use **Cloudflare Vectorize** for live vector retrieval.
- Store chunk metadata in Postgres.
- Keep `pgvector` optional for local/dev/fallback.

Reason: Vectorize fits the Cloudflare-first direction and keeps vector search serving outside the Railway app. Postgres stays the rebuildable source of truth.

## Decision: graph database vs Postgres graph tables

Recommended V1:

- Use Postgres `graph_nodes` and `graph_edges` tables.

Reason: Ocean needs useful relationships immediately, not a separate graph infrastructure bill. If graph queries become complex, we can later mirror nodes/edges into Neo4j, Kuzu, or a graph-native service.

## Immediate implementation checklist

- [x] Confirm Railway runtime has a Postgres connection (`npm run db:check` connects; schema not applied yet).
- [ ] Apply Longhouse/Ocean Ledger schema to the target Postgres.
- [x] Create dedicated R2 bucket `ocean-longhouse`.
- [x] Create Cloudflare Vectorize index `ocean-longhouse-context`.
- [x] Create KV namespace `OCEAN_LONGHOUSE_CACHE`.
- [ ] Create R2 access keys and set Railway env vars.
- [ ] Apply `db/001_longhouse_core.sql`.
- [ ] Refactor file operations through mount registry.
- [ ] Add R2 adapter using S3-compatible API.
- [ ] Add ingest job loop.
- [ ] Add text extraction/chunking.
- [ ] Provision Vectorize index.
- [ ] Add Cloudflare embedding provider.
- [ ] Add MCP wrapper tools.
