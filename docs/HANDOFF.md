# Ocean Bedrock Handoff

- Verified: 2026-07-16
- Repository: `risingtides-dev/ocean-bedrock`
- Default branch: `master`
- Working branch: `feat/s1c-room-events` (unmerged), branch base `f5e8846`
- Gate-2 S1A merge: `2e3079b`; S1B final merge: `1f20784`
- S1C reviewed implementation tip: `3fdacac` (source/integration gate green)
- S1D local protocol-harness tip: `d37c936` (not pushed)
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

The following checks passed on 2026-07-16 on branch `feat/s1c-room-events`
(Gate-2 S1C authenticated SSE room fanout plus the local S1D protocol gate;
S1A membership and S1B producer idempotency/cursor already merged on
`master`):

```text
npm test                    93/93 passed (incl. room-events + S1D harness suites)
npm run smoke               passed (F3 federated 503s, seven routes)
npm run source:check        passed
npm run secret:scan         passed (121 files)
npm run secret:globs:check  passed
node --check                clean on every touched .mjs file
npm run rooms:protocol-gate 122/122 underlying PG checks; G2-AC1..5 + G2-AC10 matrix all PASS (fresh disposable local PG14 `_test` database)
npm run db:check            green against that same disposable database: schemaReady/authRegistryReady/roomMembersReady/owner-guard trigger all true, 8 adapters seeded; stopped and discarded afterwards; no live database touched
```

These checks attest the repository only. The S1C/S1D branch is unmerged, the Gate-2 changes are NOT deployed
and nothing here is production-attested: private production data counts,
database migrations (006 has not been applied to any live database), scheduled
jobs, mounts, tokens, and deployment identity all require authorized production
access before operational claims.

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
- Registered rooms expose an authenticated SSE stream (`GET /api/v1/rooms/{room_id}/events`): header-only bearer, membership-gated with no admin bypass, scoped cursor replay (snapshot through a high-water mark) followed by a continuing live forward-poll loop — composed from the cursor read surface; there is NO push/notification primitive.
- Postgres is selected for the ledger when `DATABASE_URL` is present; otherwise JSONL is used.
- Federated room routes require auth, ledger, and rooms to resolve to one shared
  Postgres database; split or file-backed configurations fail closed with 503.
- Room producer tuples are content-checked for idempotency, and ledger consumers
  can page forward with the strict-string `after_sequence` cursor.
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
npm run rooms:protocol-gate
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
