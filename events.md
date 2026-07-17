time: [12:07pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [master]
type: [workflow]: Ocean project map
area: [writing]: Cross-repo documentation

Added the Ocean project map and repo entrypoint contract for the current
`/Users/risingtidesdev/dev/ocean-bedrock` checkout. The map routes agents across
`ocean-os`, `ocean-agents`, `ocean-surface`, and `ocean-bedrock`, and the new
root `AGENTS.md` makes the map discoverable before durable Bedrock edits.

time: [12:15pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [master]
type: [workflow]: Ocean project map
area: [writing]: Cross-repo documentation

Refined `docs/OCEAN_PROJECT_MAP.md` with a pairwise connection matrix so Bedrock
is documented as the shared knowledge/data plane connected to runtime, surfaces,
and agent packages, not as a standalone repo. The map now names the normal
all-four workflow path.

time: [12:49pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [master]
type: [workflow]: Ocean project map
area: [design]: Cross-repo documentation

Linked the mirrored project map to the new animated cartography artifact at
`../../ocean-os/docs/OCEAN_PROJECT_MAP_ART.html`. The artifact visualizes the
four connected repos as an ocean chart with `ocean-bedrock` as the shared docs,
context, ledger, graph, and API island.

time: [5:28pm] [07-01-26]
agent: [claude] [fable 5]
worktree: [master]
type: [workflow]: Phase-0 stabilization
area: [infra/docs]: Handoff refresh, doc-drift check, branch cleanup

Ran Phase-0 stabilization on ocean-bedrock. Landed the cross-repo project map,
AGENTS.md contract, and events ledger (previously untracked) alongside a
one-line README pointer, in a single docs-only commit (5158312) verified
against 27/27 passing tests both before and after. Appended a dated update
section to docs/HANDOFF.md covering everything since the 2026-06-17 snapshot:
the first test suite (27/27 across 5 files, commits #3-#6), the P0
intermediate-symlink path-traversal fix in src/mounts/adapters/local.mjs
(#8, 4 regression tests), and the CLI token-issuance audit-trail fix in
scripts/issue-token.mjs (#7) -- each claim checked against git log and the
actual test/source files before writing, not copied from the brief. Checked
the suspected MCP tool-count drift and found docs/MCP.md and docs/WIKI.md
already correctly list all 14 tools exported by scripts/ocean-bedrock-mcp.mjs;
the real drift was HANDOFF.md's stale "10 tools" claim, corrected by this
same update section. (toolboxManifest() in src/ocean-context.mjs still
hardcodes an 11-tool list missing 3 names -- left as-is, code is out of scope
for this docs-only pass, flagged as a known follow-up.) Triaged all 6 remote
branches using a squash-merge-aware method (diff each branch's own commits
against master's matching squash-merge commit, not raw --merged/ancestor
checks, since this repo's branches are squash-merged and left dangling);
all 6 were provably shipped and were deleted from origin. Full triage
detail recorded at
../ocean-discovery/08-branch-triage-ocean-bedrock.md.

time: [7:38pm] [07-01-26]
agent: [claude] [fable 5]
worktree: [feat/binary-doc-extraction]
type: [feature-request]: Binary-doc extraction
area: [backend]: Ingest pipeline

Closed the test-pinned PDF/DOCX silent-skip gap: new src/extract.mjs
(extractableBinary extension gate + extractText via unpdf/mammoth, chosen over
pdf-parse's 33MB unmaintained bundle) wired into processIngestJob so
extractable binaries flow through the same chunk -> embed -> graph pipeline as
text files, with unparseable/empty documents failing the job visibly in
ocean:jobs instead of skipping. Flipped the KNOWN-GAP pin to assert pdf/docx
pass the gate and narrowed it to a new pin for pptx/xlsx/rtf. Added
scripts/ocean-binary-backfill.mjs (npm run ocean:backfill:binary, --dry-run
/--limit, dedupes queued jobs) for pre-extractor uploads — code-verified only,
no live DB locally. Same PR syncs toolboxManifest() in src/ocean-context.mjs
from 11 to the 14 tools the MCP script exports (was missing
bedrock_toolbox_manifest/lock/unlock), pinned by a new grep-based set-equality
test. 37/37 tests, smoke/secret/source checks green. PR open against master,
not merged; ledger entry rides the PR branch to avoid touching master.
_________________________________________________________________________________

time: [7:45pm] [07-01-26]
agent: [claude] [fable 5]
type: [merge]: Phase-1 binary extractor landed
area: [backend]: Ingest pipeline

Squash-merged PR #9 (feat/binary-doc-extraction) into master as 1d4f502 and
deleted the branch. Review verdict was approve; this repo has no CI workflow
gating PRs (gh pr checks reported none), so the gate was the review plus a
green local suite. The phase-1 extractor is now live on master: src/extract.mjs
(unpdf/mammoth) wired into processIngestJob so PDF/DOCX flow through
chunk -> embed -> graph instead of silently skipping, plus the binary backfill
script and the 14-tool toolboxManifest sync. Post-merge npm test on master:
37/37 passing.
_________________________________________________________________________________

time: [07:07pm] [07-07-26]
agent: [codex] [gpt-5.5]
worktree: [master]
type: [plan]: Skill-driven onboarding ceremony
area: [design]: Coworker onboarding

Wrote `docs/superpowers/plans/2026-07-07-ocean-skill-onboarding-ceremony.md`, locking the coworker onboarding reframe around a standardized skill as conductor: any skill-capable agent can open the Ocean local site, drive setup through localhost calls, stream ceremony events, and perform a safe one-time import of Claude Code, Codex, and Pi configs. The plan defines the V1 ceremony event schema, REST + WebSocket protocol, high-level page-tool events, import safety rules, and three independent build lanes for local app protocol endpoints, Leptos/WASM stage UI, and the onboarding skill rewrite.
_________________________________________________________________________________

time: [07:29pm] [07-07-26]
agent: [codex] [gpt-5.5]
worktree: [master]
type: [feature-request]: Skill-driven onboarding ceremony
area: [frontend/backend]: Coworker onboarding

Implemented the Ocean skill-driven onboarding ceremony V1. Added local-only ceremony run/event REST endpoints and WebSocket fan-out to `scripts/ocean-local-app.mjs`; added a protocol check script; created a Leptos 0.8 CSR/WASM ceremony stage under `ui/onboarding-ceremony` with oceanic globe, real event rendering, reconnect/replay, and reduced-motion fallback; rewrote/created the standardized onboarding skill plus fixture/replay scripts; added license-safe local monogram placeholders for Claude, Codex, and Pi. Verified node syntax, ceremony protocol, ack gate, fixture dry-run, wasm cargo check, Trunk release build, smoke, source check, secret scan, and secret glob check.
_________________________________________________________________________________

time: [2:53pm] [07-07-26]
agent: [omp] [glm-5.2]
worktree: [master]
type: [refactor]: Ocean onboarding clean rebuild -- wave 2 cleanup
area: [frontend/infra]: Coworker onboarding

Clean-rebuild wave 2, cleanup lane. Deleted the entire diverged Leptos/WASM/Trunk ceremony app at `ui/onboarding-ceremony/` and stripped the dead slop from `scripts/ocean-local-app.mjs`: the inline `INDEX_HTML` button-wall admin panel (and its emoji-laden markup), the `/admin` route, the `serveCeremony` handler plus its `ceremonyDistPath` const, and the fixture-replay demo driver (`DEMO_RUN_ID` / `buildDemoRun` / `playDemoWalkthrough`) along with the now-unused `buildFixtureFrames` import. Kept the ceremony REST + WebSocket protocol spine intact (POST `/api/ceremony/runs`, `/events`, `/complete`; GET `/api/ceremony/runs/:id`; `/api/ceremony/ws`) and the `/` route serving the new `ui/onboarding/index.html` shell, plus `serveStatic` / `sendStatic` / `STATIC_TYPES`. Verified `node --check`, then restarted the app: GET `/` -> 200 serving the new `Ocean` page (467-byte shell loading `ocean-three.js`, emoji-free), GET `/admin` -> 404, GET `/ceremony` -> 404, GET `/api/state` -> 200. All four repo gates pass: smoke, source:check, secret:scan (109 files), secret:globs:check. Sibling lanes in flight this wave: three.js centerpiece (landed), anime.js frontend (`ui/onboarding/*`), real-scan skill (`scripts/ocean-ceremony-scan.mjs` + `skills/.../SKILL.md`).
_________________________________________________________________________________

time: [12:45am] [07-08-26]
agent: [claude] [fable-5]
worktree: [master]
type: [feature-request]: Onboarding E2E + design system landing
area: [frontend/backend]: Coworker onboarding

Orchestrated the final onboarding wave: three parallel lanes (scan E2E, orb
packaging, design tokens/UI) landed their code, then verification and repair
ran here. Scanner fixes: de-duplicated true source counts (Claude 98 / Codex
631 skills, was capped at 50), `POST /api/sync` now gated on a confirmed human
ack (timeout can no longer fire a production sync), and timeout runs drop the
ack/sync narration frames instead of lying. Restored the operator app at `/`
(`ui/app/index.html`, rebuilt; the old inline GUI was deleted uncommitted) and
moved the ceremony to `/onboarding` after the orb wrongly owned root. Freed
port 8790 (ocean-surface-proxy) after a test instance squatted it; test
instances now use 18xxx. Canonical design system at `design/` (tokens,
self-hosted Inter, principles, README, OceanOrb module); onboarding page wired
via import map + tokens link. Live E2E: real scan drove the stage with zero
console errors, orb intensity 28 = 12+4x4 sources; gates green (smoke,
source:check, secret:scan, secret:globs:check, ceremony protocol, node --check,
no emoji).
_________________________________________________________________________________

time: [07:33pm] [07-10-26]
agent: [omp] [glm-5.2]
worktree: [master]
type: [workflow]: Mirrored map update for daemon-owned voice routes
area: [docs]: Cross-repo documentation

Mirrored the daemon-owned voice contract (voice phase 4, 2026-07-10) into docs/OCEAN_PROJECT_MAP.md so the four Ocean repos stay byte-identical. The core daemon routes block now lists POST /v1/voice/stt, /v1/voice/tts, and POST /v1/voice/realtime/client-secret, followed by a phase-4 note that the surface proxy forwards /api/stt and /api/tts to the daemon and that provider keys (xAI, OpenAI) resolve only inside ocean-os. Docs-only change; map kept in sync across repos.
_________________________________________________________________________________
_________________________________________________________________________________

time:  [14:42] [14-07-26]
agent: [pi] [gpt-5]
worktree: [docs/current-state-reset-20260712]
type:  [refactor]
area:  [writing]

Reset docs/HANDOFF.md from an accumulated June snapshot plus override sections into a
concise verified current-state handoff. Added a docs-local devlog contract, clarified the
root child index, and removed the dead project-map artifact link and false strict-mirroring
claim. Verification from a clean checkout passed 37 tests, smoke, source contract, secret
scan, secret-glob checks, and public health/route-index requests.
_________________________________________________________________________________
time:  [18:37] [16-07-26]
agent: [claude] [fable 5]
worktree: [feat/s1a-federated-rooms]
type:  [feature-request]
area:  [backend]

Built Gate-2 S1A federated room membership end to end against the codex-frozen
v4 contract: 006 migration (room_members, composite same-principal FK, partial
uniques, deferred owner-guard trigger), rooms.mjs store with transactional
provenance/tombstone/removal locks, server routes R-1..R-7 + room-owner invite
branch + R-8 ledger gating with global-read suppression, quadruple-gated
disposable-PG protocol harness, smoke/db:check/package updates, openapi+API
docs. Verification: npm test 61/61, npm run smoke green, reviewer-executed
harness 94/0 on a second fresh disposable PG14 _test database. Store gate
approved at b9f9df0, route/R-8 gate through fb9e24f (mismatch-order fix), PG gate through e4cb564;
ocean independent trace APPROVE with no findings. Repo fact only — nothing
here is deployed or production-attested. Final merge pending docs/branch
verification closeout.
_________________________________________________________________________________
time:  [19:06] [16-07-26]
agent: [claude] [fable 5]
worktree: [feat/s1b-producer-cursor]
type:  [feature-request]
area:  [backend]

Gate-2 S1B built against the v2 freeze (codex's rejection pins superseding
v1): full-normalized producer-content comparator (actor/schema/context deltas
conflict; only chain/re-stamp fields excluded), typed LedgerConflictError 409
compared before COMMIT, STRING-strict canonical-decimal u64 source_sequence
for ^room: sources and after_sequence cursor (JSON numbers rejected),
validation pre-I/O in both stores, ::numeric cursor bind, JSONL parity, server
409/400 mapping + validated query param, PG smoke S1B section, openapi/API/
devlog docs. npm test 72/72, smoke green. Earlier premature master commit
f3704d8 was reset off master (origin untouched at 2e3079b) and reworked here.
Awaiting codex slice gate; no push until his merge verdict.
_________________________________________________________________________________
time:  [19:15] [16-07-26]
agent: [codex] [gpt-5]
worktree: [master]
type:  [handoff]
area:  [review]

Reviewed and merged Gate-2 S1B into master, including the empty-string
pre-I/O regression and a bounded-page assertion in the real Postgres cursor
walk. Final merge is 1f20784; rejected v1 SHA f3704d8 remains preserved at
review/s1b-v1-rejected-f3704d8. Independent closeout passed npm test 72/72,
smoke, source check, secret and secret-glob scans, syntax/diff/YAML checks,
and the create-only rooms Postgres protocol harness 105/0 on an isolated
temporary PostgreSQL 14 database. Refreshed docs/HANDOFF.md from the stale
pre-merge S1A snapshot. Repository evidence only; no deployment or
production-state claim.
_________________________________________________________________________________
time:  [20:26] [16-07-26]
agent: [claude] [fable 5]
worktree: [feat/s1c-room-events]
type:  [feature-request]
area:  [backend]

Gate-2 S1C built and gate-approved at 3fdacac: authenticated durable SSE room
fanout per the approved v3 freeze — protocol module with injected server-owned
gates, single serialized writer, string-strict cursor precedence, high-water
snapshot then single forward-poll, full-gate heartbeat reauth with
revoked-vs-transport split, bounded queue with reachable overflow, tracked
teardown. Reviewer-run disposable-PG harness 122/122 after a first-run
overflow hang was made deterministic (held-read cutover, single-statement
16KiB bulk seed, bounded drain). npm test 90/90, smoke green with seven F3
routes. This entry lands with the docs/closeout child; branch unmerged, not
deployed, no push until codex's final merge gate.
_________________________________________________________________________________
time:  [20:30] [16-07-26]
agent: [claude] [fable 5]
worktree: [feat/s1c-room-events]
type:  [docs]
area:  [docs]

Correction to the 20:26 entry (append-only; the prior entry stands as
written): "single forward-poll" there means ONE immediate live read from
max(cursor, H) followed by the CONTINUING 1s±100ms forward-poll loop — one
loop, not one finite read. Also recording the completed frozen-matrix
db:check: reviewer-executed green against the same restarted disposable PG14
_test database used for the 122/122 harness run (schemaReady,
authRegistryReady, roomMembersReady, owner-guard trigger all true, 8 adapters
seeded), then stopped; no live database touched.
_________________________________________________________________________________
