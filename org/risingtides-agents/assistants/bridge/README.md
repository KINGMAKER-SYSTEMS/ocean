# Slack bridge — `OCEAN_ASSISTANTS_DIR` + `reply.py` reference

This directory is the **Slack bridge**: the plumbing that lets an Ocean assistant
operate on the **Slack surface**. Slack has no native Ocean client, so the bridge
is the surface — it receives Slack events, hands each one to the Ocean daemon as a
turn, and posts the daemon's answer back into the thread/DM.

`RUN.md` covers **how to run it** (Slack app setup, env file, start command). This
README covers **how it works** — specifically the two things that bite operators:

1. **`OCEAN_ASSISTANTS_DIR`** — the env var that decides *which* surface profile the
   daemon loads for a Slack turn. Get it wrong and the bot silently runs a generic
   prompt instead of `content-agent`.
2. **`reply.py`** — the outbound half: how the daemon's answer gets back to Slack.

> Verified against the code in this repo (`socket_listener.py`, `reply.py`,
> `couriers/transport/slack.py`, `couriers/hub/router.py`,
> `content-agent/content-agent.toml`) and the daemon contract in
> [`docs/DAEMON_INTERACTION.md`](../../docs/DAEMON_INTERACTION.md). If they drift,
> the code is authoritative.

---

## The end-to-end flow

```
Slack workspace
   │  app_mention / message.im  (over a Socket Mode WebSocket)
   ▼
socket_listener.py  (run())                         ── THIS repo, inbound front door
   │  ack envelope → dedupe → resolve_context → build_turn
   │  build_turn reads content-agent.toml:
   │     client_type = "surface-slack"   (→ [SLACK] profile, see below)
   │     cwd         = "assistants/content-agent"  (→ identity ancestor-walk)
   ▼
dispatch_to_daemon()  → POST /v1/prompt              ── reuses couriers/hub/router.py
   │     { cwd, prompt, client_type, session_id, create_if_missing }
   ▼
Ocean daemon  (127.0.0.1:4780)                       ── the brain; sessions live here
   │  build_system_prompt:
   │    BASE + load_project_prompt(cwd) + append_client_type(client_type)
   │                                          │
   │    append_client_type → surface_dir("surface-slack") = "SLACK"
   │                       → loads <OCEAN_ASSISTANTS_DIR>/SLACK/system.md   ◄── KEY SEAM
   │  runs the turn, returns assistant text in the PromptResponse
   ▼
socket_listener.handle_event()  → reply.deliver()    ── THIS repo, outbound
   │
   ▼
reply.py  → couriers/transport/slack.py              ── real Slack post / canvas
   │     post_message(channel, text, thread_ts)   (in-thread reply or DM)
   │     create_canvas(...)                        (large/structured output)
   ▼
Slack workspace  (answer lands in the same thread / DM)
```

One pipeline, fail-soft at every step: a duplicate event is dropped, a bad single
message is logged and skipped, and a daemon outage posts a visible `:warning:`
in-thread instead of going silent. The loop never crashes on one message.

---

## The OTHER consumption path: `slack_canvas` ops over SSE (`canvas_consumer.py`)

The inbound flow above is **request/reply**: the bridge POSTs `/v1/prompt` and
reads the assistant text out of the response body. That covers chat. It does
**not** cover the agent's `slack_canvas` tool (ocean-os runtime, OCEAN-214),
whose canvas ops (`create`/`read`/`update`/`append`/`list`) are emitted as
**async side-effects** onto the daemon's `/v1/agent/events` **SSE** stream
(`AgentTurnEvent::SlackCanvas`), not returned in a `/v1/prompt` body. A POST
reply can't see them — they need a *subscriber*.

`canvas_consumer.py` is that subscriber (OCEAN-244). It is the bridge's first SSE
consumer and runs as its **own process**, parallel to `socket_listener.py`:

```
Ocean daemon  /v1/agent/events  (SSE, ?all=1)
   │  AgentTurnEvent::SlackCanvas { session_id, op }
   ▼
canvas_consumer.py  (run())                          ── THIS repo, SSE subscriber
   │  parse_canvas_event → apply_op → deliver_fulfillment
   ▼
couriers/transport/slack.py                          ── real Slack Canvas API
   │  create → canvases.create            (mints canvas_id)
   │  update/append → canvases.edit       (replace / insert_at_end / insert_at_start)
   │  read  → files.info + canvases.sections.lookup   (structure, see caveat)
   │  list  → files.list types=spaces                 (channel's canvas files)
   ▼
POST /v1/agent/canvas/fulfill  { session_id, op, result }   ── fulfillment return
   (resolves the agent's pending_bridge read/list to real content)
```

