---
name: ocean-bedrock-coworker-onboarding
description: Onboard an Ocean Bedrock coworker by scanning local Claude Code, Codex, and Pi configurations and streaming a safe, one-time import ceremony into the Ocean local stage at http://127.0.0.1:8765. Runtime-agnostic — any skill-capable agent can run it.
---

# Ocean Bedrock Coworker Onboarding

A standardized conductor skill that turns a freshly invited Ocean Bedrock
coworker machine into a synced Ocean context source. The runner performs local
scans, streams a live ceremony into the Ocean local stage, and completes a
single, preview-gated, human-approved import of **real config/project/skill
names** — while never importing secrets or auth material.

## Runtime neutrality

This skill is written for **any skill-capable agent** that can read a skill,
make HTTP calls to `127.0.0.1`, and read local files it is already authorized
to read: Claude Code, Codex, Pi, OMP, or a future runtime. Nothing in the
protocol, event names, endpoint names, or copy assumes a specific runner. The
only runner-specific content is the per-source scan map below (where to find
each agent's local config), and even there Claude Code, Codex, and Pi are
treated as **peer sources of equal rank**.

V1 is a **one-time import per ceremony run**. Continuous watchers, live Pi
bridging, and server-side OAuth are explicitly out of scope.

## What the runner needs

- Read access to the user's home directory config folders (`~/.claude`,
  `~/.codex`, `~/.pi` or a user-specified Pi path).
- The ability to make `POST`/`GET` calls to the Ocean local app.
- The Ocean local app running at `http://127.0.0.1:8765` (see Step 1).
- An invite token for the coworker's Ocean Bedrock account (the runner redeems
  it via the existing invite endpoint — it does not create accounts).

## Hard safety rules — read first

These are non-negotiable. Violating any of them is a ceremony failure.

1. **Never import raw auth or token files.** This includes, but is not limited
   to: Claude OAuth tokens, anything produced by `claude setup-token`, any
   string beginning with `sk-ant-oat01-`, `~/.claude/.credentials.json`,
   `~/.codex/auth.json`, API keys, refresh tokens, keychain dumps, cookies,
   browser profiles, `.env` files, `.pem`/`.key`/`.p12`/`.pfx`/keystore files,
   private SSH keys (`id_rsa`, `id_ed25519`, …), `.aws`/`.ssh`/`.gnupg`
   directories, and any path that matches `DEFAULT_SECRET_PATTERNS` from
   `src/ingest-filters.mjs`. When the runner encounters one, it emits an
   `agent.secret_skipped` event and moves on. **No token bytes ever leave the
   machine.**
2. **The ceremony must never use Claude OAuth.** Do not run
   `claude setup-token`, do not read OAuth subscription tokens, and do not
   look for `sk-ant-oat01-*`. "Importing Claude Code configuration" means
   reading the plain config/skill/project/session *files* under `~/.claude`,
   nothing more.
3. **The human acknowledgement gate stays human.** The runner may open and
   visually highlight the preview-acknowledgement step, but it **must not**
   click "I reviewed this preview — unlock sync" on the coworker's behalf.
   The runner pauses at the gate and resumes only after `GET /api/state`
   reports `syncUnlocked: true`.
4. **Real names, safe contents.** Display real project folder names, skill
   names, agent names, MCP server names, and doc filenames. Never render raw
   secrets, token values, or keychain items.
5. **No writes before preview + ack.** Every import item is represented in a
   preview event set before anything is written to Ocean, and nothing is
   written until the human acknowledges.
6. **Pi means config import.** Treat Pi as another local agent config source
   (`~/.pi` or a user-specified path). Do not assume Pi Cockpit or a live hub.
7. **Local only.** All calls go to `127.0.0.1`. Do not expose ceremony
   endpoints on production Ocean Bedrock.

## Protocol at a glance

- Event envelope: `schema: "ocean.ceremony.event.v1"`. The runner sends the
  envelope **without** `runId`, `seq`, or `ts` in the per-event body — the
  local app assigns those (the run id comes from the URL; `seq` is server-
  assigned; a runner timestamp is accepted only as `payload.runnerTs`).
- `runner.runtime` must be one of: `claude-code`, `codex`, `pi`, `omp`,
  `other`.
- Phases: `boot | scan | review | import | connect | preview | ack | sync |
  complete | error`.
- `visibility: "private"` events are stored but not rendered.
  `visibility: "redacted"` events render labels/counts only — use this for any
  event that touches a sensitive path.
