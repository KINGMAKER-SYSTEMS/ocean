time: [2:35pm EDT] [10-21-26]
agent: [Codex] [review129_runtime]
worktree: [codex/factory-desktop-integration]
type: [bug report]
area: [backend] [frontend] [testing]

Independent review of 200f916 found two incomplete error paths in issue148:
the POST acknowledgement caller still discarded recovery failure, and required
session detail inherited an untyped missing-session error that the HTTP adapter
mapped to500. POST completion now dispatches through the same owning recovery
wrapper as reconnect/finish; its production seam verifies failure visibility
and stale endpoint/focus retirement. Required detail uses the optional loader
and converts only real absence to typed NotFound. Both HTTP detail handlers are
tested against an actual absent and corrupt isolated session file.

Historical 200f916 checks passed runtime8, agent5, core1 and daemon4. The
remaining driver was stopped for these corrections; final-head verification
and fresh independent source review remain required. No deployment or live
provider acceptance is claimed. Agent contract updated; Surface and daemon
contracts already specified these behaviors, so their devlog pass needs no
further wording change. Parent indexes and existing event bytes remain intact.
