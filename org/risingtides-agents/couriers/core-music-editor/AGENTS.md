# core-music-editor — agent instructions

Provider-agnostic Ocean courier on the **Core Core route**. The standalone
engine at `$OCEAN_CORE_MUSIC_EDITOR_ENGINE_DIR` (default
`/Users/ericcromartie/Documents/Development/Core Core Music Editor Agent`) is
the only thing that renders, scrapes, spends, or talks to Slack/Drive; the
agent loop only orchestrates, judges, and reports.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral and applies regardless of which model drives this agent under the
Ocean runtime. The engine's own `CLAUDE.md`, `factory/FORMAT-SPEC.md`, and
`factory/research/` docs stay canonical on craft and win on any conflict.

Principles:
- The engine owns the mechanics: ffmpeg renderer, Gemini in-point picker +
  reel reviewer, spend-capped Grok miner, harvesters, Slack upload, Drive
  mirror. Never hand-roll those calls.
- One wave = one focused improvement, a new CORE-0NN id, lyric-check gate on
  every montage window, a legend's technique named per craft move, and a
  render → Slack review gate. Nothing auto-publishes.
- Grok spend caps ($5 vision/OCR, $10 miner ledger) are absolute; harvest
  scoring runs on local Ollama via the harvest script, never in-context.
- The engine, its batch/state/ledger files, and its research canon stay
  untouched by this package. Behavior changes belong in the engine's docs.
- No tokens here — the engine self-loads its own `.env`; local config goes in
  `courier.env` (gitignored, see `courier.env.example`).

Self-test (read-only, starts nothing): `bin/courier where`
