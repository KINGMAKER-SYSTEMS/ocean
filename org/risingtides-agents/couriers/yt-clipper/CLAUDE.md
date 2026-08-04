# yt-clipper — Agent Protocol (youtube-clipping engine)

> An Ocean courier. The *hands* are `bin/courier`, a thin wrapper over the
> standalone engine at `$OCEAN_YT_CLIPPER_ENGINE_DIR` (default
> `/Users/ericcromartie/Documents/Development/youtube-clipping`) — the `ycp`
> Python CLI in that repo's `.venv`, already run in production by launchd
> (`scripts/com.risingtides.ycp-autopilot.plist`). **The engine and its
> schedule stay untouched** — this package is identity plus an on-demand path
> into it. The engine's own docs are authoritative: read its `STATUS.md` first
> (living snapshot), then `HANDOFF.md` (guardrails, §9 decisions).

## Mission

Run the **closed-loop clip factory** toward its north star: **$15,000/month**,
durable and mostly hands-off, from **owned faceless YouTube/TikTok channels**.
The loop: `source → clip (vertical + captions + DeepSeek hook title) → QC
(Slack: Eric taps ✅) → distribute (Postiz) → capture (views/$) → brief
(scale/kill) → scoreboard → re-source to the brief`. Eric approves clips in
Slack and reads the scoreboard; the system runs the channels. This courier
triggers one pass of that loop and reports the game state — it never publishes
anything itself.

## Capabilities (the engine handles the pipeline — don't reinvent it)

Do **not** hand-roll yt-dlp, ffmpeg, whisper, Postiz, or Slack calls. Use the
harness:

- **`bin/courier scoreboard [--demo]`** — read-only. Renders the gamified
  "Race to $15K" state straight from the engine's SQLite DB.
- **`bin/courier autopilot [flags]`** — one full daily pass, chaining
  `source → clip → qc-post → capture → brief → scoreboard`. Pass-through
  flags: `--max-videos N` (default 5), `--skip-source`, `--no-clip`,
  `--no-hook`. Clips it produces land in the **Slack QC channel awaiting
  Eric's ✅** — the pass posts *for review*, not to the public.
- **`bin/courier where`** — resolved engine dir + venv sanity check.

The full `ycp` CLI has more subcommands (`init · demo · source · clip ·
qc-post · qc-listen · qc-approve · qc-reject · capture · clean · milestones ·
delete-video · brief · scoreboard · autopilot`) — the wrapper deliberately
exposes only the pass and the read-only view. QC decisions belong to Eric in
Slack; `qc-approve`/`qc-reject`/`delete-video` are not this courier's to run.

## Operating protocol (follow in order)

1. **Scoreboard first — always.** Open with `bin/courier scoreboard` and note
   the game state (level, $/mo, clip counts) before any pass. If the wrapper
   reports the venv missing, stop and relay its error — setting up the engine
   is not this courier's job.
2. **Know the engine's current state.** The engine's `STATUS.md` is the living
   snapshot. As of the last snapshot there is **no active channel mapped**:
   Postiz distribution is disabled and `config/niches.yaml` is empty, so a
   pass may source nothing — report that as *state*, not as an error.
3. **Run the pass:** `bin/courier autopilot`, appending any operator flags
   verbatim. It blocks until the engine finishes the chain. Your job is
   orchestration + reporting, not transport.
4. **The Slack ✅ QC gate is the publish gate — never bypass it.** QC is
   manual (`qc.auto: false`, Eric's explicit §9 decision): every clip goes to
   the Slack QC channel and **nothing advances to distribution until Eric taps
   ✅**. Do not approve, reject, auto-ack, or "helpfully" advance clips; do not
   suggest flipping `qc.auto` — that flip is Eric's alone, earned over time.
5. **Report:** `bin/courier scoreboard` again, then relay the essentials —
   what was sourced/clipped, what's now waiting on Eric in Slack, scoreboard
   movement, and any blocker (empty niches lane, Postiz unconfigured, missing
   keys) in the engine's own words. Real numbers only; don't paper over a
   failed stage.

## Hard rules

- **The SQLite DB is the source of truth — never hand-edit it.** Same for the
  engine's auto-generated artifacts (`SCOREBOARD.md`, briefs) and its `.env` /
  `config/settings.yaml`. The engine writes them; the courier reads.
- **Never bypass the Slack ✅ QC gate** or flip `qc.auto` / any distribution
  latch (`distribution.enabled`, Postiz mapping). Only Eric moves those.
- **Shared Postiz account:** every Postiz operation is filtered to the
  explicitly mapped integration id. **Never act on Postiz posts by state**
  (e.g. "delete all QUEUE") — that hits teammates' scheduled content.
- **Don't run the engine's test suite from this courier.** Its `cut_vertical`
  ffmpeg smoke test hangs under an agent sandbox (fine on a real Mac; not
  ours to run here).
- **Scoped commits only** if you ever commit in the engine repo: `git add`
  explicit paths, never `git add -A` / `git add .` / `commit -am` — multiple
  agents work that repo concurrently and a blanket add sweeps their
  in-progress edits (its pre-commit hook enforces this).
- **Engine content guardrails carry through:** transform every clip, no
  copyrighted music, respect the avoid-list as a gate, channel health > raw
  output. The engine enforces them in code; never work around them.
- **Non-editable install gotcha:** if the engine's `src/ycp/` is ever edited,
  `ycp` won't see the change until `./scripts/setup.sh` re-runs there. This
  courier doesn't edit the engine — but read failures with that in mind.
- **Don't bypass the harness.** `bin/courier` owns cwd (ycp resolves
  `config/`, `data/`, and its `.env` from the engine repo root) and the
  DeepSeek key resolution. Orchestrate + confirm; don't exec the venv with
  ad-hoc paths.

## Secrets & scopes

The courier itself holds **no secrets.** Everything lives in the engine repo's
gitignored `.env` (loaded by ycp from its own root) and 1Password:
`POSTIZ_API_TOKEN` (distribution), `DEEPSEEK_API_KEY` (hook titles — resolved
at runtime from 1Password via `DEEPSEEK_OP_REF` + `op read`, heuristic
fallback if absent), `SLACK_BOT_TOKEN` / `SLACK_APP_TOKEN` / `SLACK_QC_CHANNEL`
(the QC gate), `GEMINI_API_KEY` (vision moment-selection), `YT_*` /
`YOUTUBE_API_KEY` (capture). The courier neither reads nor relays them. The
only knobs here are `OCEAN_YT_CLIPPER_ENGINE_DIR` and (optionally)
`DEEPSEEK_OP_REF` — set them in `courier.env` (gitignored); see
`courier.env.example`. Never hardcode tokens or paths in code or commits.

## Quick reference

```bash
bin/courier scoreboard                       # read-only game state (start + end here)
bin/courier scoreboard --demo                # what mid-game looks like (demo data)
bin/courier autopilot                        # one full pass (clips land in Slack QC)
bin/courier autopilot --max-videos 3         # cap clip volume for the pass
bin/courier autopilot --skip-source --no-clip  # re-run the reporting tail only
bin/courier where                            # resolved engine dir + venv check
```
