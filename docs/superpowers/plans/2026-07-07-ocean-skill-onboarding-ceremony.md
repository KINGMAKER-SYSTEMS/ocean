# Ocean Skill Onboarding Ceremony Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standardized skill-driven onboarding ceremony where any skill-capable coding agent can open the Ocean local site, drive hands-on setup through localhost calls, stream real discovery/import events into a Leptos/WASM stage, and complete a safe one-time import of Claude Code, Codex, and Pi configs into Ocean.

**Architecture:** The onboarding intelligence lives in a portable skill, not inside the web page and not inside a Claude-specific integration. A skill-capable agent runs the standardized skill, scans the local machine, calls the Ocean local app over `127.0.0.1`, and streams ceremony events. The Ocean local app accepts the event stream, fans it out over WebSocket, preserves a replayable run buffer, and continues to own existing invite/folder/preview/ack/sync APIs. A Leptos 0.8 CSR/WASM UI renders the ceremony as an interactive oceanic stage using high-level page-tool events rather than raw DOM control.

**Tech Stack:** Node HTTP local app (`scripts/ocean-local-app.mjs`), `ws` WebSocket dependency, Leptos 0.8 CSR with Trunk (`ui/onboarding-ceremony`), JSON event protocol, standardized skill markdown at `skills/ocean-bedrock-coworker-onboarding/SKILL.md`, existing Ocean file/sync APIs.

---

## Locked Product Decisions

1. **Standardized skill is the conductor.** The runner may be Claude Code, Codex, Pi, OMP, or any future agent that can read skills and make localhost HTTP calls. Avoid Claude-specific assumptions in protocol names, event names, docs, and UI copy.
2. **The local site is the stage.** The browser UI is a theatrical surface and safety checkpoint, not an embedded LLM and not the authority for scanning. The runner performs scans and calls APIs.
3. **No Claude OAuth.** The flow does not use Claude subscription OAuth tokens, `claude setup-token`, or `sk-ant-oat01-*` credentials. Importing Claude Code configuration means reading local files the runner is already authorized to read.
4. **One-time import for V1.** Claude Code, Codex, and Pi configs/projects/sessions are imported once per ceremony run. Live watchers are explicitly out of scope.
5. **Human ack stays human.** The runner may open/highlight the preview acknowledgement step, but it must not click `I reviewed this preview — unlock sync` for the coworker. The skill pauses until the user clicks the gate.
6. **Real names, safe contents.** The ceremony displays real project folder names, skill names, agent names, MCP server names, and doc filenames. Raw secrets, auth tokens, keychain items, `.credentials.json`, `auth.json`, and API keys are never imported or rendered.
7. **Pi means config import.** Treat Pi as another local agent configuration source (`~/.pi` or user-specified Pi config path). Do not introduce Pi Cockpit/live hub assumptions.

---

## Ceremony Event Schema

All runner-to-site ceremony events use this envelope.

```json
{
  "schema": "ocean.ceremony.event.v1",
  "runId": "cer_20260707_abc123",
  "seq": 42,
  "ts": "2026-07-07T21:14:03.128Z",
  "runner": {
    "kind": "skill-runner",
    "runtime": "claude-code | codex | pi | omp | other",
    "name": "optional human-readable runner name"
  },
  "phase": "boot | scan | review | import | connect | preview | ack | sync | complete | error",
  "type": "agent.detected",
  "status": "started | progress | complete | blocked | error | info",
  "message": "Claude Code setup found",
  "visibility": "public | private | redacted",
  "payload": {}
}
```

### Envelope Rules

- `schema` is required and must equal `ocean.ceremony.event.v1` for V1.
- `runId` is required. A runner creates it by calling `POST /api/ceremony/runs` before emitting events.
- `seq` is assigned by the local app. Runner-sent `seq` is ignored.
- `ts` is assigned by the local app if missing. Runner timestamps are accepted only as `payload.runnerTs`.
- `runner.runtime` must be one of: `claude-code`, `codex`, `pi`, `omp`, `other`.
- `visibility=private` events are kept in the local run buffer but not rendered in the theatrical stream.
- `visibility=redacted` events render labels/counts only and never render payload fields marked sensitive.
- Events are append-only. Corrections are represented by later events.

