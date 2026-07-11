# Courier Hub — Architecture

The courier system is a **hub** over the Ocean OS daemon: narrow, single-purpose
courier agents that get invoked by command, do one job, and report back. Slack is
both the **control plane** (slash commands in) and a **delivery plane** (results
out, over the shared Slack route).

```
  Slack  ──/command──▶  intake adapter  ──▶  hub router  ──┬─▶  deterministic: run harness
 (operator)          (Socket Mode bridge) (this layer)     │
      ▲                                                     └─▶  agentic: POST /v1/prompt
      └────────────── results (response_url / chat.postMessage) ◀─  to ocean-daemon (cwd=courier dir)
```

## Layers

| Layer | What | Where | Status |
|---|---|---|---|
| **Intake** | Slack slash-command front door: `/ocean <command> [args]` over the Socket Mode bridge — 3-sec ack, async dispatch, reply via `response_url` | [`../assistants/bridge/slash_intake.py`](../assistants/bridge/slash_intake.py) | ✅ built |
| **Router** | `slash → courier` table from manifests; dispatch deterministic vs agentic | [`hub/router.py`](hub/router.py) | ✅ built |
| **Couriers** | single-purpose agents; each a manifest + protocol + harness | `<courier>/` | 11 packages ✅ (`router.py list`) |
| **Route** | shared transport the couriers ship over | [`transport/slack.py`](transport/slack.py) | ✅ built |
| **Daemon** | always-on Ocean runtime; spawns agentic couriers | `ocean-daemon` (`:4780`) | ✅ exists |

## How a courier is spawned (grounded in ocean-os)

The daemon already supports everything the agentic path needs — no new runtime
work:

- `POST /v1/prompt` accepts a `PromptRequest` with a **`cwd`**
  (`ocean-core/src/lib.rs:77`); the daemon resolves the workspace from it
  (`ocean-daemon/src/main.rs:597`).
- The agent then loads `AGENTS.md` / `CLAUDE.md` from that dir and its ancestors
  as its instructions (`ocean-agent/src/lib.rs:1229`, `load_project_prompt`).
- Tool calls (incl. the courier's `bash`-driven harness) run under the daemon's
  permission gate (`/v1/permissions`), and actions stream on `/v1/agent/events`.

So **spawning a courier = POST a scoped prompt**; the daemon does the rest.

## Deterministic vs agentic

Declared per courier in its `courier.toml`:

- **deterministic** — the router execs the harness directly (no LLM). For pure
  mechanical jobs like `file-courier /ship`. Fast, cheap, bulletproof.
- **agentic** — the router POSTs a scoped prompt; a daemon agent applies judgment
  (e.g. a Slingshot-style discover → rank → format → deliver), driving its harness.
  The router folds the Slack thread key (`slack:<courier>:<channel>:<thread_ts|dm>`)
  into a deterministic UUIDv5 `session_id` so a thread resumes one daemon session,
  retries transient daemon failures, and posts the agent's reply back over the
  Slack route.

## Manifest schema (`<courier>/courier.toml`)

```toml
name = "file-courier"
description = "…"
mode = "deterministic"        # or "agentic"
entry = "bin/courier"         # deterministic: harness to exec

[[commands]]
slash = "/ship"
subcommand = "send"           # deterministic: harness subcommand
summary = "…"
usage = "/ship --channel <id>"
# agentic variant uses `prompt = "…"` instead of `subcommand`.
```

The router (`hub/router.py`) discovers all manifests, so adding a courier =
drop a folder with a `courier.toml`; no router edits.

## Secrets

Tokens follow Ocean's own convention (`OCEAN_<X>_…` first, bare fallback, then a
file) — see `ocean-providers` `credential_env_names`. The Slack route uses
`OCEAN_SLACK_BOT_TOKEN` → `SLACK_BOT_TOKEN` → `~/.slack_token`. Nothing hardcoded.

## Intake — how the front door actually landed

The originally-planned intake was an outward-facing HTTPS adapter (Slack app +
signing secret + public ingress). It was never built — and stays deliberately
unbuilt — because the Slack bridge (`assistants/bridge/socket_listener.py`)
already holds a Socket Mode WebSocket, and Slack delivers slash commands over
that same socket (`slash_commands` envelopes). The intake
([`../assistants/bridge/slash_intake.py`](../assistants/bridge/slash_intake.py))
rides it: **no public URL, no signing secret, no new tokens**. One registered
command (`/ocean <courier-slash> [args]`) fronts this entire hub via the
router's registry; direct per-slash registration also routes with no code
change. Dispatch reuses this router's `load_registry` / session fold /
`call_daemon` — see `assistants/bridge/README.md` and `RUN.md` §7 (activation).

## Not yet built (deliberately)

- **HTTPS intake adapter** — the signed public-ingress variant remains unbuilt
  on purpose; the Socket Mode intake above covers the front door with a smaller
  surface. Revisit only if the bridge's socket ever goes away.
- **Forward-compat with the planned ocean-os plugin runtime** (`ROADMAP.md`:
  *Subprocess plugins*, *Skill/prompt packs*) — manifests are designed to map
  onto it when it lands.
