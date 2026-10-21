time: [12:57pm EDT] [21-10-26]
agent: [Codex] [factory integration builder]
worktree: [codex/factory-desktop-integration]
type: [handoff] [workflow]
area: [review] [testing]

Prepared issue #146 in an isolated factory worktree, integrating canonical main
0a00d9cb2ea2840307ffd8c351f9790912986508 with desktop candidate
63d8c0279a9658bb66f4bad24dc6f2d63ecf3f3e and repair heads
1ca67d8a0dd251e407843c7c7d91184f8de62eaf (#138),
e632aab667bb549eb0b3805c20c1782e1afdb17c (#142),
16be8ecb8d6c888a0c7a6c464678ec358eb5d97c (#143), and
d47737507640770bc3e6ba4f2cdff2af1da3171a (#145) through actual merge commits.
Existing candidate, repair and canonical branches were not advanced.

Three main/candidate conflicts required explicit resolution: retain both UI
accounts and agent_events_reset modules; retain the debug-only Rooms acceptance
binary and Windows Observatory dependency; keep main events.md byte-identical.
Both histories were proven append-only from a3af6a0e7dca1b21a48c7188c1cc415b4628d42c.
The complete 36,295-byte candidate tail is preserved verbatim with provenance in
2026-10-21-1640-factory-candidate-history.md; repeated markers and order remain.
All four repair merges composed without source conflicts. Surface, packaging
and MCP devlogs retain their repair contracts. The workflow devlog now points
to immutable root events entries instead of the frozen legacy ledger.

At source freeze: Git conflict/whitespace checks, TOML parsing, main ledger byte
comparison and every pre-existing event blob comparison passed. Focused Python
packaging/build/release fixtures, UI readiness/reconnect/reset and MCP tests are
planned after this commit under one bounded host slot. Native startup tests are
not rerun here, and no stale web bundle is substituted. Fresh required CI must
compile the integrated real web/native deliverables. Independent final-head
review, required checks and explicit merge holds remain pending. This record
claims source integration only: no publication, merge to canonical main,
installation, deployment or live provider/coworker acceptance.