- Events are append-only. Corrections are represented by later events.

---

## Step 1 — Boot the local stage

Goal: make sure the Ocean local app is running and the coworker can see it.

1. Check whether the local app is up by calling `GET /api/state`. If it
   responds, continue.
2. If it does not respond, start it yourself or ask the coworker to:
   ```bash
   npm run ocean:app
   ```
   …from the `ocean-bedrock` checkout, then re-check `GET /api/state`.
3. Open `http://127.0.0.1:8765` in the coworker's browser so the stage is
   visible. (The runner does not need a browser; this is for the human.)

Do **not** create the ceremony run until `/api/state` responds.

## Step 2 — Create the ceremony run

Create a run and keep the returned `runId` for the entire session.

```http
POST /api/ceremony/runs
Content-Type: application/json

{
  "runner": { "runtime": "codex", "name": "Codex" },
  "ownerName": "alice",
  "mode": "onboarding-import-v1"
}
```

```json
{ "ok": true, "run": { "runId": "cer_20260707_abc123", "createdAt": "…", "status": "running", "events": 0 } }
```

Set `runner.runtime` to whichever agent is actually running this skill
(`claude-code`, `codex`, `pi`, `omp`, or `other`). Store `runId`. From here on,
all events are posted to `POST /api/ceremony/runs/:runId/events`.

Emit the opening events:

```json
{ "type": "ceremony.run_started", "phase": "boot", "status": "started",
  "message": "Onboarding ceremony started for alice",
  "payload": { "ownerName": "alice", "runner": { "runtime": "codex", "name": "Codex" }, "mode": "onboarding-import-v1" } }

{ "type": "ceremony.scene_set", "phase": "boot", "status": "progress",
  "message": "Welcome to Ocean",
  "payload": { "scene": "welcome", "focus": "globe", "tone": "calm_build" } }
```

You may post a single event as the body, or a small batch:

```json
{ "events": [ { "type": "…", "phase": "…", "status": "…", "message": "…", "payload": { } } ] }
```

## Step 3 — Scan peer agent config sources

Move into the scan phase and announce it:

```json
{ "type": "ceremony.scene_set", "phase": "scan", "status": "progress",
  "message": "Scanning your agent constellations",
  "payload": { "scene": "constellation_scan", "focus": "globe", "tone": "calm_build" } }
```

Scan the three peer sources as equals. For each source that exists, emit one
`agent.source_detected` event with real counts. Skip a source entirely if its
root is absent.

### Claude Code — root `~/.claude`

Eligible to import (real names only):
- `~/.claude/CLAUDE.md`
- `~/.claude/agents/*.md`
- `~/.claude/skills/**/SKILL.md` and `*.skill.md`
- `~/.claude/projects/*/*.jsonl` session files (after preview + size cap)
- `~/.claude/settings.json` **with secrets redacted**

Never import: `~/.claude/.credentials.json`, OAuth tokens, `sk-ant-oat01-*`,
`claude setup-token` output, or anything matching the secret patterns.

```json
{ "type": "agent.source_detected", "phase": "scan", "status": "complete",
  "message": "Claude Code setup detected",
  "payload": { "sourceId": "claude-code", "label": "Claude Code", "logo": "claude", "root": "~/.claude",
    "counts": { "agents": 8, "skills": 6, "projects": 18, "mcpServers": 3 } } }
```

### Codex — root `~/.codex`

Eligible to import:
- `~/.codex/AGENTS.md`
- `~/.codex/config.toml` **with secrets redacted**
- `~/.codex/session_index.jsonl`
- `~/.codex/history.jsonl` (after preview + size cap)
- `~/.codex/agents/**` if present

Never import: `~/.codex/auth.json`, `.env`, API keys.

```json
{ "type": "agent.source_detected", "phase": "scan", "status": "complete",
  "message": "Codex setup detected",
  "payload": { "sourceId": "codex", "label": "Codex", "logo": "codex", "root": "~/.codex",
    "counts": { "agents": 0, "skills": 0, "projects": 12, "mcpServers": 1 } } }
```

### Pi — root `~/.pi` (or user-specified path)

Eligible to import: any config, skill, agent, and session files under the Pi
config root, after applying the same secret filters. If `~/.pi` is absent, ask
the coworker for their Pi config path (or accept it as a runner argument) and
scan there instead. Do **not** assume Pi Cockpit or a live hub.

