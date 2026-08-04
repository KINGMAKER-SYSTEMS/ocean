# KINGMAKER Ocean Monorepo

## Purpose

This private repository is the unified team workspace for Ocean. Imported
projects retain their own local `AGENTS.md`, build systems, license notices,
security boundaries, and release processes.

## Routing

| Work concerns | Start in |
| --- | --- |
| Runtime, daemon, permissions, providers, sessions, TUI, CLI | `platform/ocean-os/` |
| Web/native/editor/mobile/voice/canvas UI | `apps/ocean-surface/` |
| Reusable profiles, packages, and generic transports | `packages/ocean-agents/` |
| Shared authenticated knowledge and workflow records | `services/ocean-bedrock/` |
| Rising Tides production assistants and private SOPs | `org/risingtides-agents/` |

## Contracts

- Read this file, then every `AGENTS.md` from the selected project root to the
  target path before editing.
- Do not move private implementation or organization-specific behavior into a
  public project subtree.
- Do not flatten build systems or dependency graphs merely because the projects
  share this repository.
- Run the narrow validation named by the owning project's nearest contract.
- Preserve project-local license, notice, and trademark files.
- Changes spanning projects must document their interface and rollout order.

## Repository migration

The original repositories remain active during migration. Do not archive,
delete, or redirect them until the team has validated CI, releases, deployments,
branch protections, and contributor access from this monorepo.
