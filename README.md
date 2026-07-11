# ocean-agents

Provider-agnostic agent packages for the [Ocean OS](https://github.com/Risingtides-dev/ocean-os)
ecosystem — the **deployment layer for Rising Tides' agents** (repositioned
2026-07-10, [`docs/ocean-agents-builds.toml`](docs/ocean-agents-builds.toml)
`[positioning]`). Each package pairs a deterministic **harness** (the hands —
pure stdlib, drivable by any runtime via the `bash` tool) with a
`CLAUDE.md`/`AGENTS.md` **protocol** (loaded by whatever model is the brain).

Ocean OS is provider-neutral by design — the agent loop runs under DeepSeek,
Anthropic, OpenAI, or Google interchangeably. These packages preserve that: all
transport/IO logic lives in the harness, so swapping the model never changes
behavior. This repo is intentionally **not** part of the `ocean-os` Rust
workspace (that repo stays pure Rust crates/containers); these agents *run on top
of* the Ocean runtime.

**Start with [`AGENTS.md`](AGENTS.md)** — the agent index. It lists every
package and is loaded automatically by the runtime; this README only orients.

## Two families

| Group | What it is | Details |
|---|---|---|
| [`assistants/`](assistants) | **Surface specialists** — brain-in-the-loop agents an operator *unlocks*; each loads a surface profile (`assistants/<SURFACE>/system.md`) that Ocean OS prefers over its compiled seed prompt. Six named specialists (bonzai, content-agent, prompt-lab, voice-intake, creator-research, yt-prompt-bible) plus composed house profiles for surfaces like `SLACK`, `CNVS`, `MOBL`, `VOX`, `BRWSR`. | [`assistants/README.md`](assistants/README.md) |
| [`couriers/`](couriers) | **Delivery agents** — narrow, single-purpose agents that ship a payload to a destination over a shared route, invoked by slash command through the hub router. Eleven packages today, from `/ship`/`/say` (file-courier) to `/ycp`, `/hopecore`, `/ttup`. | [`couriers/ARCHITECTURE.md`](couriers/ARCHITECTURE.md) |

The registry tables live in `AGENTS.md`; the **live** courier registry is the
router itself (it reads each `courier.toml`, so it never lags):

```bash
python3 couriers/hub/router.py list
```

## Layout convention

```
ocean-agents/
├── assistants/               # surface specialists + house surface profiles
│   ├── _shared/              # cross-surface core identity (composed under every profile)
│   ├── _base/<SURFACE>/      # per-surface house SOP sources (composer inputs)
│   ├── <SURFACE>/system.md   # THE composed profile Ocean OS loads — never hand-edit
│   ├── <assistant>/          # a named specialist (system.md, CLAUDE.md, harness/, bin/)
│   └── bridge/               # Slack Socket Mode intake → daemon
├── couriers/                 # a class: same route, same workflow, different payloads
│   ├── hub/router.py         # slash → courier dispatch (reads the manifests)
│   ├── transport/            # THE route — shared, stdlib, no per-agent dup
│   └── <courier>/            # a species: courier.toml + CLAUDE.md + harness/ + bin/
├── docs/                     # cross-repo map, architecture, build register, roadmap
└── README.md
```

Adding a package is data, not plumbing: a courier = drop a folder with a
`courier.toml` (no router edits); an assistant or surface = drop a folder with a
`system.md` (surface profiles are composed — see the authoring flow in
[`assistants/README.md`](assistants/README.md)).

## Invoking a courier

```bash
python3 couriers/hub/router.py run <slash> [args]   # deterministic → harness; agentic → daemon prompt
```

Or drive one directly, e.g. file-courier:

```bash
cd couriers/file-courier
cp courier.env.example courier.env     # add your Slack bot token (or use ~/.slack_token)
# drop files into dropbox/, then:
bin/courier resolve "https://workspace.slack.com/archives/C0123ABCD"
bin/courier send --channel C0123ABCD
bin/courier message --channel C0123ABCD "done ✅"
```

Invariants (all packages): confirm the destination before sending, never
hardcode tokens or destinations, don't bypass a courier's harness. Full list in
[`AGENTS.md`](AGENTS.md).

## Where this repo sits

The Ocean quad: `ocean-os` (Rust runtime + daemon), `ocean-surface` (thin
clients), **`ocean-agents`** (agent packages + profiles — this repo), and
`ocean-bedrock` (shared knowledge/data plane). Cross-repo routing and ownership:
[`docs/OCEAN_PROJECT_MAP.md`](docs/OCEAN_PROJECT_MAP.md). Package-side build
register: [`docs/ocean-agents-builds.toml`](docs/ocean-agents-builds.toml);
typed-filesystem direction:
[`docs/AGENT_FILESYSTEM_ARCHITECTURE.md`](docs/AGENT_FILESYSTEM_ARCHITECTURE.md);
fleet-porting plan: [`docs/PORTING_ROADMAP.md`](docs/PORTING_ROADMAP.md).
