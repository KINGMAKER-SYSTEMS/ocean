# core-music-editor — Agent Protocol (Core Core route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the standalone
> engine at `$OCEAN_CORE_MUSIC_EDITOR_ENGINE_DIR` (default
> `/Users/ericcromartie/Documents/Development/Core Core Music Editor Agent`) —
> the ffmpeg overlay renderer, the Gemini in-point picker and reel reviewer,
> the spend-capped Grok miner, the liminal scrapers/harvester, and Slack +
> Drive delivery. **The engine and its files stay untouched** — this package
> is identity plus an on-demand path into it. This file is the operating
> protocol any agent loads before running the courier; the engine's
> `CLAUDE.md`, `factory/FORMAT-SPEC.md` (incl. the Clip Screen), and
> `factory/research/` docs remain the canonical craft docs and win on any
> conflict.

## Mission

Cut 1080x1920 acoustic-performance reels for Mon Rovia where the performance
footage is the spine and liminal montage cuts are the nervous system: the
acoustic take's audio runs continuously, the edit opens and closes on Mon,
and the middle cuts away to found, haunted-ordinary footage on lyric turns,
breaths, and chord changes. Render, then deliver previews to the Slack review
channel and mirror to Drive. The bar is a stranger stopping mid-scroll at 2am
with their throat tight. **Nothing this courier makes ever auto-publishes** —
Slack review is where every wave ends, and montage sources are not
rights-cleared for public posting until Eric clears them.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll ffmpeg, scraper, Grok, Gemini, or Slack calls. Use the
harness:

- **`bin/courier where`** — read-only. Resolved engine dir + entry checks.
- **`bin/courier status`** — read-only, spends nothing. Staged batch jobs,
  finished-reel count, Drive-mirror manifest, candidate backlog, the Grok
  spend ledger, and the tail of the loop_state scoreboard.
- **`bin/courier run <stage> [args…]`** — exec a real engine entry from the
  engine's `factory/`:
  - `run pipeline [--no-deliver]` → `reel_pipeline.py` — the orchestrator.
    `--no-deliver` renders/stages only (the review gate); without it,
    previews upload to Slack and mirror to Drive.
  - `run cuts [IDS|--dry-run]` → `gemini_cuts.py` — Gemini WATCHES each
    montage source clip and slides `source_start` to the most-alive window
    matched to the segment's emotional beat.
  - `run blackfix` → `fix_black_windows.py` — luma safety net after cuts.
  - `run review [IDS] [--slack]` → `gemini_review.py` — Gemini WATCHES each
    rendered reel and critiques pacing, cuts, framing, and clip choice
    against the batch.json edit-decision-list.
  - `run sync reconcile [--dry-run]` → `drive_sync.py` — Drive mirror +
    Slack-delete reconcile (a deleted Slack preview is the delete signal).
  - `run harvest` → `harvest_liminal_sources.py` — merge + score raw scrapes
    into `liminal_candidates.json` (a candidate board, never a cleared
    library).
  - `run pinterest|twitter` → the FREE liminal miners (Lab venv python).
  - `run miner …` → `grok_liminal_miner.py` — PAID xAI/Grok X-search miner
    behind the `factory/grok_liminal_spend.json` ledger. `--status` first,
    always.
  - `run archive|pexels` → rights-clean sourcing (Archive.org / Pexels).
  - `run ingest …` → `ingest_candidate_to_lab.py` — send reviewed candidate
    URLs to the Content Lab clipper for download/trim.

## Operating protocol (follow in order)

1. **Load the canon.** Read the engine's `CLAUDE.md`, then
   `factory/FORMAT-SPEC.md` (the locked template + Quality Gate + Clip
   Screen hard-rejects), and `factory/research/loop_state.md` — it names the
   **current locked era** (hard cuts, Mon bookend, uniform crop-to-fill,
   complete-phrase audio windows) and supersedes prior-era rubrics. Skim
   `factory/research/CRITIQUE.md` and the latest `gemini_review.md` for what
   already failed.
2. **Check state before cutting anything:** `bin/courier status`. Re-read the
   scoreboard so the wave never repeats a tried lever, reuses a clip that was
   named a killer flaw, or collides with a shipped CORE id.
3. **Source only when needed** (clip pool thin, or Eric asks): free miners
   and rights-clean sources first, the capped paid miner deliberately, then
   `bin/courier run harvest`. Scoring the haul is the harvest script's job —
   bulk model scoring runs on **local Ollama, batched**; never hand-score
   hundreds of candidates in-context. Candidates end at
   `liminal_candidates.json`; screening into `batch.json` is a separate,
   deliberate gate.
4. **Screen every clip, then pair with a written rationale.** Run the Clip
   Screen (meme/reaction, CGI/horror, staged-surreal, graphic text, nested
   frames, sub-0.7s flashes, travel-vlog prettiness, clashing grades → reject
   on sight). Per clip, one sentence: *"[what the footage shows] ↔ [the
   emotional beat of the song at this window] ↔ [the legend's technique this
   move reapplies]."* Can't write it → the pairing is random, pick again.
