# sound-watcher — agent instructions

Provider-agnostic Ocean courier over the **Warner Sound Watcher engine**
(`~/Projects/active/warner-sound-watcher`, override via
`$OCEAN_SOUND_WATCHER_ENGINE_DIR`). The engine — not this package — owns sheet
reads, the additive diff, Slack canvas/message writes, `state.json`, and the
Sound Board publish. It already runs hourly under launchd; this courier is the
on-demand path only.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral and applies regardless of which model drives this agent under the
Ocean runtime.

Principles:
- `preview` first (engine dry-run, no writes, no state change); confirm with
  the operator before `sweep` — a sweep posts an `@channel` roll-up into
  `#warner-ugc-sounds` and republishes the public Sound Board.
- Never touch the engine, its launchd job, or `state.json` (additive-only —
  corrupting state means duplicate pings to the whole team).
- No tokens, channel ids, or spreadsheet ids here — reads ride the
  authenticated `gws` CLI; writes use the bot env the engine loads itself.

Self-test (safe, read-only): `bin/courier preview`
