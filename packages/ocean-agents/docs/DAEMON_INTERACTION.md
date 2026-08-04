# How ocean-agents talks to the Ocean OS daemon

This repo holds **provider-agnostic agent packages** (couriers + assistants) that
run *on top of* the [Ocean OS](https://github.com/Risingtides-dev/ocean-os)
runtime. None of them embed agent logic, provider credentials, or session
authority — those live in the daemon. This guide documents the three seams where
ocean-agents and the daemon meet:

1. **Surface-profile loading** — how the daemon picks up an assistant's
   `system.md` at runtime, preferring it over its compiled-in seed prompt.
2. **Submitting turns** — how an assistant/courier gets work onto the daemon
   (`POST /v1/agent/turns`, or the router's `POST /v1/prompt`).
3. **Consuming events** — how a client reads the result back over the
   session-scoped SSE stream (`GET /v1/agent/events?session_id=`).

The daemon listens on `127.0.0.1:4780` by default (`OCEAN_BIND` to override);
clients resolve it via `OCEAN_DAEMON_URL` (default `http://127.0.0.1:4780`).

> **Verified against code.** The route table, struct fields, and the
> `client_type → surface_dir` map below were read from `ocean-os` at
> `crates/ocean-daemon/src/main.rs`, `crates/ocean-core/src/lib.rs`, and
> `crates/ocean-agent/src/lib.rs`, and from this repo's
> `couriers/hub/router.py`. If they drift, the Rust source is authoritative.

---

## 1. Surface-profile loading

Ocean OS stamps every turn with a **surface flag** derived from the turn's
`client_type`, and — via `build_system_prompt → append_client_type →
load_surface_profile` — **prefers an on-disk `assistants/<DIR>/system.md` over
its compiled-in seed const** when that file is present and non-empty. This makes
a surface profile **editable data, hot-reconfigurable without a daemon rebuild**:
edit the `system.md`, and the next turn on that surface loads it.

### The `client_type → surface_dir` map

The map is owned by Ocean OS in `ocean-agent::surface_dir` (which mirrors
`surface_flag` — same labels, one source of truth). Each `client_type` resolves
to the directory name the daemon looks up under the assistants root:

| `client_type`                     | Surface flag / dir | Profile path                   |
|-----------------------------------|--------------------|--------------------------------|
| `surface-extension`               | `BRWSR`            | `assistants/BRWSR/system.md`   |
| `tui`                             | `TUI`              | `assistants/TUI/system.md`     |
| `surface-web`                     | `WEB`              | `assistants/WEB/system.md`     |
| `surface-gpui` / `surface-native` | `GUI`              | `assistants/GUI/system.md`     |
| `cli`                             | `CLI`              | `assistants/CLI/system.md`     |
| `leo-voice`                       | `VOX`              | `assistants/VOX/system.md`     |
| `acp-zed`                         | `ACP`              | `assistants/ACP/system.md`     |
| `surface-slack`                   | `SLACK`            | `assistants/SLACK/system.md`   |
| `surface-canvas`                  | `CNVS`             | `assistants/CNVS/system.md`    |
| `surface-mobile`                  | `MOBL`             | `assistants/MOBL/system.md`    |
| *(anything else)*                 | `?`                | *(no lookup; seed const used)* |

An unknown `client_type` resolves to `"?"`, for which the daemon skips the file
lookup entirely and falls back to its seed prompt.

### Where the daemon reads from

The assistants root is resolved in this order (`ocean-agent::assistants_root`):

1. `$OCEAN_ASSISTANTS_DIR`, if set and non-empty.
2. Otherwise `assistants/` under the Ocean config dir
   (`$XDG_CONFIG_HOME/ocean-rs/assistants` or `~/.config/ocean-rs/assistants`).

To wire **this repo's** profiles into a running daemon, point
`OCEAN_ASSISTANTS_DIR` at this repo's `assistants/` directory (or symlink it into
the config dir). The lookup path is then `<root>/<DIR>/system.md`.

### Fallback behavior

The compiled-in seed consts in `ocean-agent` stay as **seed + graceful
fallback**. The file wins when present; when a surface has no authored profile
here yet, the daemon uses the seed. So adding a new authored surface = drop a
`assistants/<DIR>/system.md` — no daemon rebuild, no router edits. See
[`assistants/README.md`](../assistants/README.md) for the registry and the
phased delivery roadmap (R3: `bonzai` + `SLACK`; R4: `CNVS`/`MOBL`/`VOX`/`BRWSR`;
R5: `ACP`/`TUI`/`WEB`/`GUI`/`CLI` graduate from seeds to files).

---

## 2. Submitting a turn

There are two entry points, both `POST`s to the daemon. An assistant's
deterministic harness (e.g. `bonzai`) does **not** call these itself — it runs
*inside* a daemon turn, driven by the agent loop via the `bash` tool. These
endpoints are how a **client surface** or the **courier router** kicks a turn
off.

### `POST /v1/agent/turns` — the product agent API

The canonical product-agent entry point. First-party surfaces create or choose a
session first, then post turns scoped to that `session_id`. Body
(`AgentTurnRequest`):

| Field            | Type       | Notes                                                               |
|------------------|------------|---------------------------------------------------------------------|
| `prompt`         | `string`   | The turn prompt.                                                    |
| `cwd`            | `string`   | Working directory for the turn.                                     |
| `session_id`     | `string?`  | The session this turn belongs to. Omitted ⇒ the daemon mints one.   |
| `project_id`     | `string?`  | When set, the daemon binds the turn to the project's `workspace_root`. |
| `client_type`    | `string?`  | The originating surface — drives the surface flag and profile lookup in §1. |
| `guidance`       | `string[]?`| Optional guidance hints (e.g. active-tab context).                  |
| `room_id`        | `string?`  | Optional Track-0 room-scoped turn id.                               |
| `thinking_level` | `string?`  | Per-turn reasoning-effort override.                                 |
| `model_id`       | `string?`  | Per-session / per-turn model override.                              |

The POST returns metadata only (`turn_id`, `session_id`, `status`); the reply
text, tool calls, and ids arrive over the SSE stream (§3).

> **Set `client_type`.** It is what makes §1 work — without it the turn resolves
> to `"?"` and the daemon uses a generic seed prompt instead of the surface's
> authored profile.

### `POST /v1/prompt` — the lower-level prompt API

The courier router (`couriers/hub/router.py`) uses this for **agentic** couriers:
it POSTs a prompt scoped to the courier's directory and lets the daemon run it.
The router builds its payload as `{ "cwd": <courier dir>, "text": <prompt> }` and
sends it to `$OCEAN_DAEMON_URL/v1/prompt`.

The daemon's `PromptRequest` carries `prompt`, `cwd`, optional `session_id`,
`project_id`, `client_type`, plus run-policy flags (`create_if_missing`,
`max_turns`, `yolo`). Note two things when using this path:

- The canonical field is **`prompt`**, not `text`; the daemon does not declare a
  serde alias for `text`. Align the payload to `prompt` (and supply
  `client_type`) if you want surface-aware behavior on this route.
- `/v1/prompt` is the older steering API; new conversational surfaces should
  prefer `/v1/agent/turns` with an explicit session.

---

## 3. Consuming events

A client subscribes to the **session-scoped** SSE stream to read a turn's output:

```
GET /v1/agent/events?session_id=<id>
```

This carries the `AgentTurnEvent` stream for that one session — assistant text,
tool calls, permission requests, and turn completion. Scoping is mandatory:

- Subscribe with your own `session_id` and consume only that session's events.
- **Do not** adopt active sessions from the global SSE stream. Two surfaces
  sharing a session is explicit (both attach to the same `session_id`);
  different sessions must never blend. This is the Ocean ecosystem session
  contract — the daemon, not the client, owns session authority.

---

## End-to-end shape

```
                  surface profile (this repo)
            assistants/<DIR>/system.md  ──prefer──┐
                                                  ▼
client_type ──► surface_dir ──► load_surface_profile ──► system prompt
     │
     ▼
POST /v1/agent/turns { prompt, cwd, session_id, client_type, ... }
     │  (or router → POST /v1/prompt { cwd, prompt, client_type })
     ▼
   daemon runs the turn on the Ocean runtime
     │
     ▼
GET /v1/agent/events?session_id=<id>  ──► AgentTurnEvent stream (scoped)
```

## See also

- [`assistants/README.md`](../assistants/README.md) — the assistant registry
  (`assistants/<DIR>/system.md` + `_shared/`) and the phased roadmap.
- [`AGENTS.md`](../AGENTS.md) — the repo agent index (couriers + assistants).
- ocean-os `docs/OCEAN_ECOSYSTEM_CONTRACT.md` — the canonical session contract.
