# Ocean Bedrock Repo Contract

## Purpose

Repo-wide working contract for `ocean-bedrock`.

## Ownership

- This file governs repo-wide work in `src/`, `scripts/`, `docs/`, `db/`, `test/`, `workflows/`, and top-level durable files.
- `docs/HANDOFF.md` is the concise canonical current-state handoff; `docs/DEV-LOG.md` is the canonical long-form implementation and deployment history.
- `docs/OCEAN_PROJECT_MAP.md` maps how `ocean-bedrock` connects to `ocean-os`, `ocean-agents`, and `ocean-surface`.
- `events.md` is the append-only repository ledger.

## Local Contracts

- Read `README.md`, `docs/DEV-LOG.md`, and `docs/HANDOFF.md` before changing service behavior or writing durable handoff notes.
- Read `docs/OCEAN_PROJECT_MAP.md` before making cross-repo claims about Ocean ownership or data flow.
- Keep durable docs aligned with verified repo state.
- Record meaningful repository work in root `events.md`.
- Do not move or overwrite secrets, token files, or generated runtime state unless the task explicitly targets them.

## Work Guidance

- Prefer existing repo scripts for validation.
- For runtime/UI changes, run `node --check` on the touched script and the relevant smoke or source checks.
- For auth, ingest, or source-registry changes, use the existing `secret:*`, `source:check`, and `smoke` scripts where relevant.

## Verification

- `npm run smoke`
- `npm run source:check`
- `npm run secret:scan`
- `npm run secret:globs:check`
- `node --check <touched .mjs file>`

## Federated rooms boundary (stable rule)

- Federated rooms require auth, ledger, and rooms on the SAME Postgres
  database (resolved-URL equality); any split, file, or JSONL configuration
  fails closed with 503 on room routes and never logs a database URL.
- Room protocol changes are proven ONLY with the quadruple-gated, create-only
  `npm run rooms:pg-smoke` against a fresh disposable `_test` database
  (TEST_DATABASE_URL + ROOMS_PG_SMOKE_ALLOW_RESET=1 + `_test` name suffix +
  no-preexisting-`longhouse`-schema check). Never point it at a shared or
  live database; `DATABASE_URL` is never a fallback.
- The room event stream (`GET /api/v1/rooms/{room_id}/events`) is a stable
  authenticated boundary: header-bearer only (query tokens never read),
  membership-gated with no admin bypass, composed from the cursor read
  surface via bounded forward-poll — no push/notification primitive may be
  added without its own reviewed freeze.

## Child devlog Index

- `docs/` — active architecture, API, operations, handoff, and historical documentation → `docs/AGENTS.md`
