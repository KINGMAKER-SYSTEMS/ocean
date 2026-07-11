# Porting Roadmap — fleet → ocean-agents packages

> Source of truth for the migration of the existing agent fleet
> (`~/Documents/Development/`, `~/Projects/active/`) into ocean-agents
> packages. Inventory audited 2026-07-10 (45 project dirs; 19 active
> production agents).

## The porting convention (what a "port" is — and is not)

ocean-agents holds **manifests, personas, protocols, SOPs, and thin
harnesses** — never the engine (see `docs/ocean-agents-builds.toml`, A10:
end state is "manifests, profiles, skills, SOPs, and test fixtures only").
A port does NOT vendor the source codebase. Engines stay where they run
today (their launchd/cron jobs untouched); the package is the agent's
**identity on Ocean rails**:

```
couriers/<name>/            assistants/<name>/
├── courier.toml            ├── <name>.toml        # class="assistant", surfaces
├── CLAUDE.md               ├── CLAUDE.md          # operating protocol
├── AGENTS.md               ├── AGENTS.md          # short orientation
├── bin/courier             ├── CLI/system.md      # surface persona + overrides
│   (thin exec → engine)    ├── CLI/tools.toml     # tool grants
└── courier.env.example     └── <name>.env.example
```

- **Engine location** comes from `OCEAN_<NAME>_ENGINE_DIR` (env), with the
  current on-disk path as the documented default. Nothing sensitive in git.
- **Couriers** = narrow outbound jobs, slash-invokable via the hub router.
  `mode = "deterministic"` when the harness needs no judgment; `"agentic"`
  when the daemon should reason (prompt-driven).
- **Assistants** = brain-in-the-loop specialists; manifest mirrors
  `content-agent.toml` (CLI surface binding first; SLACK later as the
  bridge generalizes).
- Adding a package = drop the folder. No router edits, no daemon rebuild.

## Wave 1 — ported (2026-07-10)

| Package | Class | Mode | Source engine | Notes |
|---|---|---|---|---|
| `couriers/sound-watcher` | courier | deterministic | `~/Projects/active/warner-sound-watcher` | Warner tracker Sheet → Slack canvases, hourly launchd; on-demand sweep via slash |
| `couriers/pr-babysitter` | courier | agentic | `~/Documents/Development/pr-babysitter-agent` | unattended PR fixing/merging (gh + code agent); cron stays |
| `couriers/mon-email-watcher` | courier | deterministic | `~/Documents/Development/mon-rovia-email-watcher` | Rust Gmail sweep → Calendar markers + Slack drafts, daily 9 AM |
| `couriers/mon-distro` | courier | deterministic | `~/Documents/Development/Postiz Posting Agent` | Rust Postiz rotation queue → FB Reels; status/schedule commands |
| `assistants/prompt-lab` | assistant | agentic | `~/Documents/Development/prompting agent` | video-prompt evolution lab; crowns masters that feed content-agent's prompt library |
| `assistants/voice-intake` | assistant | agentic | `~/Documents/Development/Voice Memos To Do` | voice memos → anchored tickets (Linear/GitHub); mlx-whisper harness |

## Wave 2 — ported (2026-07-10)

| Package | Class | Source | Notes |
|---|---|---|---|
| `couriers/uiau` | courier (agentic) | `Us In Another Universe Agent` | Rust montage engine → Slack/Drive; `uiau cook` loop |
| `couriers/folk-edit` | courier | `mon rovia folk edit agent` | Stillness Page render loop → Slack review gate |
| `couriers/hopecore` | courier | `Mon Hopecore Agent` | quote-paired render loop; sibling of folk-edit |
| `couriers/core-music-editor` | courier | `Core Core Music Editor Agent` | acoustic-reel engine; sibling of folk-edit |
| `assistants/creator-research` | assistant | `Creator Research Agent` | campaign target-list + proposal sheets (gws) |
| `assistants/yt-prompt-bible` | assistant | `yt-longform-prompt-bible` | long-form format registry + QA contract |
| `couriers/yt-clipper` | courier (agentic) | `youtube-clipping` | `ycp` autopilot; Slack QC gate |
| `couriers/tt-uploader` | courier | `Auto Uploader (TT)` | TikTok Studio uploader; keep engine sovereign |

## Wave 3 — when their build stabilizes

`sales-agent` (scaffold phase), `outreach-agent`, `proposals-agent`,
`carousels-agent`, `mon rovia watcher` (spec-only until dashboard repo is
located), `Goldford Lore Agent` (research corpus → maybe a lore assistant),
`flow-writer-agent` (auditor/revisor personas → skill pack candidates),
`Agentic OS` knowledge API (integration point, not a package),
`campaign hub` / `lifted-ship-clipping` (platforms, not agents — Ocean
*clients*, not packages).

## Not porting

Empty/stub dirs (`ec-skills-agent`, `github repo agent`, `Label`,
`TaskLord Update Bot`, `Telegram Distro Agent`, `Tok Voice`,
`open-montage-agent`, `RT-Youtube Agenda`, `MON UGC PAGES`), one-off
analyses (`tiktok-dm-intel`, `sentiment-tagging`, `Label Chats`), static
sites (`Coach G Development`), personal-OS template family
(`personal-os-template`, `real-estate-OS` — separate product), paused
specs (`Robinhood Agent`, `System Fine TUNE`), `Vaultkeeper` (own runtime;
revisit as an Ocean peer), `TASK LORD` (meta-board; integrates via its own
SQLite, not a package).
