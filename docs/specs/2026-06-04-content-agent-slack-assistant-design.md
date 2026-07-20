# Design — `content-agent`: a conversational Slack assistant on Ocean

> **Date:** 2026-06-04
> **Repo:** `Risingtides-dev/risingtides-agents`
> **Status:** Design — awaiting review before implementation plan
> **Companion doc:** the cross-repo *Surface Awareness + Session-Bleed* addendum
> (runtime contract owned by the `ocean-os` agent). This design is the **consumer**
> side of that contract.

---

## 1. Summary

Build `content-agent`: a **conversational** Slack assistant that lives in the
Rising Tides Slack workspace, talks back in threads and DMs, renders generated
content into Slack canvases, and drives the content-lab video pipeline — all on
Ocean rails (provider-neutral, daemon-as-brain, no bypassing the runtime).

It is the **first member of a new agent class** in this repo, `assistants/`
(sibling to the existing `couriers/`). Couriers ship payloads *outbound*;
assistants are *brain-in-the-loop* — they listen, reason, and reply. The class is
designed from day one to hold **many** agents across **many** surfaces
(`SLACK`, `BRWSR`, `GUI`, `CNVS`, `WEB`, `MOBL`, `TUI`, `VOX`, `CLI`, `ACP`).

This design treats **Slack as an Ocean surface we adhere to**, not a channel we
control: the agent lives *in* the surface and respects its affordances (threads,
DMs, canvases, files), rather than reaching into it the way a courier does.

## 2. Goals / Non-goals

**Goals (v1)**
- A working conversational loop: mention or DM `content-agent` in Slack → the
  Ocean daemon reasons with the right surface profile → reply posts back in-thread.
- Replies in **threads and DMs**.
- Render content into **Slack canvases** (galleries, status, generated video refs).
- Drive the content pipeline: **generate video**, **chat / Q&A** grounded in
  Rising Tides context, **gallery / status lookups**.
- Fully kitted with permissions/tools for the above.
- Establish the **`assistants/` org tree** so future agents/surfaces slot in
  cleanly, with no per-agent duplication of house SOPs.

**Non-goals (v1)**
- Building the other surfaces (`VOX`, `GUI`, …) — only their *directory slots* and
  the `_base/` pattern exist; profile content is Slack-first.
- The runtime changes (persist `client_type`, `[SLACK]` flag, file-loaded surface
  injection) — those are the `ocean-os` agent's lane per the addendum. We build
  **against** that contract.
- Replacing the content-posting-lab pipeline — we **call** it.
- A content-pipeline MCP — deferred (see §6); v1 uses the existing HTTP API + raw
  tools.

## 3. Architecture

```
Slack workspace
   │  (mention / DM)
   ▼
Slack inbound bridge  ── couriers/transport already handles OUTBOUND.
(NEW: Socket Mode listener, app-level xapp- token)
   │  receives event, resolves thread/DM context
   │  POST /v1/prompt  { prompt, session_id, client_type: "surface-slack", cwd: <agent dir> }
   ▼
Ocean daemon (:4780)  ── the BRAIN. Owns the agent loop, tools, permissions.
   │  build_system_prompt(cwd, "surface-slack")
   │    → composes:  _base/SLACK/  +  content-agent/_shared/  +  content-agent/SLACK/
   │    (via load_project_prompt ancestor-walk over the scoped cwd)
   │  agent reasons, calls tools (content-lab API, bash/fetch, canvas ops)
   ▼
Reply path  ── bridge posts assistant output back via couriers/transport/slack.py
              (message in-thread / DM; canvas create/update for rich content)
```

**Key seam:** the bridge sends `client_type: "surface-slack"` on every turn. The
daemon maps that to the `[SLACK]` flag + the SLACK surface profile (runtime side,
per addendum R1/R3). We own everything left of the daemon and the reply path; the
daemon owns the brain.

**Reuse:** outbound Slack (post message, file upload, canvas create, rate-limit/
retry, link parse) already exists in `couriers/transport/slack.py`. The bridge and
the assistant reply path import it — they do **not** re-implement Slack I/O. The
**new** plumbing is the *inbound* Socket Mode listener; nothing in the repo
receives Slack events today.

## 4. The `assistants/` org tree (base + override)

