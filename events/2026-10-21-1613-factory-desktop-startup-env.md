time: [12:13 pm EDT] [21-10-26]
agent: [Codex] [factory coordinator]
worktree: [codex/factory-desktop-startup-env]
type: [bug report] [refactor]
area: [backend] [testing] [review]

Independent backlog reviews of PR122 and PR124 identified two startup defects
also present in desktop candidate63d8c027. Issues140/141 were created and read
back before this isolated implementation. Relative runtime state/auth/input
paths are anchored to the app's launch directory before the child changes to
its neutral working directory; absolute values and blank refusal sentinels
retain their meaning. Blank OCEAN_MODEL is removed independently of a saved
desktop selection so an existing default or persisted model can resolve.

Surface's owning devlog describes these contracts. Parent routing/indexes are
unchanged; root event-policy correction remains owned by frozen PR138. Legacy
events.md and previous entries are untouched. Added injected-environment path
and model-precedence regressions plus an isolated actual-child fixture using
only disposable state. Rustfmt and diff check passed. Focused native tests are
compiling on the Mac mini at record creation; final-head independent review and
hosted required checks remain pending. No real credentials, provider login,
service replacement, native acceptance or deployment is claimed. Candidate
integration and existing maintainer-approval holds remain separate gates.
