# assistants — surface-specialist agents

A class of **brain-in-the-loop** agents that *operate on a surface*. Where a
**courier** ships a payload outbound over a route, an **assistant** is a
specialist an operator *unlocks* by choosing it — and it loads a **surface
profile** that changes how the agent behaves while it's on that surface.

This is the consumer side of the Ocean OS **surface taxonomy**. Ocean OS stamps
every turn with a surface flag (`[BONZAI]`, `[SLACK]`, `[CNVS]`, `[MOBL]`,
`[VOX]`, `[BRWSR]`, `[ACP]`, `[TUI]`, `[WEB]`, `[GUI]`, `[CLI]`) and, via
`build_system_prompt → load_surface_profile`, prefers an on-disk
`assistants/<SURFACE>/system.md` over its compiled-in seed prompt. So a surface
profile is **editable data, hot-reconfigurable without a daemon rebuild** — the
Software-2.0 idea: the surface *is* the program.

> The `client_type → surface` map is owned by Ocean OS
> (`ocean-agent::surface_dir`): `surface-slack → SLACK`, `surface-canvas → CNVS`,
> `surface-mobile → MOBL`, `leo-voice → VOX`, `surface-extension → BRWSR`,
> `acp-zed → ACP`, `tui → TUI`, `surface-web → WEB`,
> `surface-gpui`/`surface-native → GUI`, `cli → CLI`. The daemon reads
> `assistants/<DIR>/system.md` (override the root with `OCEAN_ASSISTANTS_DIR`).
> Design authority for the surface model and the Slack assistant:
> [`docs/specs/2026-06-04-content-agent-slack-assistant-design.md`](../docs/specs/2026-06-04-content-agent-slack-assistant-design.md)
> (esp. §4).

## Layout convention

```
assistants/
├── _shared/                  # surface-agnostic core identity composed under EVERY profile
│   └── system.md
├── <SURFACE>/                # a house surface profile (SLACK, CNVS, MOBL, …)
│   └── system.md             # THE file Ocean OS loads for that surface
└── <assistant>/              # a named specialist with its own identity + harness
    ├── system.md             # its surface profile (e.g. bonzai → [BONZAI])
    ├── CLAUDE.md             # full operating protocol (provider-neutral)
    ├── AGENTS.md             # short agent instructions
    ├── harness/              # deterministic hands (pure stdlib, bash-drivable)
    ├── bin/                  # thin CLI wrapper
    └── artifacts/            # tools the assistant produces/uses (e.g. HTML boards)
```

Two kinds of entry live side by side here:

- **House surface profiles** — `SLACK/`, `CNVS/`, `MOBL/`, … — keyed by the Ocean
  surface flag. These hold *surface-wide* house rules (how to behave in Slack vs on
  a canvas vs on mobile) and apply to **any** assistant loaded on that surface.
- **Named specialists** — `bonzai/`, and future `content-agent/`, … — a full agent
  package (identity, harness, CLI, artifacts) bound to its own surface.

## The shared-SOP pattern (`_shared/`, and the `_base/<SURFACE>/` direction)

The goal is **no duplication of house rules across profiles.** Two layers do that:

1. **`_shared/system.md` — cross-surface core.** Surface-agnostic assistant
   identity + the agency doctrine (you have permissions and agency; when the
   operator tells you to do something, do it — don't ask permission for work you've
   been asked to do). The universal invariants are the *floor, not a leash*: only
   two hard guardrails — never leak secrets, and don't destroy work unasked (no
   force-push / rm -rf / production-data damage unless explicitly told). Composed
   *under* every surface profile. Write a house rule that's true everywhere **here,
   once.**

