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

_________________________________________________________________________________
time: [11:51] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [frontend] [testing]

PR #2 review repairs separate the authoritative model default from executed turn models and let Escape bubble from a closed picker. A reducer regression proves override turns preserve the default; rebuilt-browser checks prove open-picker close/focus and closed-picker Sessions dismissal. Surface: 896 tests passed; Trunk built. Agent/protocol: 431 tests passed, one opt-in live diagnostic ignored; strict all-target Clippy passed.

Added an explicit bounded no-tools catalog inference probe with fixed prompts, no session/store/auth writes, and fixed error classifications. Actual account audit: 12 passed, 16 failed, five disconnected. Astra, GPT-5.6 Sol/Terra/Luna, GPT-5.5 and all seven GLM models completed inference; other configured routes returned 401/400/429. Codex version header now matches installed CLI 0.154.0; focused retry confirms GPT-6.1 Sol remains model_unavailable while Astra succeeds. Credential presence does not establish entitlement. Fresh review, current-head CI, modern direct API coverage and operated-runtime compatibility remain required; no live installation occurred.
_________________________________________________________________________________

_________________________________________________________________________________
time: [12:06] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [backend] [testing]

Direct OpenAI API-key GPT-6/5.6 now uses Responses for tool calling, with shared request/SSE collection, preserved same-route encrypted reasoning, and no Codex-specific originator/version/account/session headers. Explicit API routing retains 1,050,000/128,000 limits; explicit Anthropic/Claude Code Fable 5.1, Opus 5.5 and Sonnet 5.5 retain 1M/128K. Bare subscription ids and older Chat Completions routes remain unchanged. Local HTTP/SSE fixtures verify the actual endpoint, flat function tools, headers, effort and streamed completion; history tests retain own reasoning and drop foreign route items.

Provider/agent/protocol final verification: 493 tests passed, opt-in live probe ignored, strict all-target Clippy passed; final workspace check includes test targets. Workspace sweep before the final Claude-limit correction: 3,334 passed, ten ignored; corrected limits passed final owner checks. Explicit-provider probe requires exact model ids and reports fixed stream-error classes. Real OpenAI API Sol/Astra attempts did not complete; focused Sol retry classifies a quota rejection. Prior pushed head 45fcbd54 Build Ocean/Build Surface passed. No billing/auth edits or live deployment occurred. Owning devlogs updated; parent ownership/indexes unchanged. Separate auth-route picker exposure, connection repair, fresh review and live compatibility/acceptance remain in the full goal.
_________________________________________________________________________________

_________________________________________________________________________________
time: [12:28] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [feature-request]
area: [frontend] [backend] [testing]

PR #2 exposes 44 provider-qualified model choices, including separate OpenAI API/Codex and Anthropic API/Claude Code routes. Qualified selections override ambient provider pins; legacy catalog fields and saved bare-id aliases remain compatible. Readiness follows each auth route. Session creation/config persist wire model/provider pairs and resumed turns reconstruct the selected auth route rather than silently returning to subscription auth.

Final Ocean workspace: 3,340 passed, ten ignored; strict providers/agent/daemon all-target Clippy passed. An unrelated registry subprocess UUID read failed in the first parallel sweep; the complete final rerun passed. Workspace test compilation and docs-check passed. Surface: 897 passed and Trunk built. Isolated read-only browser fixture verifies distinct auth labels, disabled disconnected choices, API plus max selection and reload retention; it provides renderer evidence, not account inference acceptance. No new live model calls, credentials/billing writes or installation occurred.

Owning Surface/providers/agent/daemon docs and shared catalog index updated. Root ownership and Child devlog indexes stay unchanged because no boundaries moved. Fresh review and new-head hosted builds remain required; runtime migration compatibility, connection repair and broader UI acceptance remain active.
_________________________________________________________________________________

_________________________________________________________________________________
time: [12:45] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [refactor]
area: [frontend] [design] [testing]

PR #2 replaces the empty-session full-pane animated banner and duplicate Sessions action with the existing static circular mark. Unavailable voice renders no disabled setup placeholder. Removed the unused WebGL hero module/features/styles, retired two hero-only tests, retained Sessions chrome tests, and removed two unreferenced agent-picker helpers that caused Surface's existing strict-Clippy failures. Pending-response indicators and accepted Rooms behavior remain intact.

A preview exposed Rooms mounting before proxy bootstrap resolved the daemon URL. The new endpoint-ready signal gates its mount/fetch until proxy config or the host fallback is resolved. An eight-second delayed-config browser fixture proves Rooms stays unmounted before resolution and then fetches the same-origin fixture. Desktop and 390px previews prove the quiet start and combined picker; compact viewport/scroll width are both 390 and the panel stays within x=30..360. These are fixture checks, not live acceptance.

Final Surface: 895 tests passed (two retired hero tests removed), strict WASM Clippy and Trunk build passed; docs-check passed. Initial build failed for disk space; clearing this task's disposable Ocean incremental compiler cache recovered space, and final checks reran successfully. No user data or live service changed. Surface devlog/design guidance updated; root and Child devlog indexes stay unchanged because ownership boundaries did not move. Prior auth-route head a5ac9279 has both hosted builds green. Fresh review, new-head CI, account repair, runtime compatibility and broader UI/live acceptance remain required.
_________________________________________________________________________________

_________________________________________________________________________________
time: [12:59] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [feature-request]
area: [frontend] [design] [testing]

Floating chat now uses the same model/effort control as the main composer. Drafts remain editable during streaming; submission requires nonblank input, an idle turn and resolved endpoint in both handler and button. Rejected submissions preserve drafts. Compact layout anchors the picker to the composer, uses touch-sized controls and Ocean browser background while preserving native overlay transparency through the existing host seam.

Surface: 896 tests passed, strict WASM Clippy and Trunk build passed. Docs-check passed (30 packages, 151 active Markdown files, 170 links). Read-only browser fixtures prove Escape/focus, 390px layout with panel x=24..366 and no horizontal overflow, and Enter preserving a draft while delayed bootstrap keeps Send disabled. Fixture evidence does not establish real inference or native overlay acceptance. Owning Surface devlog updated; root and Child devlog indexes unchanged because ownership boundaries did not move. Fresh review, current-head CI, account repair and operated-runtime compatibility remain pending; no live installation occurred.
_________________________________________________________________________________

_________________________________________________________________________________
time: [13:06] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [frontend] [backend] [testing]

ACP editor model modes now consume additive qualified auth routes and current.route instead of collapsing API and subscription choices into legacy wire ids. Selection remains session-local and is forwarded unchanged on each turn. Older daemons retain legacy behavior. Explicitly disconnected alternatives are omitted because ACP modes have no disabled field; disconnected or retired current ids remain representable.

ACP: 23 unit and three permission-ordering integration tests passed; strict all-target Clippy and docs-check passed. A local HTTP fixture drives the real models client, mode projection, session override and turn client, proving OpenAI API qualified selection plus uppercase MAX metadata reaches the request as exact route plus max. No provider inference, credential change or live installation occurred. Crates owning devlog updated; root and Child devlog indexes intentionally unchanged because no package boundary changed. Preceding head 1c946d0d has both hosted builds green. Fresh review, new-head builds, connected-account acceptance and runtime migration compatibility remain pending.
_________________________________________________________________________________