5. **Run the lyric-check gate** on every montage window: quote the exact
   lyric playing under the clip; the clip must match the emotional beat
   without literalizing the line. Contradiction means re-pair, not
   rationalize. No clip explains a lyric; each opens a door in the chest.
6. **Cut with Gemini, verify by eye.** New CORE-0NN id in `batch.json`, then
   `bin/courier run cuts` (Gemini watches each source clip and picks the
   window whose motion/emotion matches the beat), `bin/courier run blackfix`,
   then `bin/courier run pipeline --no-deliver`. Build/extract frames and
   LOOK: opens on Mon, closes on Mon, audio continuous, no text fighting his
   face, 9:16 fill, cuts landing on breaths/chords.
7. **Gate with the reviewer:** `bin/courier run review` — Gemini watches the
   render against the intended EDL. Fix what it names (splice-patch beats
   full re-render for single-clip fixes) or kill the cut. Log every kill.
8. **Deliver for review, never publish:** `bin/courier run pipeline` uploads
   previews to the Slack review channel and mirrors to Drive — only if the
   cut genuinely beats the current best (no channel spam; caption = what
   changed). **Slack review is the terminus.** Public posting is Eric's
   decision, after rights clearance of every montage source.
9. **Wave hygiene:** append the attempt (shipped AND killed) with scores and
   the next single lever to `factory/research/loop_state.md` conventions,
   and surface pool thin-spots or clearance needs to Eric in the report.

## Hard rules

- **Lyric-check gate is mandatory.** Every montage window gets the
  exact-lyric quote + beat match before render. Never literal lyric
  illustration; never a clip that contradicts the line under it.
- **Proven-legends mandate.** Every craft move reapplies a proven legend's
  technique, named in the written rationale — Dada found-footage collage and
  irrational juxtaposition, Fluxus event-score restraint, Murch-style
  cut-on-the-breath, the engine's GOAL-CINEMA traditions. A move no legend's
  craft backs is decoration — cut it.
- **Anti-repetition.** One focused lever per wave; a new CORE-0NN id per
  delivered version; never re-pull a lever the scoreboard shows failed;
  never reuse a clip named as a killer flaw; never deliver a non-improvement
  to Slack.
- **Respect the Grok spend caps** — $5 for any Grok-vision/OCR work and the
  $10 X-search miner ledger (`factory/grok_liminal_spend.json`). The scripts
  check the ledger before every call and stop themselves; never bypass,
  edit, or reset a ledger, and top up only when Eric says so. `run miner
  --status` before any paid run.
- **Harvest scoring is Ollama's job, not yours.** Bulk keep/drop scoring of
  scraped candidates runs through the harvest script and the local model,
  batched — never hand-scored candidate-by-candidate in-context.
- **Rights are a gate, not a vibe.** Candidate URLs and reference-account
  clips are internal-draft only; clear or re-source equivalents before any
  public Mon Rovia post.
- **Nothing auto-publishes.** Deliveries go to the Slack review channel and
  the Drive mirror, full stop. No posting to IG/TikTok/anywhere.
- **Don't bypass the harness.** The engine owns transport and mechanics
  (renderer, Gemini proxying + caches, scraper capture, spend ledger, Slack
  upload, Drive mirror + delete reconcile). Behavior changes belong in the
  engine's docs and scripts, not here.

## Secrets & scopes

No tokens live in this package. The engine self-loads its repo-root `.env`
(`SLACK_BOT_TOKEN`, `SLACK_CHANNEL`, `DRIVE_FOLDER_ID`,
`DRIVE_SOURCE_FOLDER_ID`, `LAB_ROOT`, `CONTENT_LAB_URL`, `XAI_API_KEY` —
Grok key also resolves secret-safe via the engine's `grok_client.py`, never
printed). Gemini auth resolves `GEMINI_API_KEY` → `GOOGLE_API_KEY` → Vertex
ADC (`GOOGLE_CLOUD_PROJECT`, model override via `GEMINI_MODEL`). The only
knobs here are `OCEAN_CORE_MUSIC_EDITOR_ENGINE_DIR` (where the engine
checkout lives) and `OCEAN_CORE_MUSIC_EDITOR_PYTHON` (the Lab venv python
for the scrapers) — set them in `courier.env` (gitignored), see
`courier.env.example`. Never hardcode tokens or paste key material into
code, commits, or the conversation.

## Quick reference

```bash
bin/courier where                          # engine dir + entry checks
bin/courier status                         # batch / reels / mirror / candidates / ledger / scoreboard
bin/courier run cuts --dry-run             # Gemini in-point proposals, writes nothing
bin/courier run pipeline --no-deliver      # render + stage only (the review gate)
bin/courier run review CORE-030            # Gemini watches the render, critiques vs EDL
bin/courier run pipeline                   # deliver previews → Slack review + Drive mirror
bin/courier run miner --status             # Grok ledger before any paid run
bin/courier run sync reconcile --dry-run   # preview Slack-delete → Drive reconcile
```
