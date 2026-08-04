# ocean-agents

Provider-agnostic agent package tooling and editable surface profiles for
[Ocean OS](https://github.com/Risingtides-dev/ocean-os).

This public repository contains reusable mechanisms only: composed surface
profiles, package conventions, generic delivery transport, and reference
packages. Credentials, destinations, organization-specific agents, production
workflows, campaign material, and the private shared knowledge plane are not
part of it.

Ocean OS is provider-neutral by design. Profiles and package protocols here do
not depend on a specific model provider, while deterministic transport and I/O
stay in narrow harnesses that any runtime can drive.

**Start with [`AGENTS.md`](AGENTS.md)** for the package index and repository
contracts.

## Package families

| Group | Purpose | Included references |
| --- | --- | --- |
| [`assistants/`](assistants) | Editable surface behavior loaded by Ocean OS at turn time | Ten composed house profiles and the Bonzai git-hygiene specialist |
| [`couriers/`](couriers) | Manifest-discovered delivery packages over shared transport | Generic `file-courier` with `/ship`, `/say`, and `/resolve` |

The live courier registry comes from the manifests:

```bash
python3 couriers/hub/router.py list
```

## Layout

```text
ocean-agents/
├── assistants/
│   ├── _shared/              # cross-surface identity
│   ├── _base/<SURFACE>/      # authored per-surface sources
│   ├── <SURFACE>/system.md   # generated profile Ocean OS loads
│   ├── bonzai/               # public named-specialist reference
│   └── tools/                # profile composer
├── couriers/
│   ├── hub/router.py         # manifest discovery and dispatch
│   ├── transport/            # shared delivery transport
│   └── file-courier/         # generic reference package
├── docs/                     # package/runtime architecture
└── README.md
```

## Surface profiles

Published `assistants/<SURFACE>/system.md` files are generated artifacts. Edit
`assistants/_shared/` or `assistants/_base/<SURFACE>/`, then compose and check:

```bash
make assistants-compose
make assistants-check
```

The current profiles cover ACP, browser, CLI, canvas, GUI, mobile, Slack, TUI,
voice, and web surfaces.

## Courier example

```bash
python3 couriers/hub/router.py resolve /ship
python3 couriers/hub/router.py run --dry-run /ship --channel C0123ABCD
```

To run the reference package directly:

```bash
cd couriers/file-courier
cp courier.env.example courier.env
bin/courier resolve "https://workspace.slack.com/archives/C0123ABCD"
bin/courier send --channel C0123ABCD
```

Always confirm a destination before sending. Never hardcode tokens or channel
IDs; use runtime environment/configuration.

## Repository boundary

This repository is intentionally reusable and organization-neutral. Private
agent deployments may consume these profiles and conventions, but their SOPs,
media, destinations, operational ledgers, and company knowledge remain in
private repositories and services.

The public Ocean repositories are:

- [`ocean-os`](https://github.com/Risingtides-dev/ocean-os) — Rust runtime,
  daemon, tools, and TUI;
- [`ocean-surface`](https://github.com/Risingtides-dev/ocean-surface) — thin
  client surfaces; and
- **`ocean-agents`** — profiles and reusable package conventions.

`ocean-bedrock` is a private, optional authenticated knowledge/data plane for
team deployments. Ocean OS and every public package must remain useful without
it. See [`docs/OCEAN_PROJECT_MAP.md`](docs/OCEAN_PROJECT_MAP.md).

## License status

No repository-wide license has been granted yet. A license and contribution
policy will be added after the provenance and governance review is complete.
