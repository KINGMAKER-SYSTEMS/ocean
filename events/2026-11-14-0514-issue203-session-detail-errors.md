time: [12:14AM America/New_York] [11-14-26]
agent: [Codex desktop] [factory builder, coordinator and independent reviewer]
worktree: [codex/factory-post201-handoff]
type: [bug report]
area: [backend]

Issue #203 arose during review of author-owned PR #199: the session detail endpoint mapped every runtime error to404, allowing a client to discard a valid session binding when storage was corrupt or unreadable. The handler now uses the existing optional read seam: Some keeps200 and existing projections/scope checks, None keeps404, and a runtime-reported error produces a fixed500 without storage details. No new API, schema, runtime discovery behavior or storage mutation is introduced. Existing candidate-discovery limitations remain separately tracked in #204.

Validation: six targeted daemon session_detail tests passed with locked dependencies, two Cargo jobs and debug/incremental artifacts disabled. The new registered-route fixtures cover absent404, repeated corruptJSON/non-file500 with retained bytes/directory, healthy session metadata, newest-first transcript and active turn, unscoped/matching200 and cross-scope400. No live providers or services were called. Independent final-head review and required CI follow this event; maintainer approval and deployment holds remain.

PR #199 exacthead d8d4918c received a separate independent HOLD review5901818769. Two standalone synthetic fixtures compiled against its actual Swift production sources reproduced reader loss at129 eligible rows (128 admitted, cursor129, next scan0) and same-member pending-ledger eviction. Accepted-turn cancellation after ledger-write failure is source-verified. Those adapter repairs belong to its author under #202; no adapter source edits were made. Package build and installer shell syntax passed; full Swift tests could not compile XCTest on the CommandLineTools-only host. Existing event-file modification and strict-contract adoption remain author review gates.

Devlog pass: updated only the daemon owning contract/verification. Root and parent/child indexes stay unchanged because this is a handler mapping correction within existing ownership. The pending0455 postmerge handoff event travels with this substantive repair unchanged. No existing event file was edited.

Capacity: the local targeted test completed, then free space was observed below the4GiB build floor. Hold further local heavy builds; use the existing hosted CI lane. Mini SSH remains unavailable and the remote checkpoint stale; this work makes no deployment or continuous-availability claim.
