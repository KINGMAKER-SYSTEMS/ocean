# ocean-agents documentation contract

## Purpose

Keep public package architecture, cross-repository routing, build registration,
and daemon interaction guidance aligned with executable manifests and profiles.

## Ownership

- `OCEAN_PROJECT_MAP.md` owns public cross-repository routing.
- `ocean-agents-builds.toml` owns the package-side build register.
- `AGENT_FILESYSTEM_ARCHITECTURE.md` owns the typed agent-filesystem direction.
- `PYTHON_TO_RUST_MIGRATION.md` records generic harness-to-runtime migration.
- `DAEMON_INTERACTION.md` documents the public daemon contract packages use.

## Local contracts

- Public docs must not name private agent packages, organizations, campaigns,
  artists, destinations, internal workflows, or private repository paths.
- Courier manifests and `python3 couriers/hub/router.py list` are authoritative
  for live public package inventory.
- Do not claim project-map parity unless verified across maintained copies.
- Keep local Markdown links valid.
- Record meaningful work in root `events.md`.

## Verification

- `make assistants-check`
- `python3 couriers/hub/router.py list`
- Check every changed Markdown link target.

## Child devlog index

- None.
