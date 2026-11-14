time: [00:45] [11-14-26] [America/New_York]
agent: [claude] [fable 5.2]
worktree: [claude/imessage-team-roster-20261113]
type: [review] [bug-report]
area: [backend] [testing] [docs]

PR #199 follow-up to the factory reviewer's hold at e53d19fa, the automated
Codex P1 on that head, and my delegated reviewer's third pass (no blocker).

First-run prime (Codex P1, factory hold): roster cleanup wrote an empty ledger
before the missing-ledger check, so a first start answered the previous 15
minutes. Housekeeping no longer writes when nothing changes, and the rule is now
"a ledger whose cursor is zero is primed to the newest row", in
StateStore.primeIfUnscanned. Regression:
testFirstStartBeginsFromThePresentWhateverRunsBeforeThePrime runs the startup
order (custody, sweep, roster cleanup, prime) against a fixture database.

Ledger custody (factory hold): the startup sweep closed whatever was waiting
without proving its owner had exited, so a second runner could close a live
runner's queue. run now takes a process-lifetime kernel file lock on the ledger
(RunnerLock) before the sweep and a second runner is refused. Regression:
testOnlyOneRunnerCanHoldALedgerSoTheStartupSweepCannotCloseALiveRunnersQueue.

Oversized stream frames (issue 206): a frame over the line cap was dropped
before its event or turn was known, so a completed turn could be answered with
partial text. The splitter now reports the loss with how the line began; the
frame is skipped only when it cannot shape the reply (history before this
turn's turn_started, or a non-text event during it) and otherwise the turn
fails closed and is cancelled. Regressions:
testAnOversizedFrameThatCouldShapeTheReplyFailsClosedInsteadOfSendingAPartialAnswer
and testOversizedFramesThatCannotShapeTheReplyAreSkippedAndTheAnswerIsIntact.

Also from the third pass: the age bound is applied again when a queued
message's turn comes; a process that has read Messages no longer exits during a
later reader outage; the sweep logs how many messages it closed; poll logs an
overload refusal.

Validation: swift test passed 55 tests; release build, installer shell syntax,
cargo xtask docs-check, and git diff --check passed. A fourth rehearsal against
the live local daemon started the bridge with no ledger and a five-minute-old
message already present: it was not admitted, and a message sent after the
start was admitted and answered. Not verified, as before: the installed app
reading Messages under its own permissions, a real send, and the Automation
prompt. Nothing installed, loaded, or granted.
