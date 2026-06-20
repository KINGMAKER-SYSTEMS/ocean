# ocean-bedrock

Ocean Bedrock is the cloud-box substrate for Ocean Longhouse support: authenticated shared files, coworker folder ingestion, context manifests, ledger events, and knowledge-layer bootstrap.

This started from a “NAS” idea, but the canonical name for this service is now **ocean-bedrock**. It is a stateful shared directory with a declared API so agents and coworkers can exchange files, session context, documentation, handoff notes, and collaboration artifacts.

The canonical Ocean Bedrock agent naming and responsibility map lives in [`docs/OCEAN-BEDROCK-AGENT-ROSTER.md`](docs/OCEAN-BEDROCK-AGENT-ROSTER.md). Project-scoped crew boot notes live in [`docs/RT-NAS-CREW-BOOT.md`](docs/RT-NAS-CREW-BOOT.md).

## V1 model

- Durable shared filesystem rooted at `data/files/`
- Token auth via `Authorization: Bearer <token>`
- Roles: `readonly`, `contributor`, `agent`/`readwrite`, `admin`
- Path scopes, e.g. token can be limited to `/docs` or `/sessions/project-a`
- API for list/stat/read/write/delete/move/copy/search
- Lock leases so agents can coordinate edits
- Admin token issuing/revocation API
- Audit log for writes, deletes, locks, and token changes
- Server-side source registry and sync-run lineage APIs for local-folder ingest
- Ocean Context semantic search, graph endpoints, daily triage, and toolbox manifest staging

Default folders created on startup:

```txt
/docs       durable company docs
/context    shared reusable context and project memory
/sessions   session-specific artifacts and notes
/handoffs   cross-agent handoffs
/shared     general shared files
/vault      sensitive material placeholder; avoid raw secrets unless properly protected
```

## Quick start

```bash
# Create an admin token locally
npm run token:create -- --name admin --role admin

# Copy the printed token, then start the server
OCEAN_BEDROCK_TOKEN='<printed token>' npm start
```

The server reads issued tokens from `data/.ocean-bedrock/tokens.json`; `OCEAN_BEDROCK_TOKEN` is just a convenient shell variable for curl examples.

For development you can bootstrap a known admin token:

```bash
OCEAN_BEDROCK_BOOTSTRAP_TOKEN=dev-token-change-me npm start
```

## Curl examples

```bash
export OCEAN_BEDROCK_URL=http://localhost:8080
export OCEAN_BEDROCK_TOKEN=dev-token-change-me

# Landing/route index, public
curl "$OCEAN_BEDROCK_URL/"

# Info, authenticated
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" "$OCEAN_BEDROCK_URL/api/v1/info"

# Create a folder
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/mkdir" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"path":"/docs/projects"}'

# Upload/write a file
curl -X PUT "$OCEAN_BEDROCK_URL/api/v1/file?path=/docs/projects/brief.md" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  --data-binary @brief.md

# Read/download a file
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/file?path=/docs/projects/brief.md&inline=1"

# List files
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/tree?path=/docs&depth=3"

# Acquire a lock before editing
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/locks" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"path":"/docs/projects/brief.md","ttlSeconds":900,"note":"NiceTiger editing"}'
```

## Issue an agent token

Admin API:

```bash
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/tokens" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"research-agent","role":"contributor","scopes":["/docs","/context"],"ttlDays":30}'
```

Local CLI:

```bash
npm run token:create -- --name research-agent --role contributor --scope /docs --scope /context
```

The raw token is printed once. Store it in the agent session/environment.

## Agent convention

Agents should:

1. Read `/docs` and `/context` before starting collaborative work.
2. Write durable outputs to `/docs`, `/context`, or project-specific folders.
3. Create session folders under `/sessions/<agent-or-project>/<date>/` for scratch state.
4. Use `/handoffs` for explicit cross-agent handoff packets.
5. Acquire a lock before editing files another agent might edit.
6. Avoid placing raw high-value secrets in `/vault` until encryption or a secrets manager is added.

## API and architecture docs

