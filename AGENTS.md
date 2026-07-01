# Ocean Bedrock Repo Contract

## Purpose

Repo-wide working contract for `ocean-bedrock`.

## Ownership

- This file governs repo-wide work in `src/`, `scripts/`, `docs/`, `db/`, `test/`, `workflows/`, and top-level durable files.
- `docs/DEV-LOG.md` and `docs/HANDOFF.md` are the canonical long-form project notes.
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

## Child devlog Index

- None yet.
