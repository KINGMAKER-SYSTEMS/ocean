# Courier Hub — Architecture

The courier system is a manifest-discovered delivery layer over Ocean OS.
Couriers are narrow packages that perform one bounded delivery task and report
back. The public reference package is `file-courier`.

```text
operator or client
      |
      v
hub/router.py -- manifest lookup
      |
      +-- deterministic package -> harness
      |
      +-- agentic package -> ocean-daemon scoped turn
      |
      v
shared transport -> confirmed destination
```

## Layers

| Layer | Responsibility | Location |
| --- | --- | --- |
| Router | Discover `courier.toml`, resolve commands, dispatch by mode | [`hub/router.py`](hub/router.py) |
| Package | Declare commands and own protocol/harness | `<courier>/` |
| Transport | Shared delivery API, retries, and normalization | [`transport/slack.py`](transport/slack.py) |
| Runtime | Execute permission-gated agentic turns | `ocean-daemon` |

Run `python3 hub/router.py list` for the authoritative package/command inventory.

## Deterministic and agentic modes

A package declares its mode in `courier.toml`:

- **deterministic** — the router executes the package harness directly;
- **agentic** — the router posts a prompt scoped to the package directory, so
  Ocean loads its `AGENTS.md` and `CLAUDE.md` under the normal permission model.

The same manifest format supports both without router conditionals.

## Manifest example

```toml
name = "file-courier"
description = "Upload files or send a message to a confirmed Slack destination"
mode = "deterministic"
entry = "bin/courier"

[[commands]]
slash = "/ship"
subcommand = "send"
summary = "Upload files"
usage = "/ship --channel <id>"
```

Adding a courier means adding a folder and manifest; the router discovers it.

## Destination and secret invariants

- Confirm shared destinations before delivery.
- Never commit tokens or destination IDs.
- Prefer Ocean-prefixed environment variables, such as
  `OCEAN_SLACK_BOT_TOKEN`, with documented local fallback only.
- Keep authentication, retries, rate-limit handling, and payload normalization
  in shared transport or the deterministic harness.
- Do not bypass the harness from an agent prompt.

## Slack intake

The public repository does not ship a Slack Socket Mode application. A host may
route slash commands or other client input into `hub/router.py`, or invoke it
directly. Deployment-specific intake, app identities, signing configuration,
and destinations belong to the deployment.
