time: [03:35am] [06-26-26]
agent: [codex cli] [gpt-5]
worktree: [main]
type: [plan]: Agent filesystem build list
area: [writing]: Architecture doc

Finished the `ocean-agents` package-side build inventory in
`docs/AGENT_FILESYSTEM_ARCHITECTURE.md`: split runtime foundation work from this
repo's manifest/profile/SOP/contract/verification builds and linked the
Python-to-Rust migration cutover. Verification passed:
`make assistants-check`, `python3 -m unittest discover -s assistants/bridge/tests -v`,
and `python3 couriers/hub/router.py list`.

time: [11:54am] [06-26-26]
agent: [codex cli] [gpt-5]
worktree: [main]
type: [plan]: Primitive-native status correction
area: [writing]: Architecture doc

Corrected the package-side build register and architecture mirror to state that
`ocean-agents` currently has zero primitive-native builds. Existing artifacts are
now labeled as filesystem conventions or transitional harnesses, with typed
primitive implementation/enforcement owned by `ocean-os`. Verified the TOML
register parses and asserts `primitive_native_builds = 0`; `make assistants-check`
still passes.

time: [12:07pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [main]
type: [workflow]: Ocean project map
area: [writing]: Cross-repo documentation

Added `docs/OCEAN_PROJECT_MAP.md` and linked it from the repo entrypoints so
agents can route references across `ocean-os`, `ocean-agents`, `ocean-surface`,
and `ocean-bedrock`. The map keeps runtime enforcement in `ocean-os`, agent
package/profile ownership in this repo, UI surfaces in `ocean-surface`, and
shared knowledge/data-plane ownership in `ocean-bedrock`.

time: [12:15pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [main]
type: [workflow]: Ocean project map
area: [writing]: Cross-repo documentation

Refined `docs/OCEAN_PROJECT_MAP.md` with a pairwise connection matrix so agents
see the four repos as one connected Ocean system. The update calls out this
repo's links to the runtime, surfaces, Bedrock context/handoffs, and all-four
workflow path.

time: [12:49pm] [06-26-26]
agent: [codex] [gpt-5]
worktree: [main]
type: [workflow]: Ocean project map
area: [design]: Cross-repo documentation

Linked the mirrored project map to the new animated cartography artifact at
`../../ocean-os/docs/OCEAN_PROJECT_MAP_ART.html`. The artifact visualizes the
four connected repos as an ocean chart with `ocean-agents` as the profiles,
couriers, SOPs, and tools island.

time: [05:28pm] [07-01-26]
agent: [claude] [fable 5]
worktree: [main]
type: [workflow]: Phase-0 stabilization
area: [infra]: Repo hygiene

Landed the uncommitted 06-26 session (AGENT_FILESYSTEM_ARCHITECTURE build
register, OCEAN_PROJECT_MAP, AGENTS-CHANNEL roster updates) in one docs commit
on main, and untracked `.cursor/hooks/state/continual-learning.json`
(IDE-generated state, now gitignored). Triaged stash@{0}: the mission brief
claimed it landed via commits 6b4c362/662f354/c589837, but those commits only
touch the CI workflow file — the stash's actual content (event streamer,
autoreply-channel gating, gallery-link delivery, video-gen skill wiring)
doesn't exist anywhere on main, so it was kept rather than dropped. Triaged 3
remote + 4 local branches: 4 provably shipped and deleted
(ci/ocean-328-test-runner-job, fix/reply-partial-canvas-keyerror,
fix/restore-ocean-agency remote+local, all byte-identical to main), 2 boarded
as real unlanded work (design/content-agent-slack-assistant,
wip/bonzai-content-local — the latter is the companion piece to the kept
stash). Full triage record at
`../ocean-discovery/08-branch-triage-ocean-agents.md`. `make assistants-check`
passes clean.

time: [07:31pm] [07-01-26]
agent: [claude] [fable 5]
worktree: [main]
type: [merge]: Phase-1 agents-docs-truth
area: [docs]: Spec recovery + README truth-sync

Landed the missing design authority: merged design/content-agent-slack-assistant
(docs-only, one commit) via --no-ff, so
docs/specs/2026-06-04-content-agent-slack-assistant-design.md now exists on main
and the assistants/README.md link plus every §3–§13 bridge citation finally
resolve — the spec had sat unmerged since 06-04 while everything built against
it shipped. Then truth-synced the two READMEs that were lying about repo state:
assistants/README.md and assistants/tools/README.md both still told operators
the compose-check CI workflow was parked at assistants/tools/ci/ awaiting a git
mv, but it has been live at .github/workflows/assistants-compose-check.yml since
OCEAN-327 (6b4c362); and the registry table still labeled content-agent
"scaffold (R5, OCEAN-80)" though the Slack bridge shipped end to end
(socket_listener OCEAN-112, reply, canvas_consumer OCEAN-244/375) — now "live
(Slack bridge; OCEAN-112/244/375)". Branch design/content-agent-slack-assistant
is fully merged and safe to delete. `make assistants-check` green; pushed to
origin/main.

time: [12:30am] [07-10-26]
agent: [claude] [ocean tui]
worktree: [main]
type: [plan]: Knowledge-first repositioning
area: [docs]: Build register + architecture doc

Committed the repositioning of ocean-agents as the deployment layer for Rising
Tides' agents (interactive assistants + workflow-invoked automation agents),
with baked-in company knowledge as the differentiator. Build register
(docs/ocean-agents-builds.toml, v2) gains a [positioning] block, five runtime
dependencies (R1 ocean-memory, R2 ingest bridge, R3 portable-db mount, R4 typed
agent traits, R5 workflow engine — all ocean-os), and three new package builds:
A11 bedrock-context-mounts (agents declare bedrock namespaces; runtime injects
at turn time), A12 automation-agent-retyping (courier framing retired; /ship
and /say demoted to tool grants; typed Contracts replace the Python router),
A13 knowledge-parity-gate (fixture suite proving each agent's company knowledge
is runtime-supplied, not prompt prose — required before deployment). Mirrored
into AGENT_FILESYSTEM_ARCHITECTURE.md §9.2. content-agent named the reference
package pattern. TOML parses (13 builds, 5 runtime deps); make assistants-check
green.

time: [7:42pm] [07-10-26]
agent: [claude] [opus]
worktree: [main]
type: [workflow]: Mirrored map update for daemon-owned voice routes
area: [docs]: Cross-repo documentation

Mirrored the voice phase 4 connection-contract change (2026-07-10) into
docs/OCEAN_PROJECT_MAP.md so the map stays byte-identical across the four
Ocean repos. The daemon now owns all voice provider calls: the core daemon
routes block gains POST /v1/voice/stt, POST /v1/voice/tts, and
POST /v1/voice/realtime/client-secret, with a note that the surface proxy's
/api/stt and /api/tts forward to the daemon and the browser Realtime client
mints its ephemeral secret via the daemon; provider keys resolve only inside
ocean-os.
_________________________________________________________________________________

time:      [07:10pm] [13-07-26]
agent:     [pi], [gpt-5.2-pro]
worktree:  [main]
type:      [bug report]: teach TUI assistants the live component projection contract
area:      [agent-building]: composed terminal surface profile

Replaced the obsolete blanket rich-component prohibition in the authored TUI profile with the live terminal projection contract. TUI assistants may now use callout, progress, stat, chart, timeline, table, code, diff, file tree, gallery, and confirm components with bounded props and correct lifecycle handling, while arbitrary HTML, maps, canvas layouts, and unsupported web forms remain excluded. Re-composed the published TUI system profile from its base sources.

Verification:
- `python3 assistants/tools/compose_profile.py --check TUI`
- `python3 -m py_compile assistants/tools/compose_profile.py`
- `git diff --check`
_________________________________________________________________________________
_________________________________________________________________________________

time:  [14:51] [14-07-26]
agent: [pi] [gpt-5]
worktree: [docs/current-state-reset-20260712]
type:  [refactor]
area:  [writing]

Reconciled the public courier inventory with the executable manifest router: ten courier
packages are registered and the stale nonexistent /ttup reference is removed. Added a
docs-local contract that makes manifests/router output authoritative, updated the root
child index, and removed the project map's dead artifact link and false strict-mirroring
claim.
_________________________________________________________________________________

time: [05:27am] [07-19-26]
agent: [ocean] [gpt-5]
worktree: [main]
type: [workflow]: Public/private boundary cleanup
area: [writing]: Cross-repo documentation

Updated the public entry points and local project map after `ocean-bedrock` moved
private. The repository now identifies its public package/tooling role, treats
Bedrock as an optional authenticated team service, removes assumptions that
public packages can access it, and points maintenance at ocean-os's canonical
public map. Verification passed: `make assistants-check`, live courier router
listing (20 slash entries), changed-file local-link validation, `git diff --check`,
and no direct public GitHub links to ocean-bedrock.

time:      [10:57pm] [07-19-26]
agent:     [ocean] [gpt-5]
worktree:  [security-settings-ledger-20260719]
type:      security
area:      GitHub repository settings

Enabled GitHub secret scanning and push protection for the public
`Risingtides-dev/ocean-agents` repository during the full-history public-readiness
audit. A checksum-verified Gitleaks 8.30.1 mirror scan across branches, tags,
GitHub PR refs, and recursive archives reported zero findings. No package,
profile, router, or generated assistant file changed.