```
assistants/
├── README.md                  # the class: brain-in-the-loop vs couriers; the surface model
├── _base/                     # HOUSE profiles — shared, surface-keyed, no agent identity
│   ├── SLACK/
│   │   ├── system.md          # base Slack surface role
│   │   ├── comms.md           # thread/DM etiquette, brevity, when to canvas
│   │   ├── canvas.md          # SOPs for rendering into Slack canvases
│   │   └── limits.md          # rate limits, don't-do-on-Slack rules
│   ├── VOX/  GUI/  BRWSR/  WEB/  CNVS/  MOBL/  TUI/  CLI/   # slots (stubs in v1)
│   └── _shared.md             # cross-surface house rules (optional)
├── content-agent/
│   ├── CLAUDE.md              # entry identity (auto-loaded by load_project_prompt)
│   ├── _shared/               # this agent across ALL surfaces (who it is, what it knows)
│   │   └── identity.md
│   ├── SLACK/                 # ONLY content-agent's Slack specifics / overrides of _base/SLACK
│   │   ├── system.md
│   │   ├── tools.toml         # tools/MCPs/API access content-agent has on Slack
│   │   └── sops/              # pipeline SOPs: generate, gallery, status
│   ├── content-agent.toml     # agent manifest (mirrors courier.toml pattern)
│   └── content-agent.env.example
└── bridge/                    # the NEW Socket Mode inbound listener (shared by Slack assistants)
    ├── socket_listener.py     # opens xapp- WebSocket, dispatches events → daemon
    ├── reply.py               # daemon output → couriers/transport/slack.py (thread/DM/canvas)
    └── bridge.env.example
```

**Composition rule.** At turn time the daemon scopes `cwd` to the agent's dir.
`load_project_prompt` ancestor-walks and reads `CLAUDE.md`/`AGENTS.md`. The surface
profile (`_base/SLACK/` + `content-agent/SLACK/`) is composed in via the runtime's
file-loaded surface injection (addendum R2). **Result:** house Slack SOPs live once
in `_base/SLACK/`; `content-agent/SLACK/` writes only what's specific or overrides.
Adding `other-agent` later means an `other-agent/SLACK/` with just its deltas — no
duplication.

> **Open item — injection mechanism (addendum open question).** Two ways the
> agent's dir gets the surface profile composed in: (a) **symlink** `_base/<SURFACE>`
> into the agent dir so `load_project_prompt`'s ancestor-walk picks it up in one
> pass, or (b) the runtime **path-resolves** `assistants/_base/<surface_dir>` by
> the `client_type→dir` map and concatenates. (b) is cleaner (no symlink sprawl,
> works cross-platform) and keeps the base read-only; **recommended (b)**, pending
> the `ocean-os` agent's confirmation since they own the injection code.

## 5. Namespacing (many agents, one surface)

- **One Slack app per agent identity** is the cleanest isolation: `content-agent`
  has its own bot user, its own `xapp-`/`xoxb-` tokens, its own `bridge.env`. A
  second Slack assistant gets its own app + tokens. This makes the
  session→surface→agent binding unambiguous at the Slack layer (different bot user
  IDs) and means one agent's perms/tools can never bleed into another's.
- **Session id convention:** bind a Slack session to `(agent, channel/DM, thread)`
  so threads stay isolated and resumable. Proposed key:
  `slack:<agent>:<channel_id>:<thread_ts|dm>`.
- **`cwd` per turn = the agent's dir** (`assistants/content-agent/`), so the daemon
  loads exactly that agent's identity + its surface overrides + the house base.

## 6. Pipeline access (1 + 2)

**Primary: the content-posting-lab HTTP API.** `content-agent` calls the existing,
deployed FastAPI backend (`/api/video/*`, gallery, status, etc.) for generate /
gallery / status. Reuses proven, deployed logic; thinnest path to a working v1.

**Escape hatch: built-in `bash`/`fetch` tools.** For anything the API doesn't
cover, and to serve the **self-learning / hot-add** requirement, `content-agent`
can drive Replicate / R2 / the gallery worker directly via its `tools.toml`. New
"skills" are SOPs over these tools — addable without a redeploy.

**Deferred: a content-pipeline MCP.** Once the tool surface stabilizes, wrap
generate/poll/review/upload/gallery-status as typed MCP tools for a clean,
reusable-across-agents contract. Not in v1.

This ordering means v1 ships fast on the API, the agent stays open-ended via raw
tools, and the MCP is a later hardening step — not a blocker.

## 7. Self-learning / hot-reconfigure

The whole point of profiles-as-data: an operator can grant `content-agent` a new
skill / MCP / API / CLI by **editing files** (`content-agent/SLACK/tools.toml`,
`sops/`), then **resetting** the agent so the daemon re-injects the new profile on
the next turn. No Rust rebuild, no redeploy of the runtime. This depends on the
`ocean-os` file-loaded surface injection (addendum R2) and on `tools.toml` being
read per-turn/per-session by the daemon.

