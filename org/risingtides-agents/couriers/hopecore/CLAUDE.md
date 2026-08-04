# hopecore — Agent Protocol (Mon Rovîa hopecore route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the standalone
> engine at `$OCEAN_HOPECORE_ENGINE_DIR` (default
> `/Users/ericcromartie/Documents/Development/Mon Hopecore Agent`) — scrapers,
> spend-capped Grok miners, Ollama harvest scoring, the Gemini moment finder,
> the hero/broll composer, and Slack delivery. **The engine and its files stay
> untouched** — this package is identity plus an on-demand path into it. This
> file is the operating protocol any agent loads before running the courier;
> the engine's `CLAUDE.md` and `factory/QUALITY-PROMPT.md` remain the
> canonical craft docs and win on any conflict.

## Mission

Make relics, not content: pair a **hopecore quote** — universal hope /
healing / resilience sentiment — with the Mon Rovîa footage moment whose
emotional truth matches it, render it as persistent on-screen text (hero =
Mon on camera; broll = atmospheric), score it under Bloodline-era music, and
deliver it to Slack for Eric's review. Hope must be **earned** — it
acknowledges the dark and still chooses the light. Never toxic positivity.
Every piece is one felt thing; if it wouldn't stop a stranger scrolling at
midnight, it doesn't ship. Nothing this courier makes ever auto-publishes.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll ffmpeg, scraper, Grok, Gemini, or Slack calls. Use the
harness:

- **`bin/courier where`** — read-only. Resolved engine dir + entry checks.
- **`bin/courier status`** — read-only, spends nothing. Hook-bank size,
  candidate backlog, both Grok spend ledgers ($10 miner / $5 OCR), and the
  tail of the RALPH-LOOP scoreboard (what already shipped or got killed).
- **`bin/courier run <stage> [args…]`** — exec a real engine entry from the
  engine's `factory/` with the Lab venv python:
  - `run render …` → `hopecore_render.py` — the composer. `--mode hero|broll`,
    `--hook "…"`, `--audio … --audio-start …`, and `--auto-moment` to have
    Gemini WATCH the footage and pick the cut window paired to the quote.
  - `run moment …` → `gemini_moment.py` — dry-run the moment finder alone.
  - `run deliver …` → `deliver_hopecore.py` — ship (mp4, caption) pairs to the
    hopecore Slack channel for review.
  - `run harvest …` → `harvest_to_bank.py` — clean + **Ollama-score** raw
    scrapes into `candidates.json` (never the bank).
  - `run pinterest|twitter|tiktok …` → the FREE miners (headed-Chrome network
    capture / yt-dlp caption enumeration).
  - `run miner …` → `grok_miner.py` — PAID X-search miner, **$10 ledger cap**.
  - `run ocr …` → `ocr_tiktok.py` — PAID Grok-vision on-screen-text OCR,
    **separate $5 ledger cap**.

## Operating protocol (follow in order)

1. **Load the canon.** Read the engine's `CLAUDE.md`, then
   `factory/QUALITY-PROMPT.md` (the *how well*), and the docs they name:
   `factory/FORMAT-SPEC.md` (the locked template), `factory/RALPH-LOOP.md`
   (non-negotiables + the scoreboard of everything shipped),
   `factory/lyrics/PAIRING-MAP.md` (song meanings + minimum audio offsets),
   and `factory/hopecore_hooks.json` (the bank — Eric-curated, read-only).
2. **Check state before making anything:** `bin/courier status`. Re-read the
   scoreboard so the wave never reuses a shipped quote or repeats a
   clip+theme combination.
3. **Harvest only when needed** (bank thin on a theme, or Eric asks): run the
   free miners first, the capped paid ones deliberately, then
   `bin/courier run harvest`. Scoring the haul is **Ollama's job** (local
   model, batched) — never hand-score hundreds of candidates in-context.
   Harvest ends at `candidates.json`; **Eric promotes winners into the bank.**
4. **Pair with a written rationale.** One sentence per piece: *"[quote theme]
   ↔ [what the footage actually shows] ↔ [what the song is doing at this
   moment]."* Can't write it → the pairing is random, pick again. Run the
   lyric-check gate: quote the exact lyric playing under the on-screen text;
   contradiction means re-pair, not rationalize.
5. **Cut with Gemini, verify by eye.** `bin/courier run render --auto-moment …`
   lets Gemini watch the clip and return the window whose emotional truth
   pairs with the quote. Extract and Read frames: first-3-seconds gate
   (first frame + first line must arrest with zero context) and a mid-frame
   legibility/composition check.
6. **Critic, then kill pass.** Score /25 per RALPH-LOOP; ship only ≥20 with
   no dimension <3 AND surviving the kill pass. Log every kill.
7. **Deliver to Slack for review:** `bin/courier run deliver …`, then the
   ranked recap to the hopecore channel. **Nothing auto-publishes** — Slack
   review is where every wave ends.
8. **Wave hygiene:** append every attempt (shipped AND killed) to the
   scoreboard, surface the bank's thin themes to Eric as promotion
   suggestions, and make the footage ask specific.

## Hard rules

- **Never toxic positivity.** Earned hope only — acknowledges the dark, still
  chooses the light. No religion-specific preaching, no hustle/grind hope, no
  politics, no romantic/parasocial couple notes.
- **Never invent Mon's biography or lyrics.** The bank and `factory/lyrics/`
  are the only lyric sources; only VERIFIED lyrics go on screen.
- **Respect the Grok spend caps** — the $5 OCR ledger (`grok_ocr_spend.json`)
  and the $10 miner ledger (`grok_spend.json`). The scripts check the cap
  before every call and stop themselves; never bypass, edit, or reset a
  ledger, and top up only when Eric says so.
- **Harvest scoring is Ollama's job, not yours.** Bulk keep/drop + theme +
  mode scoring runs on the local model via `harvest_to_bank.py`.
- **Never write `hopecore_hooks.json`.** Harvest ends at `candidates.json`;
  the brand-true filter stays human-in-the-loop — Eric promotes.
- **Nothing auto-publishes.** No posting to IG/TikTok/anywhere; deliveries go
  to the Slack review channel, full stop.
- **Don't bypass the harness.** The engine owns transport and mechanics
  (scraper capture, spend ledgers, Gemini upload, composer, Slack upload).
  Behavior changes belong in the engine's docs and scripts, not here.

## Secrets & scopes

No tokens live in this package. Keys resolve at runtime inside the engine —
`XAI_API_KEY` (Grok miner + OCR; loaded secret-safe from 1Password by the
engine's `grok_client.py`, never printed), `GEMINI_API_KEY` (moment finder;
model override via `HOPECORE_GEMINI_MODEL`), and the Slack bot token +
`SLACK_CHANNEL` from the engine's own repo-root `.env`. The only knob here is
`OCEAN_HOPECORE_ENGINE_DIR` (where the engine checkout lives) — set it in
`courier.env` (gitignored), see `courier.env.example`. Never hardcode tokens
or paste key material into code, commits, or the conversation.

## Quick reference

```bash
bin/courier where                                    # engine dir + entry checks
bin/courier status                                   # bank / candidates / ledgers / scoreboard tail
bin/courier run render --src CLIP.mp4 --out out.mp4 \
    --mode broll --hook "…" --auto-moment            # Gemini-cut render
bin/courier run moment --src CLIP.mp4 --hook "…" --mode broll   # dry-run the finder
bin/courier run harvest                              # clean + Ollama score → candidates.json
bin/courier run deliver '[["out.mp4","caption"]]'    # Slack review delivery
```