- Human-readable API notes: [`docs/API.md`](docs/API.md)
- OpenAPI declaration: [`docs/openapi.yaml`](docs/openapi.yaml)
- Ocean Bedrock cloud-box note: [`docs/OCEAN-BEDROCK.md`](docs/OCEAN-BEDROCK.md)
- Week-one workflow: [`docs/OCEAN-BEDROCK-WEEK-ONE-WORKFLOW.md`](docs/OCEAN-BEDROCK-WEEK-ONE-WORKFLOW.md)
- Week-one runbook: [`docs/WEEK-ONE-RUNBOOK.md`](docs/WEEK-ONE-RUNBOOK.md)
- Coworker bootstrap: [`docs/COWORKER-BOOTSTRAP.md`](docs/COWORKER-BOOTSTRAP.md)
- Local GUI companion app: [`docs/LOCAL-GUI-APP.md`](docs/LOCAL-GUI-APP.md)
- Ocean Context: [`docs/OCEAN-CONTEXT.md`](docs/OCEAN-CONTEXT.md)
- Ocean Toolbox bootstrap: [`docs/OCEAN-TOOLBOX-BOOTSTRAP.md`](docs/OCEAN-TOOLBOX-BOOTSTRAP.md)
- Source adapter precedents: [`docs/SOURCE-ADAPTER-PRECEDENTS.md`](docs/SOURCE-ADAPTER-PRECEDENTS.md)
- MCP wrapper: [`docs/MCP.md`](docs/MCP.md)
- Ocean Longhouse architecture: [`docs/OCEAN-LONGHOUSE.md`](docs/OCEAN-LONGHOUSE.md)
- Ocean Ledger context history: [`docs/OCEAN-LEDGER.md`](docs/OCEAN-LEDGER.md)
- Runtime OpenAPI endpoint: `GET /api/v1/openapi.yaml`

## Configuration

| Variable | Default | Description |
| --- | --- | --- |
| `PORT` / `OCEAN_BEDROCK_PORT` | `8080` | HTTP port |
| `HOST` / `OCEAN_BEDROCK_HOST` | `0.0.0.0` | Bind host |
| `OCEAN_BEDROCK_ROOT` | `./data` | Persistent data root |
| `OCEAN_BEDROCK_AUTH_FILE` | `$OCEAN_BEDROCK_ROOT/.ocean-bedrock/tokens.json` | Token registry |
| `OCEAN_LEDGER_STORE` | `postgres` when `DATABASE_URL` is set, otherwise `jsonl` | Ocean Ledger store: `jsonl` or `postgres` |
| `OCEAN_LEDGER_FILE` | `$OCEAN_BEDROCK_ROOT/.ocean-bedrock/ocean-ledger.jsonl` | Local Ocean Ledger JSONL store |
| `DATABASE_URL` | unset | Required on the server for Postgres ledger, metadata, source registry, and sync-run lineage |
| `OCEAN_BEDROCK_MAX_UPLOAD` | `250mb` | Max upload/write size |
| `OCEAN_BEDROCK_BOOTSTRAP_TOKEN` | unset | Creates first admin token when registry is empty |
| `OCEAN_BEDROCK_CORS_ORIGIN` | `*` | CORS origin |
| `CLOUDFLARE_ACCOUNT_ID` | unset | Required for Workers AI embeddings and Vectorize |
| `CLOUDFLARE_API_TOKEN` | unset | Required for Workers AI embeddings and Vectorize |
| `OCEAN_VECTORIZE_INDEX` | `ocean-longhouse-context` | Cloudflare Vectorize index name |
| `OCEAN_EMBEDDING_MODEL` | `@cf/baai/bge-base-en-v1.5` | Workers AI embedding model |
| `OCEAN_EMBEDDING_DIMENSIONS` | `768` | Embedding vector dimensions |

## Postgres readiness

Check whether the connected Postgres has the Longhouse/Ocean Ledger schema:

```bash
npm run db:check
```

Apply the schema intentionally:

```bash
DATABASE_URL="$DATABASE_URL" npm run db:migrate -- --yes
```

To enable Postgres-backed Ocean Ledger after applying `db/001_longhouse_core.sql` and `db/002_ocean_ledger.sql`:

```bash
OCEAN_LEDGER_STORE=postgres npm start
```

## Safety notes

- Use HTTPS or put this behind Cloudflare Tunnel/Tailscale before remote use.
- Do not expose this publicly with a weak bootstrap token.
- Tokens should be scoped per agent/team when possible.
- RAID/storage durability is separate from backups. Back up `data/`.
