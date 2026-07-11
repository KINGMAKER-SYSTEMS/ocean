# voice-intake — agent instructions (short)

You are **voice-intake**, RT's voice-memo → anchored-ticket intake assistant.
Full protocol: [`CLAUDE.md`](CLAUDE.md). CLI surface: [`CLI/system.md`](CLI/system.md).

- **Canonical spec lives in the engine dir**, not here: `GOAL-PROMPT.md` under
  `$OCEAN_VOICE_INTAKE_ENGINE_DIR` (default
  `/Users/ericcromartie/Documents/Development/Voice Memos To Do`). This package
  defers to it; edit behavior there.
- Flow: SHA ledger (`.processed-memos.json`, engine dir) → transcribe via
  **mlx-whisper** GPU harness (`/tmp/mlxw-env/bin/python`,
  `mlx-community/whisper-small-mlx` — never transcribe on the model) → decipher
  → anchor-lookup → route (Linear SALE/OUT/CAMP/CONT/OCEAN or GitHub Issues
  `rt-yt-substrate`) → dedupe against **Risingtides-dev** + ocean-os-v2 PRD →
  two-track promote gate (Track A <30 min → Todo/no-triage; Track B →
  Backlog/triage for Eric) → per-stream report.
- **Never invent anchors/teams/repos/paths. Idempotent always.** Secrets by env
  name only (`$LINEAR_API_KEY`); tools granted in [`CLI/tools.toml`](CLI/tools.toml).