**Reset semantics (v1):** a reset = start a fresh `session_id` for that
agent+thread so the next turn rebuilds the system prompt from the (now-edited)
profile. (A live in-session reconfigure is a later enhancement.)

## 8. v1 capabilities (kitted)

| Capability | Path | Notes |
|---|---|---|
| Generate video | content-lab API (`/api/video/*`) | prompt in Slack → kick off → report status + drop result in-thread |
| Chat / Q&A | daemon brain + `content-agent/_shared/identity.md` | grounded in Rising Tides context |
| Gallery / status lookups | content-lab API | surface gallery links; report queue/pipeline status |
| Canvas rendering | `couriers/transport/slack.py` canvas ops + `_base/SLACK/canvas.md` SOP | embed galleries / generated refs as a canvas, not just links |
| Files / links | transport file upload + message | drop generated media + links in-thread |

## 9. Error handling & safety

- **Inbound resilience:** Socket Mode reconnect/backoff; dedupe Slack event
  retries (Slack re-delivers on no-ack) by event id so a turn never double-fires.
- **Outbound:** reuse transport's existing rate-limit + transient retry.
- **Permissions:** the daemon is permission-gated; v1 runs `content-agent` with the
  tool set its `tools.toml` declares. Destructive ops (delete/repost) are excluded
  by SOP — mirror the content-lab "append-only is safer" rule.
- **No startup auto-sends:** the bridge only acts on inbound events (mirrors the
  content-lab "don't auto-trigger sends on boot" rule).
- **Secrets:** `xapp-`/`xoxb-` tokens + content-lab API creds via env /
  `bridge.env` / `content-agent.env`, never committed (gitignored, `.env.example`
  templates only).

## 10. Testing

- **Transport (offline):** link-parse / profile-compose unit tests, no token
  needed (mirrors the courier self-test pattern).
- **Bridge:** event-dedup + reconnect logic unit-tested with a fake Socket Mode
  stream; daemon call mocked.
- **Profile composition:** assert `_base/SLACK` + `content-agent/SLACK` compose in
  the expected order and overrides win.
- **End-to-end (manual, gated):** real workspace, mention the bot in a thread →
  confirm in-thread reply; DM it → confirm DM reply; ask for a gallery → confirm a
  canvas renders; trigger a generate → confirm status + result land in-thread.

## 11. Build order (per your sequence)

1. **This design** (here) → review/approve.
2. **Scaffold `assistants/`** — the tree in §4, `_base/SLACK/` house SOPs,
   `content-agent/` identity + `SLACK/` overrides + manifest + `.env.example`.
   Structure + content, no live wiring.
3. **Slack inbound bridge** — `bridge/socket_listener.py` (Socket Mode, dedupe,
   reconnect) + `reply.py` (daemon output → transport: thread/DM/canvas). Prove the
   loop end-to-end against the daemon.
4. **content-agent profile** — flesh out identity, pipeline SOPs, `tools.toml`;
   wire the content-lab API calls; canvas rendering SOPs. Pour the brain in last.

## 12. Dependencies / assumptions

- **Runtime contract (addendum):** `client_type` persisted on Session,
  `surface-slack`→`[SLACK]` flag, and file-loaded surface injection are delivered
  by the `ocean-os` agent. We can scaffold and build the bridge against the
  *current* daemon (which already accepts `client_type` and rebuilds the prompt per
  turn); full surface-profile injection lands when their Fix 5 does. **Stub
  gracefully** if the file-load path isn't live yet (the agent's `CLAUDE.md` still
  loads via `load_project_prompt` today).
- Ocean daemon reachable at `$OCEAN_DAEMON_URL` (default `http://127.0.0.1:4780`).
- content-posting-lab API reachable + credentialed.
- A Slack app for `content-agent` with Socket Mode enabled, `xapp-` app token, and
  bot scopes (`chat:write`, `files:write`, channel read, `canvases:write`,
  plus `app_mentions:read` / `im:history` / `im:read` for inbound).

## 13. Open questions (for review)

1. **Injection mechanism** — symlink vs path-resolve for composing `_base/<SURFACE>`
   into the agent (§4). Recommend path-resolve; needs `ocean-os` confirm.
2. **One Slack app per agent** vs one shared app routing to many agents (§5).
   Recommend per-agent app for clean isolation; confirm ops is OK managing N apps.
3. **Reset semantics** — v1 = new session id (§7). Good enough, or do we need
   live in-session reconfigure in v1?
4. **content-agent vs the `content-agent` pipeline repo** — same name, different
   thing. Keep the assistant named `content-agent` here, or rename to avoid
   confusion (e.g. `content-assistant`)?
