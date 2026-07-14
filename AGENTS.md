# ocean-agents — agent index

> Loaded automatically by the Ocean runtime: `ocean-agent` walks the working
> directory's ancestors and concatenates every `AGENTS.md` / `CLAUDE.md` it
> finds into the system prompt. This file orients any agent working anywhere in
> this repo. A courier's own `CLAUDE.md` (loaded alongside this) is the detailed
> protocol for that courier.

This repo holds **provider-agnostic agent packages** that run *on top of* the
Ocean runtime. It is intentionally separate from `ocean-os` (which stays pure
Rust). Two families so far: **couriers** (ship payloads outbound over a route)
and **assistants** (brain-in-the-loop specialists that operate on a surface).

## Assistants — surface specialists

Brain-in-the-loop agents an operator *unlocks* by choosing them; each loads a
**surface profile** (`assistants/<SURFACE>/system.md`) that Ocean OS prefers over
its compiled seed prompt — editable, hot-reconfigurable, no rebuild. The surface
*is* the program. Index: [`assistants/README.md`](assistants/README.md).

| Surface | Assistant | Domain |
|---|---|---|
| `[TUI]` | house profile | terminal-native engineering with markdown plus bounded render-protocol component projections |
| `[BONZAI]` | bonzai | git hygiene — prune branch sprawl safely, HTML triage board, enforce merge→main→delete |
| `[SLACK]` `[CLI]` | content-agent | conversational content-pipeline assistant — generate video, gallery/status, prompt library, canvas rendering |
| `[CLI]` | prompt-lab | video-prompt optimization lab — RATCHET bake-off rounds, LAWS audits, crowned master prompts feeding content-agent's library |
| `[CLI]` | voice-intake | voice memos → anchored, deduped tickets (Linear SALE/OUT/CAMP/CONT/OCEAN or GitHub Issues); SHA-ledger idempotent |
| `[CLI]` | creator-research | campaign creator-research — authoritative campaign Sheet (Target List, Rate Reference, Analytics, Value Ranking by CPV); locked roster, no invented metrics |
| `[CLI]` | yt-prompt-bible | long-form YouTube format registrar — 10 hardened master-prompt formats, channel kits by codename, 11-section contract + anti-slop QA |

More surfaces to come (content tab, campaign-booking tab, finances section — each
a surface an agent specializes into).

## Couriers — a hub of single-purpose delivery agents

Narrow agents that ship a payload to a destination over a shared route, invoked
by command. Architecture: [`couriers/ARCHITECTURE.md`](couriers/ARCHITECTURE.md).

**Registry (source of truth = each `courier.toml`):**

| Slash | Courier | Mode | Does |
|---|---|---|---|
| `/ship` | file-courier | deterministic | upload `dropbox/` files to a Slack channel |
| `/say` | file-courier | deterministic | post a message to a Slack channel |
| `/resolve` | file-courier | deterministic | verify a Slack destination |
| `/sounds` | sound-watcher | deterministic | Warner tracker sweep: Audio Tracking Sheet → Slack canvases + one roll-up (`/sounds-preview` = dry-run) |
| `/babysit` | pr-babysitter | agentic | one unattended PR-maintenance pass over open PRs — fix CI/conflicts, merge safe, report (`/babysit-status` = read-only) |
| `/monmail` | mon-email-watcher | deterministic | Mon Rovîa Gmail sweep → Calendar markers + Slack drafts (`/monmail-preview` = dry-run) |
| `/distro` | mon-distro | deterministic | read-first ops window into the Mon Rovia evergreen distro engine — health, queue, upcoming; posting double-latched |
| `/uiau` | uiau | deterministic | drive the Us In Another Universe engine — scrape viral clips, cook 9:16 Mon Rovia montages to Slack for delete-to-curate review |
| `/folkedit` | folk-edit | agentic | one Stillness wave — moment-select → phrase-complete cut → five-word mantra → staged at the Slack review gate (`/folkedit-deliver`, `/folkedit-status`) |
| `/hopecore` | hopecore | agentic | pair earned-hope quotes with Mon Rovîa footage + music → one wave to Slack review, nothing auto-publishes (`/hopecore-status`) |
| `/coremusic` | core-music-editor | agentic | one Core Core acoustic-reel edit wave — liminal montage over the performance spine → Slack review previews (`/coremusic-status`) |
| `/ycp` | yt-clipper | agentic | one clip-factory autopilot pass — source → clip → Slack ✅ QC gate → scoreboard (`/ycp-scoreboard` = read-only) |

Run `python3 couriers/hub/router.py list` for the live table (it reads the
manifests, so this prose may lag — the router is authoritative).

## How to invoke a courier

- **Via the router:** `python3 couriers/hub/router.py run <slash> [args]`
  (deterministic → runs the harness; agentic → POSTs a scoped prompt to the
  daemon at `$OCEAN_DAEMON_URL`, default `:4780`).
- **Directly:** `cd couriers/<courier> && bin/courier …` (see its `CLAUDE.md`).

## Invariants (apply to every courier)

- **Confirm the destination before sending** into a shared channel — resolve it
  and read the name back to the operator first.
- **Never hardcode tokens or destinations.** Secrets come from env / a token
  file (Ocean convention: `OCEAN_<X>_…` first); destinations are supplied at run
  time. Nothing sensitive is committed.
- **Don't bypass a courier's harness.** The harness owns transport (auth, upload
  flow, rate limits, retries, ledger). Agents orchestrate + confirm.
- **Adding a courier = drop a folder with a `courier.toml`.** No router edits.

## Architecture docs

- [`docs/OCEAN_PROJECT_MAP.md`](docs/OCEAN_PROJECT_MAP.md) maps the Ocean quad:
  `ocean-os`, `ocean-agents`, `ocean-surface`, and `ocean-bedrock`.
- [`docs/AGENT_FILESYSTEM_ARCHITECTURE.md`](docs/AGENT_FILESYSTEM_ARCHITECTURE.md)
  records the typed agent-filesystem direction across the Ocean repos.
- [`docs/ocean-agents-builds.toml`](docs/ocean-agents-builds.toml) is the
  codified source of truth for the `ocean-agents` package-side build list.
- [`docs/PORTING_ROADMAP.md`](docs/PORTING_ROADMAP.md) tracks the migration of
  the existing agent fleet into packages (the porting convention + wave plan).

## Agent channel — async coordination

Agents working in parallel use [`AGENTS-CHANNEL.md`](AGENTS-CHANNEL.md) as a
shared lobby: sign in on the roster, post short notes with `#topic` tags and
@mentions, resolve threads when done. Read it when starting overlapping work or
when you were @mentioned; don't duplicate handoff content there.

## Child devlog Index

- `assistants/` — surface specialists and composed house profiles → `assistants/AGENTS.md`
- `couriers/` — manifest-driven delivery packages, hub, and shared transport → `couriers/AGENTS.md`
- `docs/` — cross-repo routing, architecture, build registration, and fleet-porting documentation → `docs/AGENTS.md`
