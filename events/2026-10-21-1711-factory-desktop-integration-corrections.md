time: [01:11PM EDT] [21-10-26]
agent: [Codex] [factory integration builder]
worktree: [codex/factory-desktop-integration]
type: [handoff] [workflow]
area: [review] [testing]

Advanced the isolated issue #146 / PR #147 integration from
40180eacb97b4130d8df35f0def8ba7eb412b88a with actual merge commits retaining
reviewed PR #138 head 87026fdff3802276815752d202aa84f79d8af3d9 and
PR #143 head daac2673cae564021a5df3fec38bc39a1820aa7b as ancestors.
Both merges applied without source conflicts. Full session reads now use
their own latest-request projection epoch, independent of permission-only
reconciliation. Native launch cwd resolves lazily only when relative executable
or allowlisted path semantics need it. Surface devlog contracts from both repairs
survive alongside existing model readiness and startup URI contracts.

Main legacy events.md remains byte-identical, and every event from the prior
integration and both repair heads is preserved byte-for-byte. The earlier
candidate history archive remains untouched. Devlog pass required no additional
index/ownership changes. Scoped merge-delta whitespace checks passed before
freeze. Targeted projection/reconnect/reset and startup checks plus a real
integrated Trunk bundle will run on this new head; earlier integrated checks
do not substitute for these results. Source heads already had independent
review and focused checks, but this composition still needs exact-head review
and fresh required CI. No push, canonical merge, installation, deployment or
live credential/provider/coworker acceptance is claimed.