### Core Event Types

```txt
ceremony.run_started
ceremony.scene_set
ceremony.loading_set
ceremony.highlight
ceremony.toast
ceremony.await_user
ceremony.run_completed
ceremony.error

agent.source_detected
agent.config_detected
agent.project_detected
agent.session_detected
agent.skill_detected
agent.mcp_detected
agent.doc_detected
agent.secret_skipped

import.plan_started
import.item_queued
import.item_imported
import.item_skipped
import.plan_completed

ocean.invite_redeemed
ocean.folder_added
ocean.preview_started
ocean.preview_completed
ocean.ack_required
ocean.ack_observed
ocean.sync_started
ocean.sync_completed
```

### High-Level Page Tool Events

These are not raw DOM commands. They are stable, safe commands the Leptos stage knows how to render.

```json
{
  "schema": "ocean.ceremony.event.v1",
  "runId": "cer_20260707_abc123",
  "type": "ceremony.scene_set",
  "phase": "scan",
  "status": "progress",
  "message": "Scanning your agent constellations",
  "payload": {
    "scene": "constellation_scan",
    "focus": "globe",
    "tone": "calm_build"
  }
}
```

Supported `payload.scene` values for V1:

```txt
welcome
constellation_scan
source_review
import_stream
connect_ocean
preview_gate
sync_stream
complete
error
```

Supported UI intents:

```txt
ceremony.scene_set      payload: { scene, focus?, tone? }
ceremony.loading_set    payload: { label, progress?, total?, indeterminate? }
ceremony.highlight      payload: { target: "ack_button" | "folder_picker" | "globe" | "source:<id>" }
ceremony.toast          payload: { tone: "info" | "success" | "warning" | "error", text }
ceremony.await_user     payload: { gate: "preview_ack" | "folder_confirm" | "invite_input", instructions }
```

### Discovery Payloads

#### `agent.source_detected`

```json
{
  "type": "agent.source_detected",
  "phase": "scan",
  "status": "complete",
  "message": "Claude Code setup detected",
  "payload": {
    "sourceId": "claude-code",
    "label": "Claude Code",
    "logo": "claude",
    "root": "~/.claude",
    "counts": { "agents": 11, "skills": 28, "projects": 97, "mcpServers": 5 }
  }
}
```

#### `agent.skill_detected`

```json
{
  "type": "agent.skill_detected",
  "phase": "scan",
  "status": "progress",
  "message": "Skill found: axum",
  "payload": {
    "sourceId": "claude-code",
    "name": "axum",
    "relPath": "skills/axum/SKILL.md",
    "display": { "orbit": "skills", "color": "cyan" },
    "import": { "eligible": true, "targetPath": "/context/ocean-bedrock/imports/claude-code/skills/axum/SKILL.md" }
  }
}
```

#### `agent.project_detected`

```json
{
  "type": "agent.project_detected",
  "phase": "scan",
  "status": "progress",
  "message": "Project found: ocean-bedrock",
  "payload": {
    "sourceId": "codex",
    "name": "ocean-bedrock",
    "root": "~/dev/ocean-bedrock",
    "configDocs": ["AGENTS.md", "CLAUDE.md"],
    "sessions": 14,
    "import": { "eligible": true, "targetPath": "/sessions/<owner>/imported/codex/ocean-bedrock/" }
  }
}
```

#### `agent.secret_skipped`

```json
{
  "type": "agent.secret_skipped",
  "phase": "scan",
  "status": "info",
  "visibility": "redacted",
  "message": "Sensitive auth file skipped",
  "payload": {
    "sourceId": "codex",
    "relPath": "auth.json",
    "reason": "auth-token-file",
    "displayName": "codex auth.json"
  }
}
```

---

## WebSocket and REST Protocol

### REST Endpoints

#### `POST /api/ceremony/runs`

Creates a local ceremony run and returns a run id.

Request:

```json
{
  "runner": { "runtime": "codex", "name": "Codex" },
  "ownerName": "alice",
  "mode": "onboarding-import-v1"
}
```

Response:

