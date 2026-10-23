time: [15:08] [21-10-26]
agent: [Codex] [review129_runtime]
worktree: [codex/factory-desktop-integration]
type: [bug report]
area: [frontend] [testing]

Issue148 review4340730702 found that the daemon rejects oversized steering
before assigning/admitting a receipt, but Surface classified that definitive
413 as unknown. The classifier now recognizes only status413 plus ok:false
and the daemon's exact oversized-steering error. Other/malformed/proxy413
responses remain unknown. Existing202/409/429 handling is unchanged.

Focused fixtures drive the production ACK completion seam with a64KiB text
attachment, preserving full wire text, compact display, client identity and
exactly one queued row after duplicate acknowledgements. Ambiguous413 fixtures
retain unresolved intent without queueing. Source inspection confirms daemon
main.rs rejects oversized text before message creation or inbox admission;
that unchanged backend path remains outside this correction's owned files.

Nearest Surface contract updated; no package/index boundary changes. Existing
events and legacy ledger remain untouched. Final-head targeted UI verification,
real Trunk bundle and independent delta source review remain pending. No push,
merge, installation, deployment or live-provider outcome is claimed.