The op→API mapping and result-building are pure (token-free) and unit-tested with
a mocked transport (`tests/test_canvas_consumer.py`); only `run()` opens the live
SSE connection. Run it with `python3 assistants/bridge/canvas_consumer.py run`
(needs a running daemon + the same `xoxb-` bot token the transport resolves), or
`… describe` for a no-I/O JSON self-description of the contract.

**Cross-repo status** (verified against ocean-os `main`, 2026-06; OCEAN-375):
both halves of the seam are now **live** on ocean-os `main`.

1. **The daemon relays `slack_canvas` on the SSE wire.** The OCEAN-214 runtime's
   canvas ops reach this consumer via a dedicated `AgentTurnEvent::SlackCanvas`
   relay arm (landed via the OCEAN-235/#165 seam).
2. **The fulfillment-return endpoint is mounted** at `/v1/agent/canvas/fulfill`
   (overridable via `OCEAN_CANVAS_FULFILL_PATH`), so `deliver_fulfillment()`
   posts the bridged read/list result back and the agent's `pending_bridge`
   placeholder resolves to real content. It still **fail-softs on 404** or an
   unreachable daemon (logs, never crashes), so the consumer degrades gracefully
   against an older daemon build.

**Slack read caveat (not a bug, an API limit):** Slack exposes **no** endpoint
that returns a canvas's raw markdown body. `read_canvas` returns what the public
API *does* give — the section skeleton (`canvases.sections.lookup`) + file
metadata (`files.info`) — plus an explicit note, rather than fabricating a body.
`create`/`update`/`append` are fully round-trippable.

---

## Slash-command intake — `/ocean` → courier hub (`slash_intake.py`)

The bridge's Socket Mode WebSocket carries more than events: Slack also delivers
**slash commands** over it, as `slash_commands` envelopes. `slash_intake.py`
rides that — it is the courier hub's **Intake layer**
(`couriers/ARCHITECTURE.md`), built with **no public ingress and no signing
secret**, because the socket the content-agent already holds is the transport.

```
Slack: /ocean distro health        (or /ocean help, or a direct /distro …)
   │  slash_commands envelope  (same Socket Mode WS as events)
   ▼
socket_listener._on_request                          ── ack ≤3s (ephemeral "running…" for dispatches)
   │  worker thread (a slow harness never blocks the WS processor)
   ▼
slash_intake.handle_slash                            ── parse → route via the hub registry
   │  REUSES couriers/hub/router.py: load_registry, session fold, call_daemon
   ├─ help/usage      → ephemeral reply (live command table = `router.py list`)
   ├─ deterministic   → subprocess the harness (router's argv shape), capture
   │                    stdout+stderr → in-channel code block (truncated ~3,500)
   └─ agentic         → router.call_daemon (router's exact payload shape)
                        → short "dispatched to daemon — session `<id>`" note
   ▼
payload's response_url  (stdlib urllib POST, no token, 30-min validity;
                         fallback: plain reply via router.deliver_reply)
```

**One registered command fronts the whole hub:** `/ocean <courier-slash> [args]`
maps onto the router's `/<slash>` table, so new couriers appear in `/ocean help`
with zero intake changes. Direct per-slash payloads also route (if a payload's
`command` matches a courier slash), so per-command Slack registration can be
added later without code changes.

**Dormant by default.** Nothing here runs until the `/ocean` command is added to
the Slack app's manifest — see [`RUN.md`](RUN.md) §7 for the activation steps.
No new env vars, tokens, or dependencies; the intake reuses the bridge's
existing connection and the router's existing daemon path. Optional tuning:
`OCEAN_SLASH_COMMAND` (default `/ocean`), `OCEAN_SLASH_REPLY_LIMIT` (default
`3500`), `OCEAN_SLASH_EXEC_TIMEOUT` (default `600`s).

Unit tests: `tests/test_slash_intake.py` (mocked runner/daemon/responder — no
network, no Slack).

---

## `OCEAN_ASSISTANTS_DIR` — which profile the daemon loads

This is the single most important env var on the Slack path, and the easiest to
miss because **nothing errors when it's wrong** — the bot just stops sounding like
`content-agent`.

### What it points at

Set it to the **absolute path of this repo's `assistants/` directory**:

```bash
OCEAN_ASSISTANTS_DIR=/absolute/path/to/risingtides-agents/assistants
```

### Why it matters (the mechanism)

The daemon stamps every turn with a **surface flag** derived from the turn's
`client_type`, then prefers an on-disk `<root>/<SURFACE>/system.md` over its
compiled-in seed prompt. For Slack:

| Step | Value | Owned by |
|------|-------|----------|
| Bridge sends | `client_type = "surface-slack"` | `content-agent.toml` `[surface.SLACK]` |
| Daemon maps | `surface-slack → SLACK` | `ocean-agent::surface_dir` (ocean-os) |
| Daemon loads | `<root>/SLACK/system.md` | `load_surface_profile` |

…where `<root>` is resolved by `ocean-agent::assistants_root` in this order
(first hit wins):

1. **`$OCEAN_ASSISTANTS_DIR`**, if set and non-empty.
2. Otherwise `~/.config/ocean-rs/assistants` (`$XDG_CONFIG_HOME/ocean-rs/assistants`).

So if `OCEAN_ASSISTANTS_DIR` is **unset**, the daemon looks under
`~/.config/ocean-rs/assistants/SLACK/system.md`. Unless you've symlinked this repo
there, that file doesn't exist — the daemon falls back to its generic compiled-in
seed prompt, and **the content-agent persona never loads**. No error, no log on the
daemon side; the bot just answers like a generic assistant and seems to "ignore its
instructions."

### The bridge warns you (startup check)

`socket_listener.run()` does **not** set this var for the daemon — it can't, the
daemon is a separate process — but it **detects the footgun and warns** at startup:

- **Unset:** logs a warning telling you to set
  `OCEAN_ASSISTANTS_DIR=<this repo's assistants dir>`.
- **Set but pointing elsewhere:** logs a warning that the daemon won't load *this*
  repo's authored profiles, and prints the path you should use.

The warning is advisory (the bridge still starts), because the daemon might
legitimately read the profiles via a symlink. But on a normal local setup, seeing
that warning means **the bot is about to run the wrong prompt** — fix the env.

### Also note: the daemon must see the same value

Setting `OCEAN_ASSISTANTS_DIR` in `bridge.env` only puts it in the **bridge's**
environment. The profile lookup happens in the **daemon**, so the daemon process
must have the same value in *its* environment (or have this repo's `assistants/`
symlinked into `~/.config/ocean-rs/assistants`). If you run the daemon under
launchd/systemd, set it there too. The bridge's startup warning checks the
bridge's own env as a proxy — it's a good signal, not a guarantee the daemon agrees.

---

## `reply.py` — the daemon's answer → Slack

`reply.py` is the **outbound adapter**. It does not open any connection or own any
Slack credentials of its own; it translates one daemon turn's output into calls on
the existing outbound transport, `couriers/transport/slack.py` (which owns auth,
retries, rate limits, file upload, and canvas ops).

### How it's invoked

It is **not** a CLI you run. `socket_listener.handle_event()` calls
`reply.deliver()` after every completed daemon turn:

```python
deliver(reply_target, {"text": text})
```

- `reply_target` = `{channel, thread_ts, is_dm}` — built by `build_turn` from the
  inbound event (carried on the turn as `_reply`).
- `{"text": ...}` = the daemon's assistant text for the turn.
- No `token` argument is passed, so `deliver()` calls `Slack(token=None)` and the
  transport resolves the bot token itself (see "Tokens" below).

`deliver()` then routes:

| Condition | Route | Transport call |
|-----------|-------|----------------|
| short / plain output | **in-thread message** (or DM) | `post_message(channel, text, thread_ts)` |
| `render == "canvas"`, or text > 1500 chars / > 25 lines, or a `canvas` field | **canvas** + one-line in-thread pointer | `create_canvas(title, markdown, channel)` then `post_message(...)` |

The message/canvas split is `should_canvas()` — a **bridge-side fallback
heuristic**. content-agent's own SOPs (`content-agent/SLACK/sops/`) can override it
by setting `render`/`canvas` fields on the daemon output. Threading is preserved:
a threaded conversation replies in-thread (`thread_ts`); a DM posts to the DM
channel (`thread_ts = None`).

### Running it standalone

`python3 assistants/bridge/reply.py` does **not** send anything. With no args it
just prints a JSON self-description of its routes and what it reuses — handy for a
sanity check. The live send path is only ever reached through
`socket_listener.handle_event`. If the transport module can't be imported (e.g.
running the file in isolation), `deliver()` returns
`{"ok": false, "status": "transport-unavailable"}` and makes no Slack call.

