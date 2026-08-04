# uiau — agent instructions

Provider-agnostic Ocean courier over the **Us In Another Universe** Rust
content engine (viral "us in another universe" format: scraped TikTok/IG
clips → 9:16 montages cut to Mon Rovia hero sections → Slack review channel
→ Drive mirror → Slack-deletion reconciliation).

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol. The engine's
own `AGENTS.md`/`GOAL.md` (in the engine repo) are authoritative on internals.

Principles:
- The engine owns transport and mechanics: Apify/yt-dlp scraping, ffmpeg
  cuts, hero-section detection, Slack/Drive publishing, and the manifest.
  Never hand-roll any of it — go through `bin/courier`.
- `status` before action; `cook` (default `--judge none`) is the explicit
  action verb — every keeper is a real post into a shared Slack channel.
- Curation is deleting the Slack message; the engine's `sync` (launchd,
  every 5 min) drops it from Drive. Never delete Drive files or edit
  `manifest.json` by hand, and never touch the launchd job.
- Secrets resolve inside the engine (env / 1Password / gws); nothing is
  hardcoded or committed. Local overrides go in `courier.env` (gitignored).

Self-test (no secrets needed):
```bash
bin/courier status
```
