# KINGMAKER Ocean — Devlog Root

## Purpose

Public canonical workspace for Ocean runtime, clients, and reusable packages.
All four Kingmaker maintainers work from this repository.

## Ownership

- Canonical repository: `KINGMAKER-SYSTEMS/ocean`. New code, branches, PRs, and releases for public components belong here, not the personal source repositories.
- Maintainers: `Risingtides-dev`, `ecfromthedc`, `jakebalik-bit`, and `jayvespertine`; each retains admin access. Public visibility grants no contributor push or merge rights.
- `KINGMAKER-SYSTEMS/ocean-private` owns private Bedrock implementation, production assistants, SOPs, company research, and historical mixed-repository records.

## Local Contracts

- Read this file and every applicable child `AGENTS.md` before editing.
- `main` merges need one APPROVED review on the current head from someone other than the author, plus `Build Ocean` and `Build Surface`. Stale approvals are dismissed, the last push needs its own approval, and admins are bound. A COMMENTED review is not approval. Only maintainers change branch protection.
- Never import `org/`, `services/`, private deployments, production data, credentials, or mixed historical branches into this repository. Port selected public-component diffs onto public `main`.
- Preserve component build systems, licenses, notices, and trademark restrictions. Public use must not require private components.
- Keep runtime authority in Ocean OS; Surface renders state and gathers intent; reusable packages stay organization-neutral.
- Record meaningful work in root `events.md` using 24-hour `HH:MM`, date, agent, worktree, type, area, and validation. Imported historical ledgers remain private.
- After changes, update the nearest owning devlog and affected child indexes.

## Work Guidance

- Automatic Actions are small and build-only: changed-component selection, docs-only compilation skips, development profiles, read-only permissions, timeouts, and same-ref cancellation.
- Build Ocean daemon, TUI, CLI, ACP, MCP and their dependencies; Surface builds its real web bundle, proxy, Tauri shell, and editor extension. Release packaging keeps release profiles.
- Run targeted checks for actual changes, not universal test/lint/audit/MSRV matrices.
- Keep UI minimal: no emoji, sparkle icons, marketing copy, redundant controls, or instructional labels compensating for unclear interaction.
- Preserve dirty checkouts. Do not archive or delete personal source repositories as part of this migration.

## Verification

- `.github/AGENTS.md` owns workflow and scope verification.
- Each component owns narrow source checks and build commands.
- Scan the publication snapshot for credentials and review private-boundary exclusions before publication.

## Child devlog Index

- `.github/` — conditional build workflow → `.github/AGENTS.md`
- `platform/ocean-os/` — runtime, daemon, and tool contracts → `platform/ocean-os/AGENTS.md`
- `apps/ocean-surface/` — client contracts → `apps/ocean-surface/AGENTS.md`
- `packages/ocean-agents/` — reusable package contracts → `packages/ocean-agents/AGENTS.md`
- `docs/` — canonical repository migration and split boundary → `docs/AGENTS.md`
