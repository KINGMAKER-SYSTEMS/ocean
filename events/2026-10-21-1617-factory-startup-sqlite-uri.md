time: [12:17] [21-10-26]
agent: [Codex] [factory coordinator]
worktree: [codex/factory-desktop-startup-env]
type: [bug report] [refactor]
area: [review] [testing]

Independent review of PR143 de832f0 withheld source ACK: filesystem anchoring
could convert SQLite file::memory:?cache=shared to a disk filename. This scoped
follow-up preserves literal memory, parsed in-memory URI and absolute file URI
values without changing their bytes; unsupported relative disk URIs fail before
spawn with an environment-key-only diagnostic. Both database overrides receive
regression coverage. Surface AGENTS records this constraint; indexes unchanged.

The earlier head's real Trunk bundle and seven startup-model tests passed, but
that evidence is not a pass for this changed head. Rustfmt and diff check pass;
fresh final-head tests, independent review and hosted checks are required.
Existing event files, frozen PR138/142 and external candidate branches remain
untouched. No real database, provider credentials or live service was used.
