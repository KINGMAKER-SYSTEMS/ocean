# Ocean Bedrock Documentation Contract

## Purpose

Keep Bedrock's active architecture, API, operations, and handoff documentation aligned with verified repository and production state.

## Ownership

- `HANDOFF.md` is the concise current-state handoff.
- `DEV-LOG.md` owns chronological implementation and deployment history.
- `OCEAN_PROJECT_MAP.md` routes work across the Ocean repositories.
- `API.md` and `openapi.yaml` describe the HTTP API; runtime behavior remains authoritative.
- `PUBLIC-EXPOSURE-AND-LICENSE-PROVENANCE-AUDIT-2026-07-19.md` is the private redacted baseline and action register for the 2026-07-19 public-readiness audit; raw scanner reports and credential values never belong in docs.
- The remaining documents are scoped references for the named subsystem or workflow, not automatically current-state authorities. Verify their claims against source and tests before reuse; `OCEAN-LONGHOUSE.md` is an architectural direction document and may describe planned work already implemented elsewhere.

## Local Contracts

- Do not append dated updates to `HANDOFF.md`; replace stale current-state claims and move narrative history to `DEV-LOG.md`. Append the required concise repository work record to `events.md`.
- Distinguish repository-verified facts from credentialed production facts.
- Date production verification and never copy secrets, private URLs, or raw token values into documentation.
- Do not claim that project-map copies are identical unless parity is verified across all maintained copies.
- Keep local links valid and remove references to absent artifacts.

## Work Guidance

- Prefer source, tests, `package.json`, and live authorized checks over historical prose.
- Keep current docs concise. Preserve implementation history in `DEV-LOG.md` rather than repeating it across active guides.
- Use the root `AGENTS.md` verification commands for documentation changes.

## Verification

- `npm test`
- `npm run smoke`
- `npm run source:check`
- `npm run secret:scan`
- `npm run secret:globs:check`
- Check every changed Markdown link target.

## Child devlog Index

- None.
