# Ocean Bedrock Epoch 01 — P1 Secret Exclusion Evidence

Generated: 2026-06-20T05:38:00Z  
Owner: Navigator

## Implemented

- Added shared ingest filters: `src/ingest-filters.mjs`.
- `scripts/ocean-bootstrap.mjs`, `scripts/ocean-local-app.mjs`, and `scripts/ocean-ingest-local.mjs` now import the same `DEFAULT_IGNORES` source.
- `scripts/ocean-ingest-local.mjs` now excludes secret-pattern paths by default even when `--all` is used.
- Added explicit supervised override: `--include-secrets`.
- Dry-run/report manifests include:
  - `totals.skippedByReason`
  - per-source `skippedByReason`
  - per-source `skippedFiles` with `rel`, `reason`, and `matchedPattern`.
- Added checks:
  - `scripts/check-secret-globs.mjs`
  - `npm run secret:globs:check`
- Updated coworker/local GUI docs with secret exclusion and scope guidance.
- Added local GUI `/api/preview` dry-run endpoint and a preview panel that surfaces changed/skipped counts plus sensitive exclusions before sync.

## Verification

### Syntax

```txt
node --check src/ingest-filters.mjs
node --check scripts/ocean-ingest-local.mjs
node --check scripts/ocean-bootstrap.mjs
node --check scripts/ocean-local-app.mjs
node --check scripts/check-secret-globs.mjs
node --check scripts/secret-scan.mjs
```

Result: all passed.

### Secret glob check

```txt
npm run secret:globs:check
```

Result:

```json
{
  "ok": true,
  "sensitiveChecked": 13,
  "noiseChecked": 4,
  "normalChecked": 3
}
```

### Dry-run fixture

A temp folder containing `notes.md`, `.env`, `.ssh/id_rsa`, and `secret.pem` was run through:

```txt
node scripts/ocean-ingest-local.mjs --config <tmp>/bootstrap.json --state <tmp>/state.json --dry-run
```

Result summary:

```json
{
  "ok": true,
  "totals": {
    "scanned": 1,
    "skipped": 3,
    "skippedByReason": { "sensitive": 3 },
    "changed": 1,
    "uploaded": 0
  },
  "skippedFiles": [
    { "rel": ".env", "reason": "sensitive", "matchedPattern": "env-file" },
    { "rel": ".ssh", "reason": "sensitive", "matchedPattern": "secret-directory" },
    { "rel": "secret.pem", "reason": "sensitive", "matchedPattern": "pem-key" }
  ]
}
```

### Regression checks

```txt
npm run secret:scan
npm run smoke
npm run db:check
```

Results:

- secret scan: pass, 94 tracked files scanned
- smoke: `ocean-bedrock smoke test passed`
- db check: ok, schemaReady true, sourceAdaptersSeeded 8, Vectorize chunks 102/102, graph 116 nodes / 114 edges

## Remaining P1 gaps

- GUI now has a first dry-run preview panel, but still needs a hard acknowledgement gate before first real sync when sensitive files are detected.
- One-time invite exchange is not implemented yet.
- Token registry remains volume JSON unless/until the Postgres auth-token phase is implemented.
