time: [11:19] [21-10-26]
agent: [Codex]
worktree: [codex/factory-candidate-review-repairs]
type: [bug report] [refactor]
area: [review] [frontend] [backend]

Independent review of desktop candidate PR129 at 63d8c0279a9658bb66f4bad24dc6f2d63ecf3f3e found two defects, recorded and read back as issues136 and137 and source-HOLD review5601930445.

The native availability callback now reconnects the focused session with a fresh authoritative projection at the new stream generation. It preserves local intent and retires the previous sync poll; focused regressions cover stale and replacement snapshots, retained queued input/images and permission/turn state, repeated reconnects and absent focus.

The MCP bridge no longer advertises or executes direct ocean_prompt turns. Stale calls refuse before daemon I/O, including supplied yolo or decision-token arguments. This removes the new caller's unsafe unbound permission path until a complete bounded submitter-owned permission flow is available. Existing Room tools remain unchanged. Regression coverage checks no contact and continued real-stdio health after refusal.

Validation before this commit: targeted source inspection, rustfmt and whitespace checks passed. The candidate's15 packaging fixtures and release-workflow contract fixtures passed independently on the unchanged packaging source. Focused Rust regressions are still compiling on the Mac mini; no test pass, required build pass or final-head approval is claimed here. Final independent review and both real required build checks must be recorded against this commit before release.

This isolated branch is based on the unchanged external candidate head and does not take custody of PR129. Its conflicts with canonical main, maintainer approval under54/90, signing/notarization, installation/recovery, provider replies and real shared-room acceptance remain separate holds. No live service, credentials, installed binary or external branch changed.

Devlog pass: updated Surface and MCP behavior contracts and onboarding claims. Root now records the user's immutable events/ policy; legacy events.md is untouched. Child ownership/index structure is unchanged.
