# Ocean Context

Ocean Context is the actionable knowledge layer built on top of Ocean Bedrock.

It is not just document storage. It is the joined model of:

- source records and sync runs,
- Longhouse files and object metadata,
- text chunks,
- Cloudflare Workers AI embeddings,
- Cloudflare Vectorize semantic index entries,
- graph nodes and edges,
- Ocean Ledger events and context snapshots,
- daily triage reports.

## Ledger vs Context

**Ocean Ledger** is the append-only historical timeline. It answers:

- what happened,
- who/what acted,
- what context was visible,
- what correlation/thread this belongs to,
- whether history is tamper-evident.

**Ocean Context** is the retrieval and decision-support layer. It answers:

- what information is relevant right now,
- which files/chunks/entities are connected,
- what changed recently,
- what needs triage,
- which tools/agents can act on the context.

## Semantic pipeline

Current V1 pipeline:

```txt
PUT /api/v1/file
  -> longhouse.objects row
  -> index_object job
  -> longhouse.embedding_chunks rows
  -> embed_object job
  -> Workers AI @cf/baai/bge-base-en-v1.5
  -> Vectorize index ocean-longhouse-context
  -> /api/v1/semantic/search
```

Defaults:

```txt
OCEAN_EMBEDDING_MODEL=@cf/baai/bge-base-en-v1.5
OCEAN_EMBEDDING_DIMENSIONS=768
OCEAN_EMBEDDING_POOLING=cls
OCEAN_VECTORIZE_INDEX=ocean-longhouse-context
```

If Cloudflare credentials are unavailable, semantic search returns a lexical chunk fallback.

## Graph pipeline

The `extract_graph` job creates lightweight graph structure from indexed files:

- `file` nodes,
- `directory` nodes,
- markdown `heading` nodes,
- hashtag `topic` nodes,
- markdown links as `links_to` edges,
- source record/source instance lineage edges.

Current graph endpoints:

```txt
GET /api/v1/graph/nodes?path=/context&q=ocean
GET /api/v1/graph/neighborhood?path=/context/ocean-bedrock/example.md&depth=2
```

Backfill existing chunks/graph rows:

```bash
npm run ocean:semantic:backfill -- --limit 50
```

## Daily triage

Daily triage is the operating ritual for the agentic knowledge base.

Endpoint:

```txt
POST /api/v1/ocean-context/triage/daily
```

Script:

```bash
npm run ocean:triage -- --token-file ~/.config/ocean-bedrock/operator-contributor-token.txt
```

It writes a report to:

```txt
/context/ocean-bedrock/triage/YYYY-MM-DD.md
```

It checks:

- ledger hash-chain verification,
- failed/queued ingest jobs,
- source sync failures,
- pending unembedded chunks,
- semantic/Vectorize environment status,
- graph population counts,
- recent ledger events.

It also appends an `ocean_context.triage.completed` ledger event.

## Agent operating loop

Recommended daily loop:

1. Run Ocean Context triage.
2. Fix failed ingest/source jobs.
3. Re-index or re-embed stale chunks.
4. Review new ledger decisions and context snapshots.
5. Promote important findings into durable docs.
6. Queue agent tasks with scoped context snapshots.

## Current limits

- Entity extraction is heuristic only.
- Graph edges are lightweight and local-file centric.
- Vectorize is the serving index; Postgres stores metadata but not raw vectors.
- Daily triage runs automatically via the `ocean-bedrock-triage-cron` Railway service (schedule `0 6,18 * * *` UTC). See [docs/OCEAN-TRIAGE-OPS.md](OCEAN-TRIAGE-OPS.md) for the operator runbook.
