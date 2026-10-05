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

_________________________________________________________________________________
time: [13:18] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [frontend] [backend] [testing]

Terminal model/advisor catalogs now prefer qualified auth routes and current.route, retaining legacy catalog fallback and disconnected readiness. Session config decoding, scoped revisioned events and fenced snapshots retain provider identity. Alias-aware authority comparisons preserve legacy acknowledgements without treating API and subscription pins for the same wire model as equal. Existing generation, queue barrier, stale-save and revision guards remain exercised. Bare aliases normalize only when qualified choices are advertised; legacy requests stay bare. Footer presentation uses catalog labels.

Final TUI: 500 passed, four ignored; cargo check, strict all-target Clippy, docs-check and required release build passed. A real client HTTP fixture proves API-qualified PATCH and matching provider acknowledgement. A dispatch regression proves Codex authority cannot retire a pending OpenAI API pin for the same wire model, while matching API authority can. Existing model queue/revision regressions use canonical selection ids; legacy catalog/provider-less response compatibility remains covered. No inference, live installation or credential changes occurred. Owning TUI devlog updated; root and Child devlog indexes stay unchanged because package boundaries did not move. Prior ACP head 934c7b2e has both hosted builds green. Fresh review, new-head CI, connected-account inference and runtime compatibility remain pending. VS Code composer still exposes separate model/effort controls and lacks max; that integration remains active goal work.
_________________________________________________________________________________

_________________________________________________________________________________
time: [10:24] [05-10-26]
agent: [Claude Code] [Claude Opus 5.5]
worktree: [fix/gpt-5-5-served-context]
type: [fix]
area: [providers]

Ported Risingtides-dev/ocean-os#531 (b7839a45) onto PR #2: gpt-5.5 (both
spellings) now resolves with the Codex backend's served 272k context window
instead of the advertised 400k, so a session cannot overfill and fail.
Regression test gpt_5_5_resolves_with_codex_backend_served_limit added.
Validation: cargo test -p ocean-providers --locked (see PR).

time: [13:28] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [feature-request]
area: [frontend] [design] [testing]

VS Code/Cursor now uses one model/effort disclosure in the composer instead of separate permanent runtime-bar selects. ACP forwards optional namespaced readiness and effort-level metadata; the extension preserves qualified ids, disables explicitly disconnected choices and uses model-specific effort lists. Max is supported by settings, command picker and prompt metadata. Explicit model changes clear incompatible effort; missing/pinned choices stay represented. Escape closes the disclosure and restores focus, outside clicks close it, and the top status no longer repeats the selected model id.

Extension TypeScript lint and production package passed after installing locked dependencies into the task checkout. JS syntax check passed. ACP: 23 unit plus three permission-ordering integration tests passed; strict all-target Clippy and docs-check passed. Read-only renderer fixture using actual HTML/CSS/JS proves API/max display, disabled disconnected option, incompatible max clearing after model change, Escape/focus, missing pin preservation and 390px viewport bounds x=12..378 with scroll width 390. Fixture evidence does not prove a native editor host or account inference. Owning Surface/crates devlogs updated; root and Child devlog indexes unchanged because no ownership boundaries changed. Prior head 3c4a1320 has hosted CI green. Fresh review, new-head CI, native/live acceptance, account repair and runtime migration compatibility remain pending; no live installation or provider call occurred.
_________________________________________________________________________________

_________________________________________________________________________________
time: [13:48] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [backend] [testing]

Fixed missing native Claude Code login discovery. Explicit env and valid Ocean OAuth credentials retain precedence; read-only native macOS keychain discovery is bounded to two seconds/32 KiB, with native credential-file fallback. Custom CLAUDE_CONFIG_DIR bypasses default account discovery. Future expiry and inference scope are required; native secrets are never imported, refreshed, written or logged, and cannot supply Anthropic API auth.

Validation: 65 provider tests, strict all-target provider Clippy, six OAuth storage tests, seven Agent refresh tests, workspace test compilation and docs-check pass. Readiness-only candidate snapshot now reports 39/44 routes (Claude Code 4/4; Anthropic API 0/4 and Google 0/1). All four Claude Code choices completed bounded live no-tools prompts through production resolver/factory: Fable 5.1, Opus 5.5, Sonnet 5.5 and Haiku 4.5. Existing other-account acceptance failures remain unresolved. No live installation or durable service/database changes. Provider devlog updated; parent contracts and child indexes unchanged because ownership boundaries are unchanged. Fresh review and operated-runtime compatibility remain required before release.
_________________________________________________________________________________

_________________________________________________________________________________
time: [13:53] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [frontend] [testing]

Restored Surface compatibility with the operated daemon caller projection. Installed revision 6912315 exposes self_member_id from its private credential row; the candidate used caller_member_id and dropped that identity, incorrectly disabling Share for otherwise eligible live owners. Surface now accepts the equivalent legacy wire field as a read alias and serializes the current field only. Conflicting duplicate fields fail decoding; live human-owner membership checks remain unchanged. The shared projection decoder covers HTTP hydration and room_access SSE.

