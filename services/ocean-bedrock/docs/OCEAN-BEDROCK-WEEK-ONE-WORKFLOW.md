# Ocean Bedrock Week-One Workflow

Purpose: make **Ocean Bedrock** accessible as a functional, authenticated cloud-box tool for coworkers and agents.

Canonical source-of-truth boundary:

- `ocean-daemon` is the local runtime/body and execution authority.
- `ocean-longhouse` is the hive brain / coordination layer.
- `ocean-bedrock` is the cloud-box support substrate for shared files, context ingest, ledger/event store, Postgres/R2/Vectorize/KV, MCP-facing infrastructure, and coworker bootstrap.

Machine-readable workflow:

```txt
workflows/ocean-bedrock-week-one.workflow.json
```

## Model roster

Use the configured models by role:

| Role | Model | Use |
| --- | --- | --- |
| Strategic orchestrator | `chatgpt-5.5` | launch arbitration, high-impact review, final go/no-go |
| Architecture reviewer | `chatgpt-5.4` | canonical boundary/API/data model review |
| Implementation worker | `chatgpt-5.4-mini` | scripts, config, small code tasks |
| Docs/onboarding | `chatgpt-5.3-spark` | coworker docs, checklists, runbooks |
| Security reviewer | `deepseek-v4-pro` | token scope, auth, secret-handling, destructive gates |
| Test/log triage | `deepseek-v4-flash` | smoke tests, logs, fast failure analysis |
| Integration engineer | `kimi-k2.6` | Railway/Cloudflare/MCP integration |
| MCP interface designer | `minimax-m3` | MCP tool/resource contract |
| Ingestion/courier designer | `minimax-m2.7` | bootstrap UX and local folder courier flow |

## Phase 0 — Source-of-truth guardrail

Owner: `chatgpt-5.4`

- Read canonical `ocean-os/docs/LONGHOUSE.md` and `LONGHOUSE_ORCHESTRATION.md` before changing Longhouse semantics.
- Keep Bedrock as cloud-box support substrate.
- Do not claim Bedrock is the quorum engine.

Acceptance:

- Docs preserve `ocean-daemon` / `ocean-longhouse` / `ocean-bedrock` boundary.

## Phase 1 — Deploy Bedrock

Owner: `kimi-k2.6`  
Review: `deepseek-v4-pro`

Tasks:

1. Create/configure dedicated Railway service for `ocean-bedrock`.
2. Set env:
   - `OCEAN_LEDGER_STORE=postgres`
   - `DATABASE_URL`
   - `OCEAN_BEDROCK_ROOT`
   - `OCEAN_BEDROCK_BOOTSTRAP_TOKEN` only for first admin bootstrap if needed.
3. Verify migrations:

```bash
npm run db:migrate -- --yes
npm run db:check
```

4. Expose through HTTPS route.
5. Verify:

```bash
curl https://<bedrock-host>/health
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" https://<bedrock-host>/api/v1/info
```

Acceptance:

- Public HTTPS health works.
- Authenticated info shows `instance=ocean-bedrock` and `ledger.store=postgres`.

## Phase 2 — Auth/token ops

Owner: `deepseek-v4-pro`

Tasks:

- Define coworker path convention:

```txt
/coworkers/<name>/<device>
/sessions/<name>
/context/ocean-bedrock
```

- Issue scoped token per coworker/device:

```bash
npm run token:create -- \
  --name alice-mcp \
  --role contributor \
  --scope /coworkers/alice \
  --scope /sessions/alice \
  --scope /context/ocean-bedrock
```

Acceptance:

- Coworker token can bootstrap and ingest.
- Admin token is not used by coworkers.
- Revoke/rotate path is verified.

## Phase 3 — MCP wrapper

Owner: `minimax-m3`  
Review: `chatgpt-5.4`

Expose Bedrock through MCP using the same bearer token.

Resources:

```txt
ocean-bedrock://docs/...
ocean-bedrock://context/...
ocean-bedrock://coworkers/...
ocean-bedrock://sessions/...
ocean-bedrock://handoffs/...
```

Tools:

```txt
bedrock_list(path, depth?)
bedrock_read(path)
bedrock_write(path, content)
bedrock_search(query, path?)
bedrock_lock(path, ttlSeconds?, note?)
bedrock_unlock(lockId)
bedrock_trace(correlationId)
bedrock_snapshot(name, files?, events?, summary?)
```

Acceptance:

- Agent can list/read/write/search within scoped paths through MCP.
- Unauthorized path access fails.

## Phase 4 — Coworker bootstrap

Owner: `minimax-m2.7`

Coworker runs:

```bash
npm run ocean:bootstrap
npm run ocean:ingest -- --dry-run
npm run ocean:ingest
```

Acceptance:

- Coworker selects local folders without editing JSON manually.
- Upload writes files under `/coworkers/<name>/<device>/...`.
- Manifest lands under `/context/ocean-bedrock/manifests/`.
- Ledger event `ingest.local_folder_synced` is emitted.

## Phase 5 — Ingestion pipeline

Owner: `kimi-k2.6`  
Review: `chatgpt-5.4`

Tasks:

1. Add ingest worker loop over `longhouse.ingest_jobs`.
2. On file write/upload, enqueue jobs:
   - `index_object`
   - `embed_object`
   - `extract_graph`
3. Extract text for docs/code/markdown first.
4. Chunk text and store chunk metadata in Postgres.
5. Embed through Cloudflare Workers AI.
6. Upsert vectors into `ocean-longhouse-context`.
7. Add graph nodes/edges for file/source/coworker/project/session.

Acceptance:

- At least one uploaded markdown/text file is indexed.
- Vectorize upsert/query path is verified.
- Graph/chunk records exist in Postgres.

## Phase 6 — Security and launch review

Owner: `deepseek-v4-pro`  
Final review: `chatgpt-5.5`

Checklist:

- Token scopes are least-privilege.
- No raw token appears in shared manifests.
- Secret folders are excluded by default.
- Destructive tools are absent or admin-gated.
- Public endpoints are intentionally public.
- Revocation/rotation/runbook exists.

## Definition of done

- Ocean Bedrock is reachable over HTTPS.
- Postgres schema is ready and ledger store is `postgres`.
- Scoped coworker token works end-to-end.
- MCP wrapper exposes list/read/write/search/snapshot.
- Coworker bootstrap creates local config and source manifest.
- Ingest uploads changed files and emits ledger events.
- At least one uploaded file is indexed into the knowledge layer.
- Security review passes.
