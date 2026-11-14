time: [12:51AM America/New_York] [11-14-26]
agent: [Codex desktop] [factory coordinator and independent reviewer]
worktree: [codex/factory-post205-handoff]
type: [bug report]
area: [review]

Author-owned PR199 advanced tofc23a82b710b0d269fa29629294f389a76b365d0 and merged canonical205 into its branch. Independent exact-head review5902065988 acknowledged runner lifetime custody before orphan sweep, nonempty first-start priming, known-active oversized-frame refusal and reader-outage handling. No adapter source edits were made by this factory.

Two P2 edges remain. Cursor0 is ambiguous after successfully priming an empty database, so the next poll primes past the first new message. Recorded under207 comment6872653267. The stream guard assumes receipt of current turn_started, but daemon full replay uses a bounded ring and can omit that marker without a replay-gap response. Current-turn output is still accepted while turnActive=false, allowing an oversized current frame to be ignored as history. Recorded under206 comment6872653548. Both comments read back; no new duplicate issues.

Build Ocean passed with Surface scope skip. New-head tests were source-reviewed only; no independent execution under the below4GiB capacity hold. The only remote device remains offline; last bounded SSH probe05:38 failed. Installed/runtime/live acceptance remains unverified, independent approval is absent, and no merge or deployment occurred. Calendar observed05:48: Nov13=445, Nov14=100; UTC/NY authored merge counts remain separate receipts.

Devlog pass: current contracts/indexes unchanged, history recorded only in this new immutable file. Prior0525/0531 entries remain unchanged. All pending local events await substantive work rather than a status-only PR or documentation-only reviewed-head advance.