Validation: all 897 Surface tests, strict WASM Clippy and docs-check pass. Added a production-decoder-to-Share-policy regression for legacy owner, ordinary member, mismatched caller, duplicate identity and current-only serialization. Surface owning contract updated; parent/index docs unchanged because ownership boundaries did not change. No live room, service, database or installation mutation. This repairs one client compatibility issue and does not lift the daemon migration hold or establish cross-machine Rooms acceptance.
_________________________________________________________________________________

_________________________________________________________________________________
time: [13:59] [05-10-26]
agent: [codex] [gpt-6]
worktree: [feat/model-effort-control] [/Users/risingtidesdev/dev/ocean-ui-current]
type: [bug report]
area: [frontend] [testing]

Corrected shared model/effort compatibility fallback. A saved provider-qualified route now derives legacy effort choices from its wire model when capability metadata is absent, preventing always-thinking GPT/Claude routes from offering Off/Minimal. Explicit empty daemon effort metadata remains authoritative instead of repopulating generic levels. Max is still metadata-only, and pinned stored values are preserved.

Validation: all 899 Surface tests, strict WASM Clippy and docs-check pass. New regressions cover qualified saved API/subscription routes, missing-vs-empty metadata and legacy Max exclusion. The actual editor renderer with a synthetic host adapter also confirms Max resets to Default when switching to a model supporting only Low/High, and returns to API/Max correctly; this is renderer evidence, not native editor/account acceptance. Screenshot: /private/tmp/ocean-model-effort-verified.png. Owning Surface contract updated; root/index docs unchanged because ownership boundaries remain unchanged. No installation, live provider calls or credential mutations in this slice. Fresh review, other-account acceptance and operated-runtime compatibility remain unresolved.
_________________________________________________________________________________

_________________________________________________________________________________
time: [16:59] [05-10-26]
agent: [claude] [claude code]
worktree: [claude/subagent-lost-run-recovery] [/Users/risingtidesdev/dev/ocean-claude-audit]
type: [bug report]
area: [backend] [testing]

Repaired ocean-subagents lifecycle recovery. The daemon's request registry is in memory, so a run whose request disappeared (daemon restart, or a finished turn evicted after an hour) stayed active forever; four such runs exhausted the concurrency cap and blocked spawn for every session. An untracked active run now settles as terminal lost from session truth after a short grace, a cancel the daemon answers ok:false settles from daemon truth instead of parking in cancelling, and the elapsed-time watchdog retries because plugins start before the listener binds. The recursion guard now fails closed: spawn and send first confirm the worker profile resolves with a non-empty allowlist that omits subagent tools, since an unresolved named agent or empty allowlist runs with every tool. Also: optional per-child thinking_level, catalog guidance on an unroutable model, wait returns when a permission prompt appears, unchanged polls no longer rewrite and fsync state, finished runs are retained to 200, state updates are bound to the turn they were read for, and --check no longer starts watchdogs. README now states that the daemon launches plugins with a cleared environment, so the documented environment variables apply only to hand-started plugins.

Validation: 20 unit tests against a test daemon corrected to the real control contract (volatile registry, HTTP 200 ok:false cancel refusals, cancelling before cancelled, 404 for an unknown session); four of the new tests fail against the previous plugin with the reported symptoms. Wire test passes on Python 3.13 and on system Python 3.9 under a cleared environment; py_compile on both; sh -n; docs-check. The new read-only preflight and catalog reads were exercised against the operated daemon with a temporary state directory; no child was spawned and no live state or installation changed. The installed plugin copy is unchanged until install.sh is rerun and the daemon restarted. Plugin devlog and README updated; parent indexes unchanged.
_________________________________________________________________________________

_________________________________________________________________________________
time: [17:40] [05-10-26]
agent: [claude] [claude code]
worktree: [claude/subagent-lost-run-recovery] [/Users/risingtidesdev/dev/ocean-claude-audit]
type: [review]
area: [backend] [testing]

Applied the independent review of the ocean-subagents repair. The recursion guard still passed an allowlist that matched no existing tool (the daemon then keeps every tool) and ignored subprocess capabilities, whose tools are added after narrowing; it now requires an always-present built-in tool and no subprocess capability. thinking_level accepts max. Output is the latest turn's text only, so a lost run no longer reports an earlier turn's answer. A run settles even when its session cannot be read, and a settled run with no output re-reads it later; a completed run whose session read failed used to stay active. A request list that is not a list is an error instead of settling every run as lost. wait reports each permission prompt once by id, including one raised between waits. send counts against the concurrency cap. The watchdog survives malformed responses, is re-armed by refresh if it gave up, does not re-cancel a run already cancelling, and the elapsed-time reason survives a later lost settlement. The unroutable-model hint now points at a new spawn because send reuses the model.

Validation: 27 unit tests pass; ten mutations of the new logic are each killed by a test, including the three the review found surviving. The prune test now uses distinct finish times out of insertion order, and the retry test no longer races. Wire test on Python 3.13 and system 3.9 under a cleared environment, py_compile, sh -n and docs-check pass. The revised preflight passes against the operated daemon read-only. Plugin devlog and README updated.
_________________________________________________________________________________