```json
{ "type": "agent.source_detected", "phase": "scan", "status": "complete",
  "message": "Pi setup detected",
  "payload": { "sourceId": "pi", "label": "Pi", "logo": "pi", "root": "~/.pi",
    "counts": { "agents": 4, "skills": 3, "projects": 7, "mcpServers": 1 } } }
```

Whenever the scanner encounters a forbidden/sensitive file, emit a redacted
skip immediately — the user should see safety happen in real time:

```json
{ "type": "agent.secret_skipped", "phase": "scan", "status": "info", "visibility": "redacted",
  "message": "Sensitive auth file skipped",
  "payload": { "sourceId": "codex", "relPath": "auth.json", "reason": "auth-token-file", "displayName": "codex auth.json" } }
```

## Step 4 — Emit discovery events

For each real thing found, emit one discovery event. These are what the stage
renders as orbits/folders/cards. Use real names.

`agent.skill_detected`:

```json
{ "type": "agent.skill_detected", "phase": "scan", "status": "progress",
  "message": "Skill found: axum",
  "payload": { "sourceId": "claude-code", "name": "axum", "relPath": "skills/axum/SKILL.md",
    "display": { "orbit": "skills", "color": "cyan" },
    "import": { "eligible": true, "targetPath": "/context/ocean-bedrock/imports/claude-code/skills/axum/SKILL.md" } } }
```

`agent.project_detected`:

```json
{ "type": "agent.project_detected", "phase": "scan", "status": "progress",
  "message": "Project found: ocean-bedrock",
  "payload": { "sourceId": "codex", "name": "ocean-bedrock", "root": "~/dev/ocean-bedrock",
    "configDocs": ["AGENTS.md", "CLAUDE.md"], "sessions": 14,
    "import": { "eligible": true, "targetPath": "/sessions/alice/imported/codex/ocean-bedrock/" } } }
```

`agent.mcp_detected`:

```json
{ "type": "agent.mcp_detected", "phase": "scan", "status": "progress",
  "message": "MCP server found: context7",
  "payload": { "sourceId": "claude-code", "name": "context7", "transport": "http", "scope": "user",
    "import": { "eligible": true } } }
```

`agent.doc_detected` (real filenames only):

```json
{ "type": "agent.doc_detected", "phase": "scan", "status": "progress",
  "message": "Doc found: AGENTS.md",
  "payload": { "sourceId": "codex", "name": "AGENTS.md", "relPath": "~/.codex/AGENTS.md", "import": { "eligible": true } } }
```

`agent.session_detected` (counts only — never raw contents):

```json
{ "type": "agent.session_detected", "phase": "scan", "status": "progress",
  "message": "Sessions found: 14 for ocean-bedrock",
  "payload": { "sourceId": "claude-code", "project": "ocean-bedrock", "count": 14,
    "import": { "eligible": true, "targetPath": "/sessions/alice/imported/claude-code/ocean-bedrock/" } } }
```

Batch the discovery events (5–50 at a time) to keep the stream responsive.

## Step 5 — Import plan + preview (before any write)

Move to the review/import phases, then emit the full preview event set. The
preview must be produced **before** anything is written to Ocean.

```json
{ "type": "ceremony.scene_set", "phase": "review", "status": "progress",
  "message": "Reviewing discovered sources",
  "payload": { "scene": "source_review", "focus": "sources", "tone": "calm_build" } }

{ "type": "import.plan_started", "phase": "import", "status": "started",
  "message": "Import plan built for 24 items",
  "payload": { "itemCount": 24, "sources": ["claude-code", "codex", "pi"] } }
```

Emit one `import.item_queued` per eligible item, and one
`import.item_skipped` (redacted) per sensitive item:

```json
{ "type": "import.item_queued", "phase": "import", "status": "progress",
  "message": "Queued skill: axum",
  "payload": { "kind": "skill", "sourceId": "claude-code", "name": "axum", "relPath": "skills/axum/SKILL.md",
    "import": { "eligible": true } } }

{ "type": "import.item_skipped", "phase": "import", "status": "info", "visibility": "redacted",
  "message": "Skipped sensitive codex auth.json",
  "payload": { "sourceId": "codex", "relPath": "auth.json", "reason": "auth-token-file" } }

{ "type": "import.plan_completed", "phase": "import", "status": "complete",
  "message": "Import preview ready for review",
  "payload": { "queued": 24, "skipped": 4 } }
```

## Step 6 — Connect Ocean (existing endpoints)

