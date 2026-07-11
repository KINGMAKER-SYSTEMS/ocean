# Running the content-agent Slack bridge

The inbound bridge (`socket_listener.py`) is the front door for the Slack
assistant: it opens a **Socket Mode** WebSocket, receives Slack events, and pumps
each one through the pipeline:

```
Slack event → dedupe → resolve_context → build_turn
            → dispatch_to_daemon (OCEAN-84 hardened /v1/prompt)
            → reply.deliver (couriers/transport/slack.py, in-thread)
```

`reply` goes out over the existing pure-stdlib transport; only the inbound socket
needs `slack_sdk`.

> **How it works** (the flow, `OCEAN_ASSISTANTS_DIR`, and `reply.py`) is documented
> in [`README.md`](README.md). This file is the **operational** guide — how to set
> up the Slack app and start the listener.

---

## 1. Slack app setup (one app per agent identity — spec §5)

Create a Slack app for `content-agent` and enable **Socket Mode**.

- **Socket Mode** → ON → generate an **app-level token** (`xapp-…`) with the
  `connections:write` scope. This is `SLACK_APP_TOKEN`.
- **Event Subscriptions** (delivered over Socket Mode, no public URL) → subscribe
  the bot to:
  - `app_mention` — mentions in channels/threads
  - `message.im` — direct messages
- **Bot Token Scopes** (`xoxb-…`, this is `SLACK_BOT_TOKEN`):
  - `app_mentions:read`, `im:history`, `im:read` — inbound
  - `chat:write`, `files:write` — outbound replies / files
  - `channels:read` (+ `groups:read` for private channels) — resolve channels
  - `canvases:write` — optional, for canvas rendering
- Install the app to the workspace, then invite the bot to the channels it should
  listen in.

## 2. Environment

Copy the template and fill it in (the real file is gitignored):

```bash
cp assistants/bridge/bridge.env.example assistants/bridge/bridge.env
$EDITOR assistants/bridge/bridge.env
```

| Var | What | Used by |
|-----|------|---------|
| `SLACK_APP_TOKEN` | `xapp-…` app-level token — opens the Socket Mode WS | inbound listener |
| `SLACK_BOT_TOKEN` | `xoxb-…` bot token — posts replies | transport (outbound) |
| `OCEAN_DAEMON_URL` | the Ocean daemon, default `http://127.0.0.1:4780` | dispatch |
| `OCEAN_ASSISTANTS_DIR` | absolute path to **this repo's `assistants/` dir** — the profile root the daemon reads from | daemon (profile load) |

> **Don't skip `OCEAN_ASSISTANTS_DIR`.** If it's unset, the daemon falls back to
> `~/.config/ocean-rs/assistants` and never loads this repo's authored Slack
> profile (`assistants/SLACK/system.md`) — the bot then silently runs the generic
> compiled-in seed prompt instead of the content-agent persona. Point it at this
> repo's `assistants/` directory (or symlink that dir into the config path). See
> [`docs/DAEMON_INTERACTION.md`](../../docs/DAEMON_INTERACTION.md) §"Where the
> daemon reads from".

Optional tuning (env, all have sane defaults):

| Var | Default | Effect |
|-----|---------|--------|
| `OCEAN_DAEMON_TIMEOUT` | `300` | per-turn daemon timeout (s) — reused from the router |
| `OCEAN_DAEMON_MAX_TRIES` | `3` | daemon transient-retry count |
| `OCEAN_LOG_LEVEL` | `INFO` | listener log verbosity |
| `OCEAN_SLACK_BOT_TOKEN` | — | takes precedence over `SLACK_BOT_TOKEN` for the transport |

> The transport's content-lab API creds live in
> `assistants/content-agent/content-agent.env` (separate template) — the bridge
> itself doesn't need them; the daemon-side agent does.

## 3. Install the inbound dependency

The repo is otherwise pure-stdlib; the Socket Mode handshake needs `slack_sdk`:

```bash
pip install -r assistants/bridge/requirements.txt
```

## 4. Run

```bash
# load env, then start the listener against the content-agent manifest
set -a; source assistants/bridge/bridge.env; set +a
python3 assistants/bridge/socket_listener.py run assistants/content-agent/content-agent.toml
```

The listener connects, logs `Socket Mode connected`, and waits. Mention the bot in
a channel or DM it → it dispatches a turn to the daemon and replies in-thread. It
**never auto-sends on boot** (spec §9) — it only acts on inbound events.

- **Reconnect:** a dropped WebSocket reconnects with exponential backoff (1s → 30s).
- **Dedupe:** Slack re-delivers un-acked events; repeats are dropped by event id so
  a turn never double-fires.
- **Fail-soft:** a bad single message is logged and skipped; a daemon outage posts
  a visible in-thread warning instead of going silent. The loop never crashes on
  one message.

Stop with `Ctrl-C`.

## 5. Inspect without connecting

No token needed — show the turn-scoping contract for the manifest:

```bash
python3 assistants/bridge/socket_listener.py resolve assistants/content-agent/content-agent.toml
```

## 6. Tests

The whole message-handling pipeline is unit-tested with a **mocked** Slack client
and a **mocked** daemon (a live connection needs the `xapp-` token above):

```bash
python3 assistants/bridge/tests/test_socket_pipeline.py
# or: python3 -m unittest discover -s assistants/bridge/tests -v
```

## 7. Slash-command intake (courier hub) — ACTIVATION

The listener also handles `slash_commands` envelopes (courier-hub intake,
`slash_intake.py` — see [`README.md`](README.md) for how it works). It is
**dormant until the command is registered** with the existing Slack app: Socket
Mode is already on, so there is no Request URL, no signing secret, and no new
tokens — registration is the only Slack-side step.

**Step 1 — add the command to the app manifest.**
[api.slack.com/apps](https://api.slack.com/apps) → *(the content-agent app)* →
**App Manifest** → add under `features:` (alongside the existing `bot_user`):

```yaml
features:
  slash_commands:
    - command: /ocean
      description: Run a Rising Tides courier (help lists all commands)
      usage_hint: "help | <courier> [args] — e.g. distro health"
      should_escape: false
```

(Equivalent UI path: **Slash Commands** → *Create New Command* → command
`/ocean`, description and usage hint as above. Leave the Request URL blank —
Socket Mode delivery.)

**Step 2 — reinstall the app to the workspace** when Slack prompts (adding the
first slash command adds the `commands` scope).

**Step 3 — restart the live listener process** so it picks up the new code
(same start command as §4). Merging this repo change alone does nothing to a
running listener; the restart is a deliberate, manual step.

**Step 4 — smoke it:** type `/ocean help` anywhere in the workspace → the live
courier command table (same rows as `python3 couriers/hub/router.py list`)
comes back as an ephemeral message. Then try a read-only command, e.g.
`/ocean sounds-preview` or `/ocean distro health`.

Behavior notes:

- **Deterministic** couriers run their harness on the bridge host; stdout+stderr
  come back in-channel in a code block, truncated at ~3,500 chars.
- **Agentic** couriers dispatch to the Ocean daemon (same payload the hub router
  sends); the reply is a short "dispatched to daemon — session `<id>`" note. The
  courier delivers its real results itself, over transport.
- Errors, usage mistakes, and `/ocean help` reply **ephemerally** (only the
  invoker sees them).
- Optional env (defaults are fine): `OCEAN_SLASH_COMMAND=/ocean`,
  `OCEAN_SLASH_REPLY_LIMIT=3500`, `OCEAN_SLASH_EXEC_TIMEOUT=600`.