---

## Environment variables

Copy `bridge.env.example` → `bridge.env` (gitignored) and fill it in.

### Required

| Var | Value | Used by | Notes |
|-----|-------|---------|-------|
| `SLACK_APP_TOKEN` | `xapp-…` app-level token | inbound listener | Opens the Socket Mode WebSocket. Needs the `connections:write` scope. |
| `SLACK_BOT_TOKEN` | `xoxb-…` bot token | outbound transport | Posts replies / canvases. Resolved by `couriers/transport/slack.py`. |
| `OCEAN_ASSISTANTS_DIR` | absolute path to this repo's `assistants/` dir | **daemon** (profile load) | See the section above — unset ⇒ generic seed prompt, silently. Must also be visible to the daemon process. |

### Optional (sane defaults)

| Var | Default | Effect |
|-----|---------|--------|
| `OCEAN_DAEMON_URL` | `http://127.0.0.1:4780` | Where the bridge POSTs turns. |
| `OCEAN_SLACK_BOT_TOKEN` | — | Takes **precedence** over `SLACK_BOT_TOKEN` for the transport's token resolution. |
| `OCEAN_SLACK_TOKEN_FILE` | `~/.slack_token` | Fallback bot-token file if neither env var is set. |
| `OCEAN_DAEMON_TIMEOUT` | `300` | Per-turn daemon timeout (s) — reused from the OCEAN-84 router. |
| `OCEAN_DAEMON_MAX_TRIES` | `3` | Daemon transient-retry count. |
| `OCEAN_CANVAS_FULFILL_PATH` | `/v1/agent/canvas/fulfill` | Where `canvas_consumer.py` POSTs bridged read/list results back to the daemon. |
| `OCEAN_LOG_LEVEL` | `INFO` | Listener log verbosity. |

