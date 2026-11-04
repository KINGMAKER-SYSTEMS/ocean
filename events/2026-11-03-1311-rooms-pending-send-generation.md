time: [13:11] [03-11-26] America/New_York
agent: Codex [GPT-6]
worktree: fix/rooms-draft-ownership; follow-up to a7ba4af43ec4d4c76f86eb2c0809f01a92f6a304
type: bug report
area: frontend

Follow-up review found that `Rooms::post_message` drops failure results after
its captured generation stops matching, so a pending UI latch cannot survive
navigation. Room/generation changes now retire the pending confirmation while
the origin/key-owned draft remains saved. A stale A failure is still unknown;
returning to A restores the draft and leaves sending enabled. No `rooms.rs`
change was needed: its existing generation fence is the authoritative guard.

Added a regression for a pending A HTTP failure while B is active, the stale
generation rejection (including A reopened under the same key), draft restore,
and retry admission. Validation: formatting passed; all 1,097 UI unit tests and
83 UI integration tests passed; WASM cargo check passed; release Trunk build
passed to `/tmp/ocean-room-draft-dist` with WASM magic `0061736d`; diff check
passed. Compiler emitted the existing unused-import and dependency future-
compatibility warnings. Build targets and output stayed under `/tmp`.

No room messages, provider calls, deployment, secret access, or runtime control.
The earlier ledger entry remains unchanged. The Surface contract records the
unknown-send behavior; root devlog and indexes remain unchanged because
ownership and tree structure did not change.
