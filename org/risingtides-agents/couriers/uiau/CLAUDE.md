# uiau — Agent Protocol (Us In Another Universe engine)

> An Ocean courier. The *hands* are `bin/courier`, a thin wrapper over the live
> Rust engine at the "Us In Another Universe Agent" repo (single `uiau` binary
> — tokio/reqwest/clap; external tools `ffmpeg` + `yt-dlp`; services Apify,
> Slack, Google Drive via `gws`, optionally Anthropic). This file is the
> operating protocol any agent loads before running the courier. The engine's
> own docs are authoritative: `AGENTS.md` + `GOAL.md` in the engine repo.

## Mission

Run the "us in another universe" viral-format pipeline on demand: keep the
scraped clip pool stocked, cut 9:16 montages synced to the auto-detected hero
section of Mon Rovia love songs, and post them into Slack
`#us-in-another-universe-agent` where **Eric curates by deleting** — a deleted
Slack message drops the file from Drive too, automatically. The courier
inspects and fires the engine's own verbs; the engine owns every mechanic.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll ffmpeg cuts, Slack/Drive calls, or manifest edits. Use
the harness:

- **`status`** — wrapper-level, read-only: binary built, raw-pool and output
  counts (file names only, never opens media), active Slack↔Drive manifest
  links, songs available in the audio factory, launchd sync job loaded, and
  the configured review channel.
- **`tribe <video>`** — real TRIBE brain-encoding score for one file.
  **Browser-only today**: the HF Space can't be driven headless (state
  serializes to null over HTTP), so this fails fast with a pointer.
- **`init`** — create local dirs + the Drive folder ("Us In Another
  Universe"), save settings. Idempotent.
- **`scrape`** — Apify discovery → yt-dlp download into `clips/raw/`. TikTok
  is the real source; IG hashtag scraping returns almost nothing.
- **`build`** — one montage from a named song (`--start auto` finds the hero
  section) → posts to Slack + mirrors to Drive.
- **`cook`** — THE RALPH LOOP and the explicit action verb: generate variants
  (rotating song, pacing, clip selection), optionally judge, post keepers,
  repeat until `--keepers` posted or `--rounds` hit, then drop a summary in
  the channel. **Default judge mode is `none`** = manual curation: every
  variant is posted and Eric curates by deleting in Slack. `--judge claude`
  needs a **funded** `ANTHROPIC_API_KEY` — verify before promising scores.
  `--judge tribe` cannot run headless (see `tribe` above).
- **`sync`** — reconcile: anything deleted from the Slack channel is deleted
  from Drive. Guarded (aborts on any Slack API error, skips clips < 2 min
  old). launchd already runs this every 5 minutes; a manual run is the same
  guarded verb, safe and idempotent.

## Operating protocol (follow in order)

1. **Status before action — always.** Open with `bin/courier status`. If the
   binary isn't built, stop and report the `cargo build --release` line; if
   the raw pool is empty, `build`/`cook` will bail — `scrape` first.
2. **Confirm the destination once per session.** `status` prints the review
   channel from the engine's `config.json` (default
   `#us-in-another-universe-agent`). Read it back to the operator before the
   first post-producing verb — cook/build land in a shared channel.
3. **Stock the pool deliberately.** `scrape` spends Apify credits and hits
   yt-dlp — confirm `--limit` with the operator rather than re-scraping on
   reflex. Default query is the format itself; don't drift it.
4. **Cook is the action verb.** Default invocation is manual-curation mode:
   `cook --judge none` (the engine's default) posts every variant. Confirm
   `--keepers`/`--rounds` before a run — each keeper is a real Slack post +
   Drive upload. Only reach for `--judge claude` when the operator asks AND a
   funded `ANTHROPIC_API_KEY` resolves; a judge failure mid-loop just skips
   the round, but don't promise scored runs you can't back.
5. **Curation = deleting in Slack. Nothing else.** If the operator wants a
   variant gone, they (or you, on instruction) delete the Slack message; the
   engine's `sync` drops it from Drive within ~5 minutes. Never "help" by
   deleting Drive files or manifest entries directly.
6. **Report.** Relay the cook summary (posted count, best variant when
   judged) and anything red — empty pool, missing song, judge/API failures —
   in the engine's own wording.

## Hard rules

- **Slack↔Drive reconciliation is the engine's job — never hand-delete.** No
  direct Drive deletions, no `manifest.json` edits. The manifest is the link
  table `sync` reasons over; hand edits desync it.
- **The launchd sync job stays untouched.** `com.risingtides.uiau-sync` runs
  every 5 minutes; no `launchctl` mutations, no replacement loops from this
  package. (Invoking the guarded `sync` verb manually is fine.)
- **Never hardcode tokens or secrets.** The engine resolves its own creds at
  runtime (see below); the courier neither reads nor relays them.
- **Don't bypass the harness.** `bin/courier` owns the engine path, exports
  `UIAU_HOME`, and sets cwd — orchestrate + confirm, don't exec the binary
  with ad-hoc paths.
- **Hands off the media dirs.** `clips/raw/` and `clips/out/` belong to the
  engine; count via `status`, never prune or rename by hand.
- **Songs are added by dropping files, not by code.** The rotation reads the
  Mon Hopecore factory audio dir — a new `.mp3`/`.wav`/`.m4a` there joins
  automatically. A missing song (e.g. `field_song`) is a file to drop, not a
  bug to patch.

## Secrets & scopes

The courier itself needs **no secrets**. The engine resolves everything at
runtime, by env name only:

- **Apify** — `APIFY_TOKEN` env wins; otherwise `op read` on `APIFY_OP_REF`
  (1Password). Needed by `scrape` only.
- **Slack** — `SLACK_BOT_TOKEN`, parsed by the engine from the existing
  eric-claude-bot env file under `~/.cortextos/`. Needed by
  `build`/`cook`/`sync`.
- **Drive** — the sanctioned `gws` CLI handles auth itself; no key involved.
- **Anthropic** — `ANTHROPIC_API_KEY` (env, or the carousel-agent env file),
  **only** for `cook --judge claude`, and it must be funded.

Local courier config (engine path override) goes in `courier.env`
(gitignored); see `courier.env.example`. `OCEAN_UIAU_ENGINE_DIR` defaults to
`/Users/ericcromartie/Documents/Development/Us In Another Universe Agent` —
the path contains spaces, so quote it everywhere.

## Quick reference

```bash
bin/courier status                                   # read-only engine/pool/sync overview
bin/courier scrape --limit 40                        # refresh the viral clip pool
bin/courier build --song rust --start auto           # one montage → Slack + Drive
bin/courier cook --keepers 6                         # ralph loop, manual curation (judge none)
bin/courier cook --keepers 12 --rounds 40 --judge claude --min-score 85   # scored grind (funded key)
bin/courier sync                                     # reconcile Slack deletions → Drive (guarded)
bin/courier tribe clips/out/<file>.mp4               # TRIBE score (browser-only today; fails fast)
```
