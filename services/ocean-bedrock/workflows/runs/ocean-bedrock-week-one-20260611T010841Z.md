# Ocean Bedrock Week-One Workflow Run

Run started: 2026-06-11T01:08:41Z  
Last updated: 2026-06-11T01:28:00Z  
Workflow: `workflows/ocean-bedrock-week-one.workflow.json`

## Summary

Ocean Bedrock is now deployed as a dedicated Railway service and reachable over HTTPS.

Public URL:

```txt
https://ocean-bedrock-production.up.railway.app
```

Current status: **week-one functional baseline is live**.

## Phase status

| Phase | Status | Evidence |
| --- | --- | --- |
| Phase 0 — Source-of-truth guardrail | Passed | Canonical Longhouse docs were read from `ocean-os/docs/LONGHOUSE.md` and `LONGHOUSE_ORCHESTRATION.md`; Bedrock docs do not redefine Longhouse. |
| Phase 1 — Deploy Bedrock | Passed | Created/configured Railway service `ocean-bedrock`; added `/data` volume; deployed successfully; generated Railway domain; `/health` returns 200; authenticated `/api/v1/info` shows `instance=ocean-bedrock`, ledger store `postgres`. |
| Phase 2 — Auth/token ops | Passed | Issued scoped token `operator-contributor-mcp` with role `contributor`; verified token can write but cannot delete; verified revocation path with temporary token. |
| Phase 3 — MCP wrapper | Passed | Added `scripts/ocean-bedrock-mcp.mjs`, `npm run mcp`, and `docs/MCP.md`; public MCP smoke returned initialize/tools/list/bedrock_list/bedrock_snapshot successfully. |
| Phase 4 — Coworker bootstrap | Passed | `npm run ocean:bootstrap` and `npm run ocean:ingest` succeeded against public Bedrock with scoped contributor token; files landed under `/coworkers/operator-contributor/...`; manifests landed under `/context/ocean-bedrock/...`. |
| Phase 5 — Ingestion pipeline | Partial pass | Added metadata recording and server-side worker; file writes enqueue `index_object`; background worker chunk-indexed uploaded markdown into `longhouse.embedding_chunks`; Vectorize embedding/upsert remains next step. |
| Phase 6 — Security/launch review | Initial pass | `/health` public; `/api/v1/info` requires bearer token; contributor role has read/write/lock but no delete; shared manifests redact token. Remaining review needed before broad coworker rollout. |

## Deployed service

| Item | Value |
| --- | --- |
| Project | `rising-tides-agents` |
| Service | `ocean-bedrock` |
| Service ID | `b117c870-d503-4cb3-8653-c094b50b6fbb` |
| Public URL | `https://ocean-bedrock-production.up.railway.app` |
| Volume | `ocean-bedrock-volume` mounted at `/data` |
| Ledger store | `postgres` |
| Background worker | `OCEAN_BEDROCK_WORKER_ENABLED=true` |

## Verification evidence

### Health

```json
{
  "ok": true,
  "instance": "ocean-bedrock",
  "version": "0.1.0"
}
```

### Authenticated info

```json
{
  "instance": "ocean-bedrock",
  "version": "0.1.0",
  "ledger": { "store": "postgres" },
  "principal": {
    "name": "bootstrap-admin",
    "role": "admin",
    "scopes": ["/"]
  }
}
```

### Postgres readiness

`npm run db:check` reports:

```json
{
  "schemaReady": true,
  "tables": {
    "mounts": "longhouse.mounts",
    "ledgerEvents": "longhouse.ledger_events",
    "contextSnapshots": "longhouse.context_snapshots"
  }
}
```

### Scoped contributor token behavior

Issued token record:

```json
{
  "name": "operator-contributor-mcp",
  "role": "contributor",
  "scopes": [
    "/coworkers/operator-contributor",
    "/sessions/operator-contributor",
    "/context/ocean-bedrock"
  ]
}
```

Verified write works and delete fails:

```txt
delete_status=403
Token role "contributor" does not allow delete.
```

### Coworker bootstrap / ingest public smoke

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

### Chunk indexing

Uploaded markdown:

```txt
/coworkers/operator-contributor/workflow-smoke/contrib-notes/README.md
```

Worker inserted chunk record:

```txt
chunks=1
```

Additional indexed smoke file:

```txt
/coworkers/operator-test/workflow-smoke/local-notes/indexed-smoke.md
```

Postgres object metadata includes:

```json
{
  "chunks": 1,
  "source": "ocean-bedrock-http",
  "indexModel": "text-chunk-v1"
}
```

### Public MCP smoke

MCP wrapper pointed at public URL with scoped token returned:

```txt
initialize OK
tools/list OK, 10 tools
bedrock_list OK
bedrock_snapshot OK
```

## What shipped during this run

- Dedicated Railway `ocean-bedrock` service.
- Persistent Railway volume mounted at `/data`.
- Public Railway domain.
- Service variables:
  - `OCEAN_BEDROCK_ROOT=/data`
  - `OCEAN_BEDROCK_INSTANCE=ocean-bedrock`
  - `OCEAN_LEDGER_STORE=postgres`
  - `OCEAN_BEDROCK_WORKER_ENABLED=true`
  - `DATABASE_URL`
  - `OCEAN_BEDROCK_BOOTSTRAP_TOKEN`
- MCP wrapper: `scripts/ocean-bedrock-mcp.mjs`.
- Contributor role: read/write/lock, no delete.
- Metadata recording for file writes.
- Ingest job enqueue on file writes.
- Server-side background ingest worker.
- Text chunk indexing into Postgres.

## Remaining work

1. Add Cloudflare Workers AI embeddings.
2. Upsert embedding vectors into `ocean-longhouse-context` Vectorize index.
3. Add R2 adapter for canonical object bytes.
4. Add production MCP install instructions for each coworker/agent client.
5. Run final security review before issuing real coworker tokens.
6. Consider moving token registry from volume JSON to Postgres for multi-replica readiness.
