time: [2:36pm EDT] [10-21-26]
agent: [Codex] [review129_runtime]
worktree: [codex/factory-desktop-integration]
type: [bug report]
area: [testing]

Exact-head 9a000fc7 agent5, daemon5 and legacy steering1 tests passed. UI test
compilation exposed a missing std::cell::Cell import in the new deterministic
recovery fixtures. Added that test-only import; runtime behavior and contracts
are unchanged. Driver stopped before projection tests/Trunk. Final-head UI
tests, real WASM bundle, and independent delta review remain pending. Nearest
Surface devlog was reviewed and left unchanged because this is fixture plumbing;
existing event history remains untouched. No deployment or live claim.
