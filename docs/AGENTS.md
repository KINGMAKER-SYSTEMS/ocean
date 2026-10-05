# Migration documentation

## Purpose

Keep canonical repository routing and the public/private split discoverable.

## Ownership

- `MIGRATION.md` owns cutover and existing-work migration instructions.
- `SOURCE_RECONCILIATION.md` owns verified source-tip comparisons, unresolved divergence, and catch-up constraints.

## Local Contracts

- Public code lands in `KINGMAKER-SYSTEMS/ocean`; private operational work lands in `KINGMAKER-SYSTEMS/ocean-private`.
- Do not publish historical mixed-repository branches or private PR discussions.

## Work Guidance

- Preserve source repositories and private history; port only selected public diffs.

## Verification

- Check local Markdown targets and repository routing against current remote metadata.

## Child devlog Index

- None.
