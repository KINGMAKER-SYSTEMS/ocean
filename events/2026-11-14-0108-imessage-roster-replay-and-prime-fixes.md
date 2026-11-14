time: [01:08] [11-14-26] [America/New_York]
agent: [claude] [fable 5.2]
worktree: [claude/imessage-team-roster-20261113]
type: [review] [bug-report]
area: [backend] [testing] [docs]

PR #199 follow-up to the reviews of head fc23a82b: the factory reviewer's hold
(issues 206 and 207), three automated Codex findings, and my delegated
reviewer's fourth pass (no blocker).

Start marker (Codex P1, factory P2, issue 206): the daemon's replay history is
bounded and can begin after a turn's turn_started, so a lost oversized frame of
the current turn could be taken for history and a partial reply sent. The
reader now accepts no frame of a turn until that turn's own start marker has
been seen; a replay without it fails closed and the turn is cancelled. A probe
of one real turn on the operator's daemon showed the order session_created,
turn_started, assistant_text_delta, turn_finished, each carrying the turn id.
Regression: testAReplayThatBeginsAfterThisTurnsStartMarkerIsNeverTrusted.

Empty database prime (Codex P2, factory P2, issue 207, delegated should-fix):
a cursor of zero was read as "never started", so an empty Messages database was
primed again past its first message. The ledger now records that it was primed
separately from the cursor. Regression:
testAnEmptyMessagesDatabaseIsPrimedOnceAndItsFirstMessageIsAnswered.

Deadline (Codex P1): the wait-for-turn check counted from admission, which gave
an already old message up to another 15 minutes. The scan now carries each
message's sent time and the deadline runs from there.

Left as documented boundaries: ledger custody follows the lock file's path; a
reader that has worked does not restart itself after a later outage.

Validation: swift test passed 57 tests; release build passed. Further checks
are recorded in the pull request. Not verified, as before: the installed app
reading Messages under its own permissions, a real send, and the Automation
prompt. Nothing installed, loaded, or granted.