Use the **existing** local app endpoints — do not invent new ones. Redeem the
invite, then stage the folders the coworker wants to import.

```http
POST /api/invite/redeem   { "token": "<invite token>" }
POST /api/folders          { "path": "~/dev/ocean-bedrock", "label": "ocean-bedrock" }
```

Emit a ceremony event for each so the stage reflects the connect phase:

```json
{ "type": "ocean.invite_redeemed", "phase": "connect", "status": "complete",
  "message": "Invite redeemed", "payload": { "ownerName": "alice" } }

{ "type": "ocean.folder_added", "phase": "connect", "status": "progress",
  "message": "Folder staged for import: ~/dev/ocean-bedrock",
  "payload": { "path": "~/dev/ocean-bedrock", "label": "ocean-bedrock" } }
```

## Step 7 — Preview gate + human acknowledgement

Generate the dry-run preview using the existing endpoint, then surface the
human gate. **Do not click the unlock button for the coworker.**

```http
POST /api/preview   {}
```

```json
{ "type": "ocean.preview_started", "phase": "preview", "status": "started",
  "message": "Dry-run preview generated", "payload": { "itemCount": 24 } }

{ "type": "ocean.preview_completed", "phase": "preview", "status": "complete",
  "message": "Preview complete — awaiting your review", "payload": { "queued": 24, "skipped": 4 } }

{ "type": "ocean.ack_required", "phase": "preview", "status": "blocked",
  "message": "Acknowledge the preview to unlock sync", "payload": { "gate": "preview_ack" } }

{ "type": "ceremony.highlight", "phase": "preview", "status": "info",
  "message": "Review the preview, then click the unlock button",
  "payload": { "target": "ack_button" } }

{ "type": "ceremony.await_user", "phase": "ack", "status": "blocked",
  "message": "Waiting for you to acknowledge the preview",
  "payload": { "gate": "preview_ack",
    "instructions": "In the Ocean stage, click “I reviewed this preview — unlock sync”. The runner will resume automatically." } }
```

Now **pause**. Poll `GET /api/state` on a ~1.5s interval until
`syncUnlocked === true`. That flag only becomes true after the human clicks the
unlock button (which calls `POST /api/preview/ack` from the browser). The runner
must not call `/api/preview/ack` itself. Once unlocked, emit the resume event:

```json
{ "type": "ocean.ack_observed", "phase": "ack", "status": "complete",
  "message": "Preview acknowledged by user — sync unlocked", "payload": { "gate": "preview_ack" } }
```

If the poll runs longer than the coworker expects, emit a `ceremony.toast` to
reassure them the runner is still waiting, and keep waiting.

## Step 8 — Sync

Run the real sync via the existing endpoint and stream the sync phase:

```http
POST /api/sync   {}
```

```json
{ "type": "ocean.sync_started", "phase": "sync", "status": "started",
  "message": "Sync started", "payload": { "items": 24 } }

{ "type": "ocean.sync_completed", "phase": "sync", "status": "complete",
  "message": "Sync complete", "payload": { "items": 24 } }
```

## Step 9 — Complete

```json
{ "type": "ceremony.run_completed", "phase": "complete", "status": "complete",
  "message": "Onboarding ceremony complete for alice",
  "payload": { "ownerName": "alice", "sources": ["claude-code", "codex", "pi"] } }
```

Then mark the run complete (the local app will broadcast
`ceremony.run_completed` if you have not already):

```http
POST /api/ceremony/runs/:runId/complete   {}
```

On any unrecoverable failure, emit `ceremony.error` with `phase: "error"` and a
safe (non-sensitive) message, then stop.

## Event reference

| Phase     | Events |
|-----------|--------|
| boot      | `ceremony.run_started`, `ceremony.scene_set` |
| scan      | `agent.source_detected`, `agent.skill_detected`, `agent.project_detected`, `agent.mcp_detected`, `agent.doc_detected`, `agent.session_detected`, `agent.secret_skipped`, `ceremony.loading_set` |
| review    | `ceremony.scene_set`, `ceremony.toast` |
| import    | `import.plan_started`, `import.item_queued`, `import.item_skipped`, `import.plan_completed` |
| connect   | `ocean.invite_redeemed`, `ocean.folder_added` |
| preview   | `ocean.preview_started`, `ocean.preview_completed`, `ocean.ack_required`, `ceremony.highlight`, `ceremony.await_user` |
| ack       | `ocean.ack_observed` |
| sync      | `ocean.sync_started`, `ocean.sync_completed` |
| complete  | `ceremony.run_completed` |
| any       | `ceremony.error` |

