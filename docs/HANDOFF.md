# Ocean Bedrock Handoff

- Verified: 2026-07-14
- Repository: `risingtides-dev/ocean-bedrock`
- Default branch: `master`
- Verified base commit: `576c5cb2e63a9c106ef99bc8d5b0886417975367`
- Service: `ocean-bedrock`
- Public URL: <https://ocean-bedrock-production.up.railway.app>

This is the current-state handoff. Historical implementation notes, deployment IDs, and superseded counts belong in [`DEV-LOG.md`](DEV-LOG.md), not here.

## Current role

Ocean Bedrock is the authenticated shared data and knowledge substrate for the Ocean system. It owns shared files, scoped access, locks, source ingestion, ledger events, semantic search, graph context, and its MCP wrapper.

It does not own agent execution, provider routing, sessions, or product surfaces:

- `ocean-os` owns daemon and runtime authority.
- `ocean-surface` owns browser, extension, and native product surfaces.
- `ocean-agents` owns provider-agnostic assistant and courier packages.
- `ocean-bedrock` owns the shared data and knowledge plane.

Use [`OCEAN_PROJECT_MAP.md`](OCEAN_PROJECT_MAP.md) for cross-repo routing.

## Verified repository state

The following checks passed from a clean checkout on 2026-07-14 after `npm ci`:

```text
npm test                    37/37 passed
npm run smoke               passed
npm run source:check        passed
npm run secret:scan         passed (115 files)
npm run secret:globs:check passed
```

The public unauthenticated endpoints also responded successfully:

```text
GET /health  -> ok=true, instance=ocean-bedrock, version=0.1.0
GET /        -> route index with current API links
```

These checks attest the repository and public health surface. They do not attest private production data counts, database migrations, scheduled jobs, mounts, tokens, or deployment identity. Verify those with authorized production access before making operational claims.

## Runtime shape

- Node.js 22 or newer; entry point `src/server.mjs`.
- Default local port `8080`.
- Durable root defaults to `./data`; production commonly overrides `OCEAN_BEDROCK_ROOT`.
- Bearer-token roles and path scopes protect authenticated routes.
- File operations route through storage adapters.
- Source ingestion records lineage and queues indexing work.
- Text plus PDF/DOCX content can enter the chunk, embedding, and graph pipeline.
- PDF/DOCX objects uploaded before binary extraction was deployed require an explicit backfill, then worker processing:

```bash
npm run ocean:backfill:binary -- --dry-run --limit 100
npm run ocean:backfill:binary -- --limit 100
npm run ocean:worker
```

Always inspect the dry run before enqueueing production work.
- Postgres is selected for the ledger when `DATABASE_URL` is present; otherwise JSONL is used.
- Workers AI and Vectorize require the Cloudflare environment documented in the README.

Canonical setup and configuration live in [`../README.md`](../README.md). API details live in [`API.md`](API.md), [`openapi.yaml`](openapi.yaml), and the runtime `GET /api/v1/openapi.yaml` endpoint.

## MCP surface

`scripts/ocean-bedrock-mcp.mjs` currently exports 14 tools:

```text
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

`test/toolbox-manifest.test.mjs` pins the toolbox manifest to the MCP exports so these lists cannot silently diverge.

## Known limitations

- `.pptx`, `.xlsx`, and `.rtf` uploads are stored but are not indexable by the binary extraction path.
- `/vault` is a folder placeholder, not an encrypted secrets manager.
- Plain file search and semantic search are separate APIs.
- Some source integrations still depend on adapter-specific implementation and credentials; run `npm run source:check` and inspect the relevant runner before promising support.
- The active auth store depends on deployment configuration. Do not assume Postgres auth cutover from the existence of schema/code alone.
- Basic health does not prove private mounts, database state, embeddings, triage schedules, or Telegram delivery.

## Safety invariants

1. Never commit, print, or paste raw tokens, database URLs, or provider credentials.
2. Use scoped contributor tokens for normal testing; reserve admin tokens for admin-only checks.
3. Keep coworker folder ingestion opt-in and inspect dry-run exclusions before syncing.
4. Do not describe Bedrock as the Ocean runtime or Longhouse coordination brain.
5. Do not claim production counts or deployment status from historical notes.
6. Use HTTPS, Tailscale, or another encrypted transport for remote access.

## Local verification

```bash
npm ci
npm test
npm run smoke
npm run source:check
npm run secret:scan
npm run secret:globs:check
```

Additional checks require appropriate environment and credentials:

```bash
npm run db:check
curl -fsS https://ocean-bedrock-production.up.railway.app/health
```

## Handoff checklist

Before updating this handoff:

- Record the commit and branch being handed off.
- Run the local verification suite.
- Separate repository facts from production facts.
- For production claims, capture the verification time and command without recording secrets.
- Put narrative implementation and deployment history in `DEV-LOG.md`; append the required concise work record to `events.md`.
- Keep this file short, current, and free of appended update sections.
