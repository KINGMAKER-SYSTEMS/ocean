time: [15:09] [21-10-26]
agent: [Codex] [review129_runtime]
worktree: [codex/factory-desktop-integration]
type: [bug report]
area: [testing]

Independent review and the focused734543c6 run found a fixture expectation
error: fallback queue rows intentionally receive a fresh local ID. Corrected
the regression to capture that queue ID after the first acknowledgement and
assert it remains unchanged after the duplicate acknowledgement, with one row.
Production behavior remains unchanged. Prior run passed13 of14; Trunk was not
started. New-head targeted verification and delta review remain pending.
Surface devlog pass leaves the existing behavioral contract unchanged;
all existing event files are preserved.
