# ANCHOR-P0-REVIEW — Source-of-Truth & Secret Safety Review

**Owner:** Anchor — ANK (sourceOfTruth)
**Task:** task-2 / [OCEAN-354](https://linear.app/rising-tides-agents/issue/OCEAN-354/obe-p0-ank-source-of-truth-secret-review)
**Workflow:** `workflows/agents/anchor-p0.workflow.ts` (mode: `read-only-review`)
**Worktree:** `.worktrees/epoch01-anchor` on branch `epoch01/anchor-p0` (createFrom HEAD `414b7f9`)
**Created:** 2026-06-20T03:50:00Z
**Mode:** Read-only review. No product/docs files edited. No migrations, deploys, or token files touched. Only this packet was written.

---

## Checked Inputs

Read-only inputs consumed (per `anchor-p0.workflow.ts` `readOnlyInputs`):

| Input | Source | Status |
|---|---|---|
| `workflows/runs/epoch01-p0/P0-SNAPSHOT.md` | worktree | ✅ read |
| `workflows/runs/epoch01-p0/P0-FILE-INVENTORY.md` | worktree | ✅ read |
| `workflows/runs/epoch01-p0/P0-CLASSIFICATION-PROPOSAL.md` | worktree | ✅ read |
| `workflows/runs/epoch01-p0/P0-MERGE-DECISION.md` | worktree | ✅ read (context) |
| `workflows/runs/epoch01-p0/P0-TRACKING-MAP.md` | worktree | ✅ read (context) |
| `README.md` | worktree (tracked) + master working-tree diff | ✅ read + diff |
| `docs/HANDOFF.md` | worktree (tracked) | ✅ read |
| `docs/RT-NAS-CREW-BOOT.md` | master working tree (untracked candidate) | ✅ read |
| `docs/ocean-bedrock-agent-roster.json` | master working tree (untracked candidate) | ✅ read |
| `docs/OCEAN-BEDROCK-AGENT-ROSTER.md` | master working tree (untracked candidate) | ✅ read |
| `docs/agents/{ANK,NAV,KEL,SNR,BCN}.md` | master working tree (untracked candidates) | ✅ read |
| `scripts/rt-nas-crew-launch.sh` | master working tree (untracked candidate) | ✅ read |
| `workflows/ocean-epoch-01-p0.workflow.ts` | worktree (copied candidate) | ✅ read |
| `workflows/agents/{anchor,p0-common,p0-tracking-policy}.ts` | worktree (copied candidates) | ✅ read |

**Scope note (important):** The worktree was created from HEAD `414b7f9`, so it contains only *tracked* files plus the workflow/run artifacts that were copied in. The untracked candidate-commit files (`docs/OCEAN-BEDROCK-AGENT-ROSTER.md`, `docs/ocean-bedrock-agent-roster.json`, `docs/RT-NAS-CREW-BOOT.md`, `docs/agents/*.md`, `scripts/rt-nas-crew-launch.sh`, and the `README.md` modification) are **not present in the worktree** — they live only in master's working tree. Anchor completed the scan by reading those files in master's working tree **read-only** (no edit/stage/commit). This is a P0 process gap — see Source-of-Truth Findings #3. All reads stayed inside `/root/rt-nas/rt-nas`.

---

## Secret Scan Result

**Verdict: PASS — secret-block NOT triggered.**

### Method (ANK-P0.1)

1. Broad keyword pass (`api_key|token|secret|private key|DATABASE_URL|BEGIN .*PRIVATE`) over `README.md docs scripts workflows` — produced only keyword hits, reviewed for context.
2. **Value-pattern pass** (filenames only, no values printed) over all candidate commit files + `README.md` + `docs/HANDOFF.md`, targeting real credential *values*: OpenAI `sk-…`, Slack `xox…`, GitHub `gh[pousr]_…`, AWS `AKIA…`, `-----BEGIN … PRIVATE KEY-----`, DB URLs of form `postgres(ql)?://user:pass@host`, Telegram-style `<digits>:<token>`. **Result: `NO_VALUE_PATTERN_MATCHES`.**

### Findings

- **No raw credential, token, private key, password, or live DATABASE_URL value** appears in any candidate commit file.
- Keyword matches in the broad pass are all benign: shell/env **variable names** (e.g. `OCEAN_BEDROCK_TOKEN`, `DATABASE_URL` in config tables), **`<secret>` placeholders** in `docs/HANDOFF.md` (e.g. `OCEAN_BEDROCK_BOOTSTRAP_TOKEN=<secret>`, `CLOUDFLARE_API_TOKEN=<secret — has R2 object read/write/delete>`, `DATABASE_URL=<Railway Postgres URL — internal only>`), **policy/scan-pattern text** inside the workflow files (e.g. the `grep -RInE "(api[_-]?key|token|secret|…)"` string in `anchor-p0.workflow.ts` and `p0-common.ts` `secretBlock` hook), and prose about secrets.
- The only token-like **literal** is `dev-token-change-me` in `README.md` and `docs/MCP.md` — a documented **dev/example bootstrap placeholder**, explicitly flagged in README safety notes ("Do not expose this publicly with a weak bootstrap token"). Obviously fake, not a live credential.
- Token **file paths** are referenced (`/root/.config/ocean-bedrock/*.txt`, `/data/.ocean-bedrock/tokens.json`) but **no token values**. Anchor did **not** open or read any token file (out of scope + forbidden by workflow).
- `scripts/rt-nas-crew-launch.sh` contains no secrets; it sets a default model string, tmux session name, and log paths, all scope-bound to `/root/rt-nas/rt-nas`.

### Recommendation (non-blocking)

Add a pre-commit secret scanner (e.g. `gitleaks`) and a `.gitignore` guard for `/root/.config/ocean-bedrock/` and `data/.ocean-bedrock/tokens.json` to prevent future leaks. Not required for this merge — current packet is clean.

---

## Source-of-Truth Findings

Core authority claims are **consistent** across `README.md`, `docs/HANDOFF.md`, the roster, and the agent lane docs. No data-authority violation, no false claim that `/vault` is encrypted, no claim that the token registry is Postgres-backed.

### Consistent (verified)

| Claim | README | HANDOFF | Verdict |
|---|---|---|---|
| Token registry is **file-backed JSON** (`…/.ocean-bedrock/tokens.json`), not Postgres | config table (`OCEAN_BEDROCK_AUTH_FILE`) | §4 + §13 #2 ("volume JSON… not multi-replica safe") | ✅ consistent |
| Token files under `/root/.config/ocean-bedrock/` are gitignored, never commit/log | safety notes | §2 #9 | ✅ consistent |
| Token roles: `readonly`, `contributor`, `agent`/`readwrite`, `admin` with path scopes | §V1 model | §4 contributor scopes | ✅ consistent |
| `/vault` is a **placeholder, not an encrypted vault**; avoid raw secrets there | default folders + §Agent convention #6 | §13 #3 | ✅ consistent — no candidate file falsely claims encryption |
| Postgres ledger + source registry + sync-run lineage live when `DATABASE_URL` set | §Postgres readiness | §6 schema tables, §10 source API | ✅ consistent |
| R2 mounts via Cloudflare REST API using existing `CLOUDFLARE_API_TOKEN` (no S3 creds) | (deferred to HANDOFF/API) | §5 | ✅ consistent |
| Agent lanes (NAV/ANK/KEL/SNR/BCN scope, role, approval boundaries) | — | §2 guardrails | ✅ consistent; agent docs do not contradict HANDOFF |
| `test-nonexistent.txt` = junk | — | — | ✅ 0 bytes, **no tracked references** (`git grep` clean) |

### Defects / follow-ups (non-blocking)

**1. Branch-name mismatch in `workflows/ocean-epoch-01-p0.workflow.ts` `worktreePolicy.worktreesAfterP0`.**
   The top-level epoch workflow declares branch names that contradict the operative per-agent workflows, the tracking map, **and the actual git branches**:
   | Agent | `worktreePolicy.worktreesAfterP0` (stale) | Actual / per-agent workflow / tracking map |
   |---|---|---|
   | Anchor | `epoch01/anchor-persistence` | `epoch01/anchor-p0` |
   | Keel | `epoch01/keel-runtime` | `epoch01/keel-p0` |
   | Sonar | `epoch01/sonar-context-docs` | `epoch01/sonar-p0` |
   | Beacon | `epoch01/beacon-evidence` | `epoch01/beacon-p0` |
   Paths are correct; only branch names diverge. The per-agent workflows + `P0-TRACKING-MAP.md` + `git rev-parse` all agree on the `-p0` names, so the operative contracts are correct. Recommend reconciling the `worktreePolicy.worktreesAfterP0` branch names to `epoch01/<agent>-p0` (or marking that section superseded by the per-agent workflows) so the canonical epoch workflow does not commit contradictory branch names.

**2. Roster hardcodes "currentSession: Navigator".**
   Both `docs/OCEAN-BEDROCK-AGENT-ROSTER.md` ("Current session identity") and `docs/ocean-bedrock-agent-roster.json` (`currentSession`) pin the session to Navigator. This is a point-in-time authoring artifact that will go stale whenever another agent session reads the canonical roster. Recommend relabeling as "roster authored from Navigator session" or dropping the `currentSession` field from the canonical file. Not a safety issue.

**3. Worktree does not contain candidate commit files (process gap).**
   Per-agent worktrees were created from HEAD and only the workflow/run artifacts were copied in. The untracked candidate-commit docs/scripts were not copied, so a reviewer working solely inside the worktree cannot see or scan them. Anchor worked around this by reading master's working tree read-only. Recommend either (a) copying candidate-commit files into each review worktree, or (b) explicitly documenting in the P0 workflow that reviewers must read master's working tree for untracked candidates. This affected scan completeness only until the master read was performed; final scan is complete and clean.

**4. Roster lists 14 agents; P0 boots 5.** Not a defect — roster is the forward-looking target team, P0 launches the core 5 (NAV/ANK/KEL/SNR/BCN). `scripts/rt-nas-crew-launch.sh` boots exactly those 5. Consistent. No action.

---

## Recommended Classification Changes

Anchor agrees with Navigator's `P0-CLASSIFICATION-PROPOSAL.md` / `P0-MERGE-DECISION.md` patch set, with the following source-of-truth annotations:

| # | Path | Proposed | Anchor position |
|---|---|---|---|
| 1 | `test-nonexistent.txt` | delete | ✅ **Concur** — 0 bytes, no tracked refs. ⚠️ The `rm` **action** must still pass the **data-loss-block** gate (Anchor + Beacon + human) per `p0-common.ts`. Anchor's source-of-truth assessment: deletion is safe. |
| 2 | `.wrangler/` | ignore (gitignore) | ✅ **No objection** — local Cloudflare emulation SQLite, not token registry / DB / deploy config. (Keel owns final runtime call.) |
| 3–18 | `docs/**`, `workflows/**`, `scripts/rt-nas-crew-launch.sh` | commit | ✅ **Concur** — canonical crew docs + declarative workflow contracts. No secrets. |
| 19 | `README.md` (modified) | inspect → commit | ✅ **Concur** — diff is exactly 2 lines adding cross-links to `OCEAN-BEDROCK-AGENT-ROSTER.md` and `RT-NAS-CREW-BOOT.md`. Accurate, intentional, safe. |

**Additional recommendation (not a classification change):** Before or with the commit, reconcile the `worktreePolicy.worktreesAfterP0` branch names in `workflows/ocean-epoch-01-p0.workflow.ts` (Finding #1) so the canonical workflow does not enshrine contradictory branch names. If left for a later patch, record it as a tracked follow-up.

---

## Grade

Per Anchor's `peerReview` rubric (`p0Workflow.peerReview[0]`): A = no source-of-truth/storage/auth/secret concern; B = safe to proceed with documented follow-up; C = needs revision before merge; BLOCK = unsafe.

| Dimension | Grade | Justification |
|---|---|---|
| **P0 source-of-truth** | **B** | Core authority claims (token registry file-backed, `/vault` placeholder-not-encrypted, DB schema, R2 mounts, token scopes, agent lanes) are consistent across README/HANDOFF/roster/agent-docs. Non-blocking follow-ups recorded: branch-name mismatch in epoch workflow (Finding #1), roster `currentSession` staleness (Finding #2), worktree process gap (Finding #3). None create data-authority violation or merge-blocking ambiguity — operative contracts are correct. |
| **Secret safety** | **A** | No raw credential values in any candidate commit file. Only documented example placeholder (`dev-token-change-me`) and `<secret>` markers. Token paths referenced, no values. secret-block not triggered. |

**No C or BLOCK from Anchor.** Review packet is merge-eligible from the source-of-truth and secret-safety lane pending Navigator's merge gate and the documented follow-ups.

---

## Blocks

| Hook | Status | Detail |
|---|---|---|
| 🔒 secret-block | ✅ Clear | No raw secrets found. Value-pattern scan clean. |
| 🔒 scope-block | ✅ Clear | All work inside `/root/rt-nas/rt-nas`. Master working-tree reads were read-only; no edits/staging/commits. Did not touch `/root/.config/ocean-bedrock/` or any other mesh dir. |
| 🔒 deploy-risk-block | ✅ Clear | Candidate files have no deploy/env/routing impact. No deploy commands run. |
| 🔒 data-loss-block | ⚠️ Conditional (on action, not on review) | The `rm test-nonexistent.txt` **action** requires Anchor + Beacon + human approval before execution. Anchor assessment: source-of-truth-safe (0 bytes, no refs). Beacon + human still required per policy. The review itself triggers no data loss. |
| 🔄 refinement-loop | ✅ Not triggered by Anchor | Anchor grades are A and B (no C/BLOCK). |

**Kanban note (not a content block):** `pi_messenger task-2` is currently in **blocked** state with reason "Max attempts (3) reached" — a lobby-worker retry artifact (FastEagle/LoudYak/MintQuartz exited code 1 before Navigator manually assigned Anchor). This is **not** a content, safety, or refinement block. Navigator/human should reset task-2 to reflect that Anchor has produced this packet. Anchor did not mark the task done (it is not todo/in_progress and per policy the task is not done until Beacon/Navigator accept evidence and peer grade is A/B).

---

## Peer review handoff

Per `p0-tracking-policy.ts` review-swap rounds:
- **R1 domain-peer:** Anchor → reviewed by **Keel** for runtime impact of source/storage claims.
- **R2 cross-grade-swap:** Anchor → reviewed by **Sonar** for doc/claim clarity.

Anchor stands ready to perform its R1/R2 reviewer duties for Keel (persistence/secret implications of runtime claims), Sonar (source-of-truth correctness), Beacon (safety gates + source-of-truth accuracy), and Navigator (merge packet) when those packets are produced.
