# mon-distro — agent instructions

Provider-agnostic Ocean courier: a **read-first ops window** into the Mon Rovia
evergreen distro engine (Rust `mon-posting-agent` in the Postiz Posting Agent
repo — launchd, `127.0.0.1:3000`, rotation queue `data/queue.json`, posts
Facebook Reels via Postiz).

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol. The engine's own
`OPERATIONS.md` (in the engine repo) is the authoritative runbook.

Principles:
- Status before action: open with `bin/courier health` / `status`.
- The courier inspects; the engine posts. Never flip `DRY_RUN`,
  `POSTING_LOCKED`, or `OCEAN_MON_DISTRO_ALLOW_POSTING` without explicit
  operator instruction.
- Engine repo, launchd jobs, and queue state stay untouched — no `.env` edits,
  no queue hand-edits, no `launchctl` calls from here.
- No secrets in this package; the engine binary loads its own `.env`.
- The engine path has a space — always quote `$OCEAN_MON_DISTRO_ENGINE_DIR`.

Self-test (no service needed): `bin/courier help` prints usage.
