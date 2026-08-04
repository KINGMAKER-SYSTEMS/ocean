# Public Agent Filesystem Architecture

Status: current public package boundary and direction.

## Purpose

`ocean-agents` is the filesystem package layer above Ocean OS. It owns authored
behavior and declarative package data. Ocean OS owns execution, permissions,
providers, tools, sessions, memory, and daemon authority.

```text
public package data
  profiles + manifests + SOPs + fixtures
                    |
                    v
Ocean daemon/runtime
  permissions + tools + sessions + providers
                    |
                    v
client surfaces
```

## Repository boundary

Public packages are reusable without organization-specific accounts, knowledge,
destinations, campaigns, media, or workflows. Private deployments may compose
or extend public packages, but private behavior and records never flow back into
this repository by default.

Bedrock is an optional authenticated data plane. A public package may declare a
generic context capability, but it must remain useful without Bedrock and may
not embed private namespaces or data.

## Package classes

### Surface profile

Authored under `assistants/_shared/` and `assistants/_base/<SURFACE>/`, then
composed into `assistants/<SURFACE>/system.md`. Ocean loads the published profile
according to the turn's client type.

### Named assistant

A focused specialist directory may contain:

```text
assistants/<name>/
├── AGENTS.md
├── CLAUDE.md
├── system.md
├── agent.toml          # planned typed manifest
├── harness/            # transitional deterministic code
└── artifacts/          # provenance-cleared templates only
```

Bonzai is the public reference.

### Courier

A narrow delivery package contains `courier.toml`, protocol files, and a
harness or agentic contract. `couriers/hub/router.py` discovers manifests; it
does not carry package-specific routing logic. `file-courier` is the public
reference.

## Authority split

| Concern | Owner |
| --- | --- |
| Profile text, package manifests, package SOPs | `ocean-agents` |
| Tool execution, permission gates, cancellation | `ocean-os` runtime |
| Sessions, turns, queueing, events | `ocean-daemon` |
| Provider authentication and model routing | `ocean-os` providers/protocol |
| Client rendering and interaction | `ocean-surface` and editor/TUI clients |
| Optional shared team knowledge | private `ocean-bedrock` deployment |

Packages can request capabilities; they cannot grant themselves permissions.

## Build register

The machine-readable public build register is
[`ocean-agents-builds.toml`](ocean-agents-builds.toml).

| Build | Status | Purpose |
| --- | --- | --- |
| A1 | filesystem convention | Typed package manifests |
| A2 | filesystem convention | Composed surface profile catalog |
| A3 | filesystem convention | Named-assistant reference |
| A4 | filesystem convention | Courier reference and manifest router |
| A5 | transitional harness | Composition, syntax, and inventory gates |
| A6 | proposed | Pure-content cutover after verified Rust replacements |

## Composition invariant

The daemon currently loads one published surface file. The deterministic
composer joins shared identity, per-surface sources, and optional specialist
overrides. Generated output and authored source are committed together.

```bash
make assistants-compose
make assistants-check
```

## Typed direction

Future manifests should declare, without embedding credentials:

- package class and mode;
- surfaces or routes;
- profile and working-directory paths;
- requested tool/capability identifiers;
- input/output contract schemas;
- retry and closeout semantics; and
- optional generic context requirements.

The Rust runtime parses and enforces those declarations. This repository does
not implement a second runtime or scheduler.

## Transitional Python

Python currently supplies deterministic composition, manifest discovery, shared
Slack transport, and reference harnesses. Remove a Python path only after its
Rust replacement has migrated callers and passed parity checks. See
[`PYTHON_TO_RUST_MIGRATION.md`](PYTHON_TO_RUST_MIGRATION.md).

## Verification

- `make assistants-check`
- `python3 couriers/hub/router.py list`
- compile changed Python files
- validate changed Markdown links
- run a public secret scan
