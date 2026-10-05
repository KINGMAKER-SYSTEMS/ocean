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