```json
{
  "ok": true,
  "run": {
    "runId": "cer_20260707_abc123",
    "createdAt": "2026-07-07T21:14:00.000Z",
    "status": "running",
    "events": 0
  }
}
```

#### `POST /api/ceremony/runs/:runId/events`

Appends one event or a small batch. The local app assigns `seq` and `ts`, stores it in a ring buffer, and broadcasts to WebSocket subscribers.

Request, single event:

```json
{
  "type": "agent.skill_detected",
  "phase": "scan",
  "status": "progress",
  "message": "Skill found: axum",
  "payload": { "sourceId": "claude-code", "name": "axum" }
}
```

Request, batch:

```json
{
  "events": [
    { "type": "agent.skill_detected", "phase": "scan", "status": "progress", "message": "Skill found: axum", "payload": { "sourceId": "claude-code", "name": "axum" } },
    { "type": "agent.skill_detected", "phase": "scan", "status": "progress", "message": "Skill found: leptos", "payload": { "sourceId": "claude-code", "name": "leptos" } }
  ]
}
```

Response:

```json
{
  "ok": true,
  "runId": "cer_20260707_abc123",
  "accepted": 2,
  "nextSeq": 44
}
```

#### `GET /api/ceremony/runs/:runId`

Returns run status and replay buffer.

```json
{
  "ok": true,
  "run": { "runId": "cer_20260707_abc123", "status": "running", "lastSeq": 44 },
  "events": []
}
```

#### `POST /api/ceremony/runs/:runId/complete`

Marks the run complete and broadcasts `ceremony.run_completed` if the runner has not already emitted one.

### WebSocket Endpoint

#### `GET /api/ceremony/ws?runId=:runId&after=:seq`

- Uses the `ws` npm package on top of the existing Node HTTP server.
- Local-only. Reject non-loopback remote addresses.
- Same-origin browser clients only. Reject unexpected `Origin` when present.
- Server sends a snapshot first, then live event frames.
- Mutation is not accepted over WS in V1; the runner writes via REST only.

Server-to-client frames:

```json
{ "kind": "hello", "schema": "ocean.ceremony.ws.v1", "runId": "cer_20260707_abc123" }
{ "kind": "snapshot", "runId": "cer_20260707_abc123", "lastSeq": 41, "events": [] }
{ "kind": "event", "runId": "cer_20260707_abc123", "event": { "seq": 42, "type": "agent.skill_detected" } }
{ "kind": "heartbeat", "ts": "2026-07-07T21:14:03.128Z" }
{ "kind": "error", "error": "run not found" }
```

Client-to-server frames:

```json
{ "kind": "ping", "ts": "2026-07-07T21:14:03.128Z" }
```

V1 ignores all client frames except ping.

### Buffering and Replay

- Keep the latest 1,000 renderable events per run in memory.
- Persist the latest completed run metadata to `~/.local/state/ocean-bedrock/ceremony-runs.json` with private payloads stripped.
- If the browser reloads, it calls `GET /api/ceremony/runs/:runId` and reconnects with `after=<lastSeq>`.
- If a runner emits before the browser opens, the first WS snapshot replays the backlog.

---

## Import Safety Contract

The standardized skill may import these classes of files after preview:

```txt
Claude Code:
- ~/.claude/CLAUDE.md
- ~/.claude/agents/*.md
- ~/.claude/skills/**/SKILL.md and *.skill.md
- ~/.claude/projects/*/*.jsonl session files, after preview and size cap
- ~/.claude/settings.json with secrets redacted

Codex:
- ~/.codex/AGENTS.md
- ~/.codex/config.toml with secrets redacted
- ~/.codex/session_index.jsonl
- ~/.codex/history.jsonl, after preview and size cap
- ~/.codex/agents/** if present

Pi:
- ~/.pi/** config, skill, agent, and session files, after applying the same secret filters
- user-specified Pi config paths if ~/.pi is absent
```

Never import these raw files:

```txt
~/.claude/.credentials.json
~/.codex/auth.json
API keys, OAuth tokens, refresh tokens, keychain dumps, cookies, browser profiles
files matching DEFAULT_SECRET_PATTERNS from src/ingest-filters.mjs
```

All import runs must produce a preview event set before writing to Ocean:

