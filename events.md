# Ocean events

_________________________________________________________________________________
time: [10:38] [05-10-26]
agent: [Codex desktop] [GPT-6]
worktree: [main] [/private/tmp/ocean-public-cutover]
type: [workflow]
area: [automations] [writing]

Prepared the clean public Kingmaker Ocean snapshot, preserving component licenses
and excluding private services, production agents, research, deployment records,
historical ledgers, and mixed-repository ancestry. Retained all four maintainers
and made Kingmaker Ocean canonical. Included changed-component development-profile
CI; six scope tests and actionlint passed, and the selected Ocean build passed.
Credential scan findings were reviewed as synthetic test fixtures and prose.

_________________________________________________________________________________
time: [10:44] [05-10-26]
agent: [Codex desktop] [GPT-6]
worktree: [codex/record-public-cutover]
type: [workflow]
area: [automations] [writing]

Verified live cutover: KINGMAKER-SYSTEMS/ocean is public with a parentless
publication snapshot; ocean-private is private with default private-main and
27 original open PRs preserved. Risingtides-dev, ecfromthedc, jakebalik-bit and
jayvespertine have admin access to both. Public main retains strict Build Ocean
and Build Surface checks, enforced admins, no force pushes/deletions, and writes
restricted to those four users. Private Actions are disabled. Public build
37326823597 started both hosted build jobs without the former billing rejection;
completion was still pending at this receipt. Scan of public history found only
seven reviewed synthetic test/prose matches. docs-check passed for 30 packages,
151 active Markdown files and 170 links. Component devlogs not affected by the
split retain their source contracts; no runtime behavior changed.

_________________________________________________________________________________
time: [10:46] [05-10-26]
agent: [Codex desktop] [GPT-6]
worktree: [codex/record-public-cutover]
type: [gh actions]
area: [automations] [testing]

First public Ocean hosted build passed. A docs-only PR exposed the trailing empty
entry in NUL-terminated Git output being classified as a root build input. Scope
now ignores empty entries; a regression test exercises actual main output for
docs, Ocean-only, and Surface-only diffs. Seven tests and actionlint passed.

_________________________________________________________________________________
time: [11:13] [05-10-26] America/New_York
agent: [Codex desktop] [GPT-6]
worktree: [feat/model-effort-control] /Users/risingtidesdev/dev/ocean-ui-current
type: [feature-request]
area: [frontend] [backend] [testing]

Implemented one composer model/effort disclosure with shared daemon catalog,
credential-readiness disabling, pinned-id preservation, model-specific effort
choices, Escape focus recovery and compact viewport positioning. Updated Codex
catalog/routing for Sol 6.1, Sol 6 and Luna 6 alongside Astra; Opus aliases now
track 5.5 while explicit older ids remain routable. Current adaptive Claude
requests use adaptive thinking and output_config effort; budget-based legacy
models retain their limits. GPT-6/5.6 xhigh is preserved rather than reduced.

Validated native Surface tests (864 unit plus 30 integration), WASM check,
Trunk bundle and actual desktop/390px browser interaction. Provider/protocol
suites (55 provider, 168 protocol plus 5 integration) and strict all-target
Clippy pass. Strict Surface Clippy still fails on two pre-existing unused
Rooms helpers, reproduced on unchanged main. Read-only live catalog reports
24 credential-ready entries across 7 routes; Claude Code and Google are not
ready. These are configuration receipts, not live inference or entitlement.

Devlog pass updates Surface and provider/protocol contracts plus crate catalog
guidance; root and child indexes retain unchanged boundaries. Existing PRODUCT
and design docs intentionally remain unchanged. Only selected public component
diffs were ported onto public main 0b7e20f1; no private history is published.
Goal remains active: independent review, required builds, full auth/model live
acceptance, max effort support, wider UI/integration pass and deployment remain.
_________________________________________________________________________________

_________________________________________________________________________________
time: [11:22] [05-10-26]
agent: [Codex desktop] [GPT-6]
worktree: [codex/source-tip-audit]
type: [workflow]
area: [research] [writing]

Verified merged CI selector repair and green public main Build scope, Build Ocean,
and Build Surface. Updated all three personal public source-repository descriptions
and homepage links to designate KINGMAKER-SYSTEMS/ocean as shared development home.
Compared source tips: runtime b7839a45 (298 source commits after import), Surface
1b88f862 (176), reusable packages f21d05f6 unchanged. Content-only merge preview
found 56 conflicting paths; no preview source or ancestry was published. Recorded
the reconciliation boundary and retained concurrent model/UI PR #2 separately.
Devlog pass updated owning docs contract and migration guide; component contracts
remain unchanged because no component source was edited.

_________________________________________________________________________________
time: [11:34] [05-10-26]
agent: [Codex desktop] [GPT-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [refactor] [bug report]
area: [frontend] [backend] [testing]

Extended public PR #2 with daemon-owned effort choices and additive max support,
current Sonnet 5.5 routing/constructors and its between_tools Off wire, and missing
Opus 5.5 runtime construction. Every catalog id now passes selection through wire
construction with matching limits. Authored agent validation accepts production
routes rather than current picker membership, preserving older pinned models.
Surface persists max and honors daemon metadata while retaining older-daemon
fallbacks; TUI explicit selection, cycling and footer carry max.
Validation: Ocean workspace 3,330 tests passed (9 ignored), Surface 895 tests
passed; final workspace test compilation, provider suite (57), five focused TUI
thinking tests, strict providers/protocol/agent/TUI Clippy, Trunk bundle and TUI
release build passed. Existing two Surface dead-code warnings remain. Devlog pass
updated Surface and affected crate contracts; indexes/roots retain ownership.
No live deployment or inference claimed: live daemon 6912315aff3d remains selected,
Claude/Google readiness is false, and public runtime reconciliation plus fresh
review are required before installation. API-auth modern-model coverage and the
remaining broad UI integration/acceptance goal remain active.