> **Token resolution order (outbound, first hit wins):**
> `$OCEAN_SLACK_BOT_TOKEN` → `$SLACK_BOT_TOKEN` → `$OCEAN_SLACK_TOKEN_FILE`
> (default `~/.slack_token`). The bot token must be an `xoxb-…` token with at least
> `chat:write`, `files:write`, the channel-read scopes, and (optional)
> `canvases:write`.

> The content-lab API creds (`CONTENT_LAB_API_URL` / `CONTENT_LAB_API_KEY`) live in
> `assistants/content-agent/content-agent.env`, **not** here — the bridge doesn't
> call the content pipeline; the daemon-side agent does, via its granted tools
> (`content-agent/SLACK/tools.toml`).

---

## Tokens — who needs what

- **`xapp-` app token** is the bridge's alone: only `socket_listener.run()` uses it,
  to open the inbound WebSocket. It is never passed to the daemon or the transport.
- **`xoxb-` bot token** is the transport's: `reply.py` → `couriers/transport/slack.py`
  resolves it (env or file) to post replies. `reply.deliver()` is called **without**
  a token, so the transport's own resolution applies — there's no token plumbed
  through the reply call itself.
- The **daemon** holds no Slack tokens; it only produces turn text. All Slack I/O is
  in this repo (inbound in `socket_listener.py`, outbound in `reply.py` + transport).

---

## See also

- [`RUN.md`](RUN.md) — operational walkthrough: Slack app setup, env, start command,
  tests.
- [`bridge.env.example`](bridge.env.example) — the env template (with the
  `OCEAN_ASSISTANTS_DIR` footgun note inline).
- [`../../docs/DAEMON_INTERACTION.md`](../../docs/DAEMON_INTERACTION.md) — the daemon
  contract: the full `client_type → surface_dir` map, profile-load order, and the
  turn/event APIs.
- [`../README.md`](../README.md) — the assistants registry and the surface-profile
  composition model (`_base/<SURFACE>/` → composed `<SURFACE>/system.md`).