2. **`_base/<SURFACE>/` — per-surface house SOPs (design-spec direction, §4).** The
   spec defines a richer base where each surface's shared SOPs are split into
   focused files — e.g. for Slack: `system.md` (base surface role), `comms.md`
   (thread/DM etiquette, brevity, when to canvas), `canvas.md` (canvas rendering
   SOPs), `limits.md` (rate limits, don't-do-on-Slack rules) — so a named agent's
   own `<agent>/SLACK/` writes **only its specifics or overrides**, never the house
   rules. Today the house Slack/Canvas/Mobile SOPs live consolidated in
   `<SURFACE>/system.md`; as more agents share a surface they split out into
   `_base/<SURFACE>/` per the spec. Either way the principle holds: **common rules
   live once; per-agent files hold only deltas.**

**Composition / injection — resolved (OCEAN-80).** The spec left the injection
mechanism as an open item owned by Ocean OS (§4, §13). Verified against the live
daemon (`ocean-os crates/ocean-agent/src/lib.rs`): `build_system_prompt` =
`BASE_SYSTEM_PROMPT` + `load_project_prompt(cwd)` (ancestor-walks `AGENTS.md`/
`CLAUDE.md` **only**) + `append_client_type`, and `append_client_type` →
`load_surface_profile_from` reads **exactly one** file, `assistants/<SURFACE>/system.md`.
It does **not** read `_shared/`, does **not** path-resolve `_base/<SURFACE>/`, does
**not** concatenate. The daemon's own `surface_dir` doc comment confirms the
symlink-vs-resolver composition is *"Still parked for John."* So the daemon layering
is **flat** — the rich `_base/` composition the spec wants is not runtime-side yet.

To make the DRY pattern **real today** without a Rust change, ocean-agents assembles
the base itself with [`tools/compose_profile.py`](tools/compose_profile.py) — option
**(b)** from §4/§13 (path-resolve + concatenate; no symlink sprawl; base read-only).
It composes `_shared/system.md` + `_base/<SURFACE>/{system,comms,canvas,tools,limits,safety,vibe}.md`
(+ optional `<agent>/<SURFACE>/system.md`) into the single `assistants/<SURFACE>/system.md`
the daemon loads. That published file is a **composed artifact — do not hand-edit it**;
edit the sources and re-run `compose_profile.py <SURFACE> --write` (CI gate:
`--check`). When ocean-os ships runtime composition, the composer retires.

**Applied to every authored surface (OCEAN-89).** OCEAN-80 split `SLACK` into
`_base/SLACK/`. OCEAN-89 extends the same pattern to **all** the authored house
surfaces — `VOX`, `BRWSR`, `CNVS`, `MOBL` — so each one's house rules now live as
`_base/<SURFACE>/*.md` sources and the published `assistants/<SURFACE>/system.md` is
the composed output (which now also carries the `_shared/` core, prepended by the
composer). The house invariants live **once** in `_shared/system.md`; no surface
restates them. Per-surface section files follow a small shared vocabulary —
`system.md` (role), `comms.md` (talk/format), `canvas.md` (rich/spatial rendering),
`tools.md` (tool SOPs), `limits.md` (format/rate rules), `safety.md` (the
surface-specific shape of the safety invariants), `vibe.md` (the closing) — composed
in that order, with any other `.md` appended alphabetically.

### Authoring flow — how to change a surface profile

The published `assistants/<SURFACE>/system.md` is generated; **never hand-edit it.**
To change how an assistant behaves on a surface:

1. **Edit the source(s)** under `assistants/_base/<SURFACE>/` (or `_shared/system.md`
   for a rule that's true on *every* surface, written once).
2. **Re-compose** the published file:
   `python3 assistants/tools/compose_profile.py <SURFACE> --write`
   (for an agent override: add `--agent <agent>`).
3. **Commit BOTH** the edited `_base/<SURFACE>/*.md` source(s) **and** the regenerated
   `assistants/<SURFACE>/system.md` together.

Then the drift gate keeps source and output in lockstep:

- `python3 assistants/tools/compose_profile.py --check` — verifies **every** surface's
  published file matches its sources and is non-empty (exits non-zero on drift).
  `--check <SURFACE>` checks one. Wired three ways, all active today:
  `make assistants-check`, a pre-commit hook (`.pre-commit-config.yaml`), and CI —
  `.github/workflows/assistants-compose-check.yml` runs the same gate on every
  push/PR (activated in OCEAN-327). A hand-edited published file is caught by any
  of these and fails the gate.

> **Why the published file must never go empty or stale:** the Ocean daemon loads
> exactly `assistants/<SURFACE>/system.md` at turn time. An absent, empty, or stale
> profile is what the agent actually runs on that surface — so the composer always
> publishes the full composed text, and `--check` fails any empty/missing target.

## Registry

| Surface flag | Profile / Assistant | Domain | Status |
|---|---|---|---|
| `[BONZAI]` | **bonzai** | git hygiene / worktree — prune branch sprawl safely; HTML triage board; enforce merge→main→delete | live (R3) |
| `[SLACK]` | **SLACK** house profile | Slack-native comms (threads/DMs, mrkdwn not full Markdown, emoji-aware), canvas-rendering SOPs, inbound-only safety — the base every Slack assistant loads. Now sourced DRY from `_base/SLACK/{system,comms,canvas,limits}.md` via the composer; `SLACK/system.md` is the composed artifact | live (R5) |
| `[SLACK]` | **content-agent** | conversational content-pipeline assistant: generate video, chat/Q&A, gallery/status, **prompt library** (crowned master prompt per format + top performers + page assignments, local SQLite harness), canvas rendering — calls the content-posting-lab API; identity + `SLACK/` overrides + `tools.toml` + pipeline SOPs + inbound `bridge/` — Socket Mode listener → daemon → threaded reply, plus the canvas consumer — all wired and live | live (Slack bridge; OCEAN-112/244/375; prompt-library OCEAN-80) |
| `[CNVS]` | **CNVS** house profile | tldraw / spatial canvas — visual, durable artifacts; additive layout; confirm destructive board ops. Sourced DRY from `_base/CNVS/{system,canvas,vibe}.md`; `CNVS/system.md` is the composed artifact | live (R4 seed, OCEAN-89) |
| `[MOBL]` | **MOBL** house profile | mobile / on-the-move — glanceable, voice-friendly, answer-first; defer bulky artifacts to richer surfaces. Sourced DRY from `_base/MOBL/{system,comms,vibe}.md`; `MOBL/system.md` is the composed artifact | live (R4 seed, OCEAN-89) |
| `[VOX]` | **VOX** house profile | voice / hands-free (`leo-voice`) — spoken-clean output (no markdown/paths/code), answer-first, barge-in-aware brevity; defer bulky output to richer surfaces. Sourced DRY from `_base/VOX/{system,comms,safety,vibe}.md`; `VOX/system.md` is the composed artifact | live (R4, OCEAN-89) |
| `[BRWSR]` | **BRWSR** house profile | browser / Chrome side panel (`surface-extension`) — docked, sees the live active tab (URL/title injected per OCEAN-70), browser-tool SOPs, narrow-panel brevity. Sourced DRY from `_base/BRWSR/{system,comms,tools,safety,vibe}.md`; `BRWSR/system.md` is the composed artifact | live (R4, OCEAN-89) |
| `[CLI]` | **prompt-lab** | video-prompt optimization lab — RATCHET bake-off rounds, LAWS audits, career-mean crowning of master prompts + modular composer feeding content-agent's prompt library; engine stays at its on-disk lab (`OCEAN_PROMPT_LAB_ENGINE_DIR`) | ported (wave 1) |
| `[CLI]` | **voice-intake** | voice-memo intake — mlx-whisper GPU transcription → decipher → anchored, deduped tickets into Linear SALE/OUT/CAMP/CONT/OCEAN or GitHub Issues (rt-yt-substrate); SHA-ledger idempotent, two-track promote gate; engine's `GOAL-PROMPT.md` stays canonical | ported (wave 1) |

## Phased delivery roadmap

The surface taxonomy holds **ten** client surfaces plus the `[BONZAI]` specialist.
We author real profiles in waves rather than stubbing all ten at once — generic
seed consts in Ocean OS remain the graceful fallback for any surface not yet
authored here.

- **R3 — current.** `bonzai` (git-hygiene specialist) + the **`SLACK`** house
  profile (load-bearing: the content-agent Slack assistant depends on it). Slack is
  the first real conversational surface; its profile is complete per the design
  spec §4.
- **R4 — current.** Flesh out **`CNVS`** (Slack/tldraw canvas rendering), **`MOBL`**
  (Ocean mobile app), **`VOX`** (`leo-voice` — hands-free voice agent), and
  **`BRWSR`** (`surface-extension` — browser/Chrome control surface). All four now
  ship real profiles: CNVS/MOBL as seeds, **VOX** (spoken-clean, barge-in-aware) and
  **`BRWSR`** (docked side panel, live-tab-aware, browser-tool SOPs) at full depth.
- **R5 — later.** `ACP` (`acp-zed` editor control plane), `TUI` (terminal
  cockpit), `WEB` (Ocean Surface PWA), `GUI` (GPUI native desktop), `CLI`. These
  run on Ocean OS's built-in seed prompts today; they graduate to file-loaded
  profiles here as each surface's behavior is worth specializing.

**Adding an assistant or a surface = drop a folder with a `system.md`.** No router
edits. The surface flag is assigned by Ocean OS; the behavior is authored here.
