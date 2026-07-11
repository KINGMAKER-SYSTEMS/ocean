# folk-edit — agent instructions

Provider-agnostic Ocean courier over **The Stillness Page** engine (the Mon
Rovia folk edit agent — a standalone production checkout at
`$OCEAN_FOLK_EDIT_ENGINE_DIR`, default
`/Users/ericcromartie/Documents/Development/mon rovia folk edit agent`; the
path has spaces, always quote it). The engine owns the whole pipeline —
Gemini moment selection, phrase-complete cuts, the fixed five-word mantra,
Slack delivery, Drive mirror; the agent loop only curates and confirms.

See [`CLAUDE.md`](CLAUDE.md) for the operating protocol. **The engine's own
`CLAUDE.md` and `OPERATOR-PROMPT.md` (in the engine dir) stay canonical** —
this package distills and defers to them.

Principles:
- The locked format is law: same five-word mantra, same type, same full-bleed
  9:16 framing, phrase-complete cuts, forever. Footage is the only variable.
- The Slack review gate (`#mon-rovia-edits`) is never skipped; nothing
  auto-publishes. Stage with `--no-deliver`, human eyeball, then deliver.
- Curation follows the engine's evidence-ranked taste hierarchy (gesture >
  soft eyes > settled delivery > raw intimacy > medium shot).
- Secrets live in the engine's own `.env` (Gemini / Slack / Drive), never in
  this package. Package knobs come from `courier.env` (gitignored).
- Don't bypass the harness: `bin/courier run|status|where` is the way in.

Self-test (read-only, no render):
```bash
bin/courier where && bin/courier status
```
