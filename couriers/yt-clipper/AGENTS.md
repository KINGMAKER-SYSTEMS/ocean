# yt-clipper — agent instructions

Provider-agnostic Ocean courier over the **youtube-clipping engine** (the
`ycp` Python CLI at `$OCEAN_YT_CLIPPER_ENGINE_DIR`, default
`/Users/ericcromartie/Documents/Development/youtube-clipping`). The engine is
a closed-loop clip factory: big-creator videos → vertical captioned clips →
Slack QC (Eric taps ✅) → Postiz → owned faceless channels, chasing
$15K/month. The courier triggers one autopilot pass or reads the scoreboard;
the engine does everything else.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is
provider-neutral and applies regardless of which model drives this agent
under the Ocean runtime.

Principles:
- The engine owns the pipeline: sourcing, cutting, captions, hooks, QC posts,
  capture, briefs, scoreboard. Never hand-roll yt-dlp/ffmpeg/Postiz/Slack calls.
- **The Slack ✅ QC gate is the publish gate.** Nothing distributes until Eric
  approves in Slack; the courier never approves, bypasses, or flips `qc.auto`.
- The engine's SQLite DB is the source of truth; `SCOREBOARD.md` and briefs
  are auto-generated. Read, never hand-edit.
- Secrets (Postiz / DeepSeek / Slack / Gemini / YouTube) live in the engine's
  `.env` + 1Password — the courier holds none.
- Don't run the engine's pytest from here (an ffmpeg smoke test hangs under
  the agent sandbox).

Sanity check (no creds needed):
```bash
bin/courier where          # engine dir + venv resolution
bin/courier scoreboard     # read-only game state from the DB
```
