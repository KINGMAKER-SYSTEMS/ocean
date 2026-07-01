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
