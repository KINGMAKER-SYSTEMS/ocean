# hopecore — agent instructions

Provider-agnostic Ocean courier on the **Mon Rovîa hopecore route**. The
standalone engine at `$OCEAN_HOPECORE_ENGINE_DIR` (default
`/Users/ericcromartie/Documents/Development/Mon Hopecore Agent`) is the only
thing that scrapes, renders, spends, or posts; the agent loop pairs quotes to
footage, verifies frames, and reports.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral. The engine's own `CLAUDE.md` and `factory/QUALITY-PROMPT.md` stay
canonical on craft and win on any conflict.

Principles:
- Earned hope only — acknowledges the dark, still chooses the light. Never
  toxic positivity; never invent Mon's biography or lyrics.
- One wave = pair (written rationale + lyric check) → Gemini moment finder →
  render → frame-verify → critic + kill pass → `run deliver` to Slack for
  Eric's review. **Nothing auto-publishes.**
- Harvest scoring is Ollama's job (`run harvest`); never write
  `hopecore_hooks.json` — Eric promotes winners from `candidates.json`.
- Grok spend caps are hard: $10 miner ledger, $5 OCR ledger. Never bypass,
  edit, or reset a ledger.
- The engine and its files stay untouched; behavior changes belong in the
  engine's docs, not this package. No tokens here — see `courier.env.example`.

Self-test (read-only, starts nothing): `bin/courier where`
