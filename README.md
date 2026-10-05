# Ocean

The canonical shared Ocean workspace is `KINGMAKER-SYSTEMS/ocean`.
All runtime, client, and reusable-package changes land here.

- `platform/ocean-os/` — Rust runtime, daemon, TUI, CLI, and tools
- `apps/ocean-surface/` — web, native, editor, voice, and canvas clients
- `packages/ocean-agents/` — reusable profiles and generic transports

Private service implementations, production assistants, SOPs, company research,
and pre-split history live in `KINGMAKER-SYSTEMS/ocean-private`.
That repository is optional for public Ocean builds.

Read [AGENTS.md](AGENTS.md) before making changes and
[the migration guide](docs/MIGRATION.md) before moving existing work.
Component license, notice, and trademark files remain authoritative.