```txt
import.plan_started
import.item_queued / import.item_skipped
import.plan_completed
ceremony.await_user gate=preview_ack
```

The user must ack before import/sync writes happen.

---

## Build Lane 1 — Local App Ceremony Protocol

**Owner role:** Local App Protocol Engineer

**Files:**
- Modify: `scripts/ocean-local-app.mjs`
- Modify: `package.json`
- Modify: `package-lock.json`
- Create: `scripts/check-ceremony-protocol.mjs`

**Tasks:**

- [ ] Add `ws` dependency and wire an upgrade handler to the existing Node HTTP server.
- [ ] Add in-memory ceremony run store with `createCeremonyRun`, `appendCeremonyEvents`, `getCeremonyRun`, `completeCeremonyRun`.
- [ ] Implement REST endpoints:
  - `POST /api/ceremony/runs`
  - `POST /api/ceremony/runs/:runId/events`
  - `GET /api/ceremony/runs/:runId`
  - `POST /api/ceremony/runs/:runId/complete`
- [ ] Implement WS endpoint `GET /api/ceremony/ws?runId=:runId&after=:seq` with hello, snapshot, event, heartbeat, and error frames.
- [ ] Enforce local-only access and same-origin browser clients.
- [ ] Strip or suppress `visibility=private` payloads before WS broadcast.
- [ ] Keep the existing invite/folder/preview/ack/sync endpoints unchanged.
- [ ] Add `scripts/check-ceremony-protocol.mjs` that starts the app on an ephemeral port, creates a run, posts two events, connects WS, verifies snapshot + live event frames, and exits 0.

**Acceptance:**

- `node --check scripts/ocean-local-app.mjs` passes.
- `node --check scripts/check-ceremony-protocol.mjs` passes.
- `node scripts/check-ceremony-protocol.mjs` exits 0 and prints `{ "ok": true, "createdRun": true, "postedEvents": 2, "wsReceived": true }`.
- Existing `npm run local-app:ack:check` still passes if present in this checkout.
- Existing manual flow at `localhost:8765` still loads if no ceremony bundle is built.

---

## Build Lane 2 — Leptos/WASM Ceremony Stage

**Owner role:** Leptos Ceremony UI Engineer

**Files:**
- Create: `ui/onboarding-ceremony/Cargo.toml`
- Create: `ui/onboarding-ceremony/Trunk.toml`
- Create: `ui/onboarding-ceremony/index.html`
- Create: `ui/onboarding-ceremony/src/main.rs`
- Create: `ui/onboarding-ceremony/src/types.rs`
- Create: `ui/onboarding-ceremony/src/app.rs`
- Create: `ui/onboarding-ceremony/src/globe.rs`
- Create: `ui/onboarding-ceremony/src/event_stream.rs`
- Create: `ui/onboarding-ceremony/style.css`
- Create: `assets/integrations/claude.svg`
- Create: `assets/integrations/codex.svg`
- Create: `assets/integrations/pi.svg`
- Integration note: Lane 1 owns serving `ui/onboarding-ceremony/dist`; this lane does not edit `scripts/ocean-local-app.mjs`.

**Tasks:**

- [ ] Build a Leptos 0.8 CSR app with `leptos::prelude::*`, `RwSignal<Vec<CeremonyEvent>>`, and no SSR/server functions.
- [ ] Define Rust event structs matching `ocean.ceremony.event.v1`; unknown event types must render as generic timeline entries instead of panicking.
- [ ] Implement WS subscriber that connects to `/api/ceremony/ws?runId=<id>` and applies snapshot/event frames to signals.
- [ ] Implement reconnect with `after=<lastSeq>` and visible disconnected/reconnecting states.
- [ ] Render the ceremony scenes:
  - `welcome`
  - `constellation_scan`
  - `source_review`
  - `import_stream`
  - `connect_ocean`
  - `preview_gate`
  - `sync_stream`
  - `complete`
  - `error`
- [ ] Render oceanic globe as CSS/SVG/Canvas with signal-driven layers; prefer CSS/SVG first unless frame rate fails.
- [ ] Render detected source logos orbiting only when corresponding `agent.source_detected` events arrive.
- [ ] Render project folders, skill files, MCP servers, and docs with real names from payloads.
- [ ] Render `ceremony.await_user gate=preview_ack` as a dramatic pause that points at the existing ack action, not as an auto-click.
- [ ] Add reduced-motion fallback for accessibility.

