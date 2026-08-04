# risingtides-agents

Private production agent catalog for Rising Tides deployments on
[Ocean OS](https://github.com/Risingtides-dev/ocean-os).

This repository contains organization-specific assistants, couriers, Slack
intake, SOPs, prompt libraries, internal skills, factory workflows, and package
harnesses. It is private and must not be made public as a whole.

Reusable, organization-neutral profiles and reference packages live in the
public [`ocean-agents`](https://github.com/Risingtides-dev/ocean-agents)
repository. Bedrock remains the separate private knowledge/data plane; this repo
owns behavior, not shared-record storage authority.

**Start with [`AGENTS.md`](AGENTS.md)** for the live package inventory and local
contracts.

## Production package families

- [`assistants/`](assistants) — named surface specialists plus composed house
  profiles. Production specialists include content generation, prompt
  optimization, voice intake, creator research, and long-form format tooling.
- [`couriers/`](couriers) — manifest-discovered automation and delivery packages
  for media, email, distribution, review, and operational maintenance.
- [`docs/orchestrator/`](docs/orchestrator) — private factory and Longhouse
  workflow-discovery references.
- [`internal/`](internal) — proprietary operational skills relocated from public
  Ocean repositories.

The live courier inventory is manifest-driven:

```bash
python3 couriers/hub/router.py list
```

## Boundary

The public/private split is capability-based:

- **Public `ocean-agents`:** reusable mechanism, generic profiles, package
  conventions, Bonzai, file-courier, manifest routing, and generic transport.
- **Private `risingtides-agents`:** Rising Tides identities, operating recipes,
  artists/campaigns, destinations, Slack app behavior, production automation,
  media, internal factory procedures, and private package roadmaps.
- **Private `ocean-bedrock`:** authenticated knowledge, durable records,
  context, handoffs, search/graph, and team data.

See
[`docs/PUBLIC_PRIVATE_REPOSITORY_SEPARATION_2026-07-20.md`](docs/PUBLIC_PRIVATE_REPOSITORY_SEPARATION_2026-07-20.md)
for the executed separation record.

## Local layout

```text
risingtides-agents/
├── assistants/               # production specialists and surface profiles
├── couriers/                 # production automation/delivery packages
├── docs/
│   └── orchestrator/         # private workflow discovery and factory docs
├── internal/                 # private operational material
└── events.md                 # private development ledger
```

## Invariants

- Keep this repository private.
- Never copy private packages, SOPs, media, or workflow documents into public
  Ocean without an explicit allowlist and operator approval.
- Never hardcode credentials or destinations.
- Confirm shared destinations before delivery.
- Do not bypass package harnesses or duplicate shared transport.
- Treat Bedrock as optional data/context authority, not a miscellaneous package
  repository or local execution shell.

## Architecture

- [`docs/OCEAN_PROJECT_MAP.md`](docs/OCEAN_PROJECT_MAP.md) — public/private
  ownership and routing.
- [`docs/AGENT_FILESYSTEM_ARCHITECTURE.md`](docs/AGENT_FILESYSTEM_ARCHITECTURE.md)
  — typed package direction.
- [`docs/risingtides-agents-builds.toml`](docs/risingtides-agents-builds.toml) —
  private package build register.
- [`docs/PORTING_ROADMAP.md`](docs/PORTING_ROADMAP.md) — private production fleet
  migration plan.
