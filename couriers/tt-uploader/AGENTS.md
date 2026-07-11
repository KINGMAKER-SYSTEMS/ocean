# tt-uploader — agent instructions

Provider-agnostic Ocean courier: a **read-first ops window** into the TikTok
auto-uploader engine (`Auto Uploader (TT)` repo — Playwright stealth uploader +
self-healing orchestrator in `src/`, deployed FastAPI backend in `backend/`;
Google Drive intake → TikTok Studio for RT's owned pages).

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol. The engine's own
`AGENTS.md` / `README.md` (in the engine repo) are the authoritative runbooks.

Principles:
- Status before anything: open with `bin/courier status` / `heartbeat` (live
  status files — work even when the backend API is down).
- **Visibility, not triggering.** v1 has no upload verb; uploads stay entirely
  behind the engine's own orchestrator. Never run the scheduler, sandbox, or
  any mutating backend route from here.
- Cookies (`config/cookies/`, Fernet-encrypted), proxy config, and stealth
  parameters are the engine's domain — never read, edit, or tune them.
- Slack alerts are the engine's channel; the courier reports to the operator.
- No secrets in this package; the engine loads its own `.env`.
- The engine path has spaces and parens — always quote
  `$OCEAN_TT_UPLOADER_ENGINE_DIR`.

Self-test (no service needed): `bin/courier help` prints usage.
