time: [12:30] [21-10-26]
agent: [Codex] [factory builder]
worktree: [codex/factory-account-readiness]
type: [bug report] [refactor]
area: [frontend] [testing]

Issue #144 was created and read back before this isolated repair on candidate
63d8c027. Opening the existing model disclosure refreshes daemon-owned readiness
so browser login completed after Accounts closes can become selectable. Stable
model option identities now react to readiness and label updates. Shared model
request generations and endpoint identity checks retire stale replies without
changing local drafts, overrides or effort. Accounts polling and login authority
are unchanged; no extra controls, credential handling or automatic selection.

Focused regressions cover closed-dialog login refresh ordering, retained local
intent, endpoint retirement and same-id option readiness changes. Surface's
owning devlog records the contract; parent routing/indexes are unchanged.
Rustfmt and diff check pass. Targeted compile/tests and independent final-head
review remain pending. No real login, provider inference, rendered native
acceptance or deployment is claimed.
