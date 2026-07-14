# ocean-agents documentation contract

## Purpose

Keep cross-repo routing, package architecture, build registration, and fleet-porting documentation aligned with executable manifests and repository state.

## Ownership

- `OCEAN_PROJECT_MAP.md` owns cross-repo routing from this repository.
- `ocean-agents-builds.toml` owns the codified package-side build list.
- `AGENT_FILESYSTEM_ARCHITECTURE.md` owns the typed agent-filesystem direction.
- `PORTING_ROADMAP.md` owns the active migration convention and wave plan.
- Other documents are scoped references for their named subject; verify current claims before reuse.

## Local Contracts

- The courier manifests and `python3 couriers/hub/router.py list` are authoritative for the live courier inventory; prose must not override them.
- Do not claim that project-map copies are identical unless parity is verified across all maintained copies.
- Keep local links valid and remove references to absent artifacts.
- Separate active architecture and workflow from historical planning notes.

## Work Guidance

- Validate package counts and slash commands from the router rather than counting prose tables.
- Keep cross-repo ownership claims consistent with the project map and each target repository's root contract.
- Record meaningful documentation work in root `events.md`.

## Verification

- `python3 couriers/hub/router.py list`
- Check every changed Markdown link target.
- Run the narrow validation named by any changed build or architecture document.

## Child devlog Index

- None.
