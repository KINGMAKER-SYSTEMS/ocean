# Ocean Bedrock — Week One Runbook

Goal for this week: get the cloud box running, issue authenticated MCP/Bearer tokens, let coworkers bootstrap selected local folders, and begin feeding approved files into the Ocean knowledge layer.

This runbook follows the canonical Ocean split from `ocean-os/docs/LONGHOUSE.md`:

- `ocean-daemon` remains local execution authority.
- `ocean-longhouse` is the hive brain / coordination layer.
- This repo is the Bedrock cloud-box prototype that can host supporting storage, context, ledger, ingest, and MCP-facing infrastructure.

## Milestones

### 1. Cloud service live

- Start this HTTP service behind HTTPS/Cloudflare/Railway.
- Ensure `DATABASE_URL` is connected.
- Apply migrations:

```bash
npm run db:migrate -- --yes
npm run db:check
```

- Run with Postgres-backed Ocean Ledger:

```bash
OCEAN_LEDGER_STORE=postgres npm start
```

### 2. Authenticated tokens

For each coworker, issue a scoped token. Treat this as the first MCP token until the MCP gateway wraps the same auth.

Example:

```bash
npm run token:create -- \
  --name alice-mcp \
  --role contributor \
  --scope /coworkers/alice \
  --scope /context/ocean-bedrock \
  --scope /sessions/alice
```

Send the raw token once over a secure channel. Store only hashes server-side.

### 3. Coworker bootstrap

Coworker runs:

```bash
npm run ocean:bootstrap
```

or non-interactive:

```bash
npm run ocean:bootstrap -- \
  --server https://bedrock.example.com \
  --token '<MCP_TOKEN>' \
  --name alice \
  --folder ~/Documents \
  --folder ~/Projects/ocean-notes \
  --yes
```

This writes:

```txt
~/.config/ocean-bedrock/bootstrap.json
~/.config/ocean-bedrock/env
```

and registers a redacted source manifest under:

```txt
/context/ocean-bedrock/sources/<coworker>-<device>.json
```

### 4. First ingestion pass

Coworker runs:

```bash
npm run ocean:ingest
```

Dry run:

```bash
npm run ocean:ingest -- --dry-run
```

The ingest client:

- scans selected local folders,
- skips common build/cache folders,
- uploads only changed files,
- defaults to document/code/knowledge extensions,
- writes a manifest under `/context/ocean-bedrock/manifests/`,
- appends an Ocean Ledger event: `ingest.local_folder_synced`.

### 5. Ingestion pipeline next

After files land in Bedrock/Longhouse storage:

1. enqueue `ingest_jobs.index_object`,
2. extract text and metadata,
3. chunk content,
4. embed through Cloudflare Workers AI,
5. upsert vectors into `ocean-longhouse-context`,
6. update graph nodes/edges in Postgres,
7. cache hot path/query results in `OCEAN_LONGHOUSE_CACHE`.

## Cloudflare resources already provisioned

- R2 bucket: `ocean-longhouse`
- Vectorize index: `ocean-longhouse-context`
- KV namespace: `OCEAN_LONGHOUSE_CACHE`

## Safety gates

- Start with read/write file ingest only; no destructive remote actions.
- Tokens should be scoped by coworker path.
- Do not ingest secrets folders by default.
- Do not ingest hidden/build/cache directories.
- Add allow/deny review before enabling background daemon sync.
- Ocean Longhouse coordination must not bypass `ocean-daemon` local permission gates.