High-level page-tool events (`ceremony.scene_set`, `ceremony.loading_set`,
`ceremony.highlight`, `ceremony.toast`, `ceremony.await_user`) carry stable
payloads the stage knows how to render — see the plan for the full payload
schema.

## Real-scan ceremony (agent-steered)

`scripts/ocean-ceremony-scan.mjs` is the standardized, agent-steered way to run
this skill for real. It scans the coworker's actual local agent configs
(`~/.claude`, `~/.codex`, `~/.pi`, and `~/.omp` when present), emits the same
ceremony event timeline as the fixture, but with **real counts and names** from
the machine — and drives the real preview/ack/sync pipeline.

Secret handling reuses `src/ingest-filters.mjs` (`DEFAULT_IGNORES` +
`classifySensitivePath`, the logic exercised by `scripts/check-secret-globs.mjs`)
plus the explicit auth-token-file set (`.credentials.json`, `auth.json`, …).
Sensitive files are reported as `agent.secret_skipped` with
`visibility:"redacted"` and a `displayName` only — no token bytes ever leave the
machine.

Dry run (no local app needed; prints JSONL + real per-source counts to stdout):

```bash
node scripts/ocean-ceremony-scan.mjs --dry-run
```

Live, agent-steered run against the Ocean local stage:

```bash
npm run ocean:app            # in another terminal, then open http://127.0.0.1:8765
node scripts/ocean-ceremony-scan.mjs --base http://127.0.0.1:8765 --name 'Jane'
```

Set `--runtime` to the agent actually steering (`claude-code`, `codex`, `pi`,
`omp`, or `other`). The scanner creates the run, streams the full scan → review
→ import → connect → preview timeline, and at the preview gate it:

1. Runs the real dry-run preview itself (`POST /api/preview`) so the coworker
   sees the real changed/skipped/sensitive-excluded plan.
2. Emits `ceremony.await_user gate=preview_ack` and **pauses**, polling
   `GET /api/state` until `syncUnlocked === true`. **It never calls
   `POST /api/preview/ack`** — only a real human clicking "unlock sync" in the
   Ocean stage does that. `--ack-timeout <sec>` bounds the wait (default 300; 0
   = wait forever).
3. After unlock, emits `ocean.ack_observed` and runs the real sync
   (`POST /api/sync`), then completes the run.

The ceremony timeline narrates; the real endpoints do the work — one unified
thing. Use the fixture replay below only to rehearse on a machine with no real
configs yet.

## Fixture and replay

A coworker machine often has no real Claude/Codex/Pi setup to scan yet. Two
scripts let the runner rehearse the full ceremony with synthetic (but
realistic) data:

- `scripts/ocean-ceremony-fixture.mjs` — the fixture dataset and a frame
  builder that emits an acceptance-compliant event timeline (3 sources, 6
  skills, 6 projects, 4 MCP servers, docs, sessions, secret skips, the import
  plan, the preview gate, sync, and completion).
- `scripts/replay-ceremony-fixture.mjs` — streams that timeline into the Ocean
  local app as if it were a real skill runner.

Dry run (no local app needed):

```bash
node scripts/replay-ceremony-fixture.mjs --dry-run
```

Live run against a running local app:

```bash
npm run ocean:app            # in another terminal
node scripts/replay-ceremony-fixture.mjs --base http://127.0.0.1:8765 --runtime codex --name "Codex"
```

The replay pauses at the preview-ack gate and waits for a real human click
(`GET /api/state` → `syncUnlocked: true`). For hands-off demos only, the replay
accepts `--simulate-ack` (drives preview/ack itself) and `--run-sync` (performs
a real sync). **These flags are fixture conveniences and are never part of this
skill's instructions.** The skill itself always waits for the human.

## Acceptance summary

A run following this skill is correct when:

- It contains no instruction or action that uses Claude OAuth,
  `claude setup-token`, `sk-ant-oat01-*`, or raw auth/token files.
- Claude Code, Codex, and Pi are all treated as peer import sources.
- Discovery events carry real project/skill/MCP/doc/session names.
- Every sensitive file is reported via `agent.secret_skipped` and never
  imported.
- The import plan is fully previewed before any write.
- The runner pauses at `ceremony.await_user gate=preview_ack` and resumes only
  after `/api/state` reports `syncUnlocked: true`.
- The run ends with `ceremony.run_completed` and a complete call.