**Acceptance:**

- `cargo check --target wasm32-unknown-unknown` passes in `ui/onboarding-ceremony`.
- `trunk build --release` passes in `ui/onboarding-ceremony`.
- Loading the built UI against a fixture run displays real fixture names, source logos, and phase transitions.
- If WS disconnects, the UI shows reconnecting and resumes from `lastSeq` without duplicating cards.
- No event payload marked `visibility=private` is rendered.

---

## Build Lane 3 — Standardized Onboarding Skill Rewrite

**Owner role:** Skill Contract Engineer

**Files:**
- Create or modify: `skills/ocean-bedrock-coworker-onboarding/SKILL.md`
- Create: `scripts/ocean-ceremony-fixture.mjs`
- Create: `scripts/replay-ceremony-fixture.mjs`

**Tasks:**

- [ ] Rewrite the skill around the standardized conductor model: any skill-capable agent can run it; avoid Claude-only language except inside the Claude source scan section.
- [ ] First step: ensure local app is running or instruct runner to start `npm run ocean:app`; open `http://127.0.0.1:8765`.
- [ ] Create a ceremony run with `POST /api/ceremony/runs` and store the returned `runId` for the session.
- [ ] Define scan steps for Claude Code, Codex, and Pi config sources.
- [ ] Define safe skip rules and explicitly forbid raw token import.
- [ ] Emit discovery events for real project names, skill names, MCP server names, docs, and sessions.
- [ ] Emit import preview events before writing anything into Ocean.
- [ ] Use existing local app endpoints for invite redeem, folders, preview, acknowledgement observation, and sync.
- [ ] Pause at the preview ack gate and require human click. Poll `/api/state` until `syncUnlocked` is true before continuing.
- [ ] Add fixture/replay scripts that simulate a skill runner without requiring a real Claude/Codex/Pi setup.

**Acceptance:**

- A runner following the skill can complete a dry-run ceremony using fixture data only.
- The skill contains no instruction to use Claude OAuth, `claude setup-token`, `sk-ant-oat01-*`, or raw auth files.
- The skill treats Claude Code, Codex, and Pi as peer import sources.
- The fixture replay streams at least:
  - 3 `agent.source_detected` events
  - 5 `agent.skill_detected` events
  - 5 `agent.project_detected` events
  - 2 `agent.mcp_detected` events
  - 1 `ceremony.await_user gate=preview_ack` event
  - 1 `ceremony.run_completed` event
- `node --check scripts/ocean-ceremony-fixture.mjs` passes.
- `node --check scripts/replay-ceremony-fixture.mjs` passes.

---

## Integration Gate

Run these after all lanes land:

```bash
node --check scripts/ocean-local-app.mjs
node --check scripts/check-ceremony-protocol.mjs
node scripts/check-ceremony-protocol.mjs
node --check scripts/ocean-ceremony-fixture.mjs
node --check scripts/replay-ceremony-fixture.mjs
(cd ui/onboarding-ceremony && cargo check --target wasm32-unknown-unknown)
(cd ui/onboarding-ceremony && trunk build --release)
npm run secret:scan
npm run secret:globs:check
npm run smoke
```

Expected results:

```txt
all node --check commands pass
check-ceremony-protocol prints ok JSON
fixture replay streams events to the running local app
Leptos build produces ui/onboarding-ceremony/dist
secret gates pass
smoke passes
```

---

## Rollout Notes

- V1 is local-only and ceremony-only. Do not expose ceremony endpoints on production Ocean Bedrock.
- Existing `localhost:8765` dashboard remains the fallback if Leptos assets are absent.
- The theatrical UI should feel alive, but all writes must remain gated by existing preview/ack/sync safety.
- Logo assets must be stored locally under `assets/integrations/` with source/license recorded in the file header when possible. If a logo license is unclear, render a text monogram instead.
- This plan intentionally does not add server-side OAuth, live Pi bridging, or continuous config watchers.
