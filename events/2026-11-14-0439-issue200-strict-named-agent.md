time: [11:39PM America/New_York] [11-13-26]
agent: [Codex desktop] [factory builder, coordinator and independent reviewer]
worktree: [codex/factory-recovery-20261114]
type: [bug report]
area: [backend]

Issue #200 repairs the daemon authority gap found while reviewing PR #199. Added opt-in `require_named_agent` to product turns: admission captures one resolved profile and rejects a missing name, missing/invalid definition, empty policy or subprocess-capability profile before session/lifecycle/request publication. Agent enforces the captured exact-name ceiling after all tool additions, refuses zero matches before inference, and preserves it through provider fallback. Ordinary named-agent fail-open behavior, Room admission and `without_tools()` remain compatible.

The flag is not authentication, a per-member grant, runtime revocation, filesystem confinement or operator-context isolation. Older daemons may ignore the additive field; clients need a supporting daemon. PR #199's author-owned adapter and its other findings remain untouched. Existing independent approval, required builds and deployment holds remain in force.

Validation at this event: SDK request compatibility 5 tests passed; strict Agent tests 4 passed (real-loop visibility, zero-match refusal, provider fallback and subprocess pre-persistence refusal); legacy narrowing test passed; shared-request workspace/tests compilation passed. Daemon focused regression tests are compiling. A fixture cleanup assertion was corrected after proving refusal creates no state directory. No provider calls, live runtime changes, merge, installation or live messaging acceptance are claimed. Final exact-head review and required CI follow this entry.

Devlog pass: updated crate-level cross-owner contract, Agent and daemon contracts, and SDK README. Root indexes and TUI guidance remain unchanged because no ownership boundary changes and TUI edits only initialize the legacy-default false field. Existing event files are immutable. The two earlier local recovery/review entries travel with this substantive repair; no status-only PR is created.

Operational receipt: desktop storage recovered and bounded local builds are running with two jobs and no debug/incremental artifacts. Mini SSH timed out; its last successful 04:18 UTC health receipt is historical, not current runtime verification. Local checkpoint is newer than the last mirrored remote checkpoint. No continuous-availability claim.
