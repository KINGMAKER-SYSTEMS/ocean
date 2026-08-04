# Ocean Bedrock Epoch 01 — P0 Docs Map

**Created:** 2026-06-20T03:38:29Z
**Owner:** Sonar (SNR)
**Worktree:** `.worktrees/epoch01-sonar`
**Branch:** `epoch01/sonar-p0`
**Task:** `OCEAN-356` / `task-4`

## Canonical Docs Map

### Tier 0 — Repo entry and live-state surface

| Path | Role | Authority |
| --- | --- | --- |
| `README.md` | Repo/service overview, links hub, config summary | Top-level entrypoint |
| `docs/HANDOFF.md` | Live-state handoff and deployment facts | Current operational snapshot |

### Tier 1 — Canonical identity and boundaries

| Path | Role | Authority |
| --- | --- | --- |
| `docs/OCEAN-BEDROCK.md` | Bedrock glossary / boundary note | Canonical repo identity note |
| `docs/OCEAN-LONGHOUSE.md` | Longhouse architecture and source-of-truth split | Canonical architecture reference (external dependency to `ocean-os`) |
| `docs/OCEAN-LEDGER.md` | Ledger / temporal memory model | Canonical ledger model |
| `docs/OCEAN-CONTEXT.md` | Retrieval / semantic / graph layer | Canonical context model |
| `docs/OCEAN-LONGHOUSE-DATA-PLANE.md` | Data-plane implementation direction | Canonical data-plane plan |

### Tier 2 — Interfaces and operator flows

| Path | Role | Authority |
| --- | --- | --- |
| `docs/API.md` | HTTP API reference | Canonical API guide |
| `docs/openapi.yaml` | Machine-readable HTTP contract | API source of truth for tooling |
| `docs/MCP.md` | MCP server wrapper doc | Agent/tooling interface guide |
| `docs/COWORKER-BOOTSTRAP.md` | Coworker ingest/bootstrap flow | Operator onboarding guide |
| `docs/LOCAL-GUI-APP.md` | Local sync companion app | UI onboarding guide |
| `docs/OCEAN-TOOLBOX-BOOTSTRAP.md` | Toolbox bootstrap flow | Operator tooling guide |
| `docs/OCEAN-TRIAGE-OPS.md` | Daily triage runbook | Ops/runbook authority |
| `docs/WEEK-ONE-RUNBOOK.md` | Implementation/runbook checklist | Transitional runbook |
| `docs/OCEAN-BEDROCK-WEEK-ONE-WORKFLOW.md` | Phase plan and launch workflow | Execution plan / historical roadmap |

### Tier 3 — Crew identity and coordination

| Path | Role | Authority |
| --- | --- | --- |
| `docs/RT-NAS-CREW-BOOT.md` | Project-scoped crew boot notes | Crew startup guide |
| `docs/OCEAN-BEDROCK-AGENT-ROSTER.md` | Human-readable agent roster | Crew identity source |
| `docs/ocean-bedrock-agent-roster.json` | Machine-readable roster | Automation-friendly roster |
| `docs/agents/*.md` | Per-agent lane docs | Agent-specific instructions |

### Tier 4 — Source adapters and provenance

| Path | Role | Authority |
| --- | --- | --- |
| `docs/SOURCE-RUNNERS.md` | Source runner behavior | Adapter implementation guide |
| `docs/SOURCE-ADAPTER-PRECEDENTS.md` | Prior art / examples | Reference material |

### Tier 5 — Historical / snapshot docs

| Path | Role | Authority |
| --- | --- | --- |
| `docs/WIKI.md` | Generated docs index / live tree snapshot | Historical snapshot, not authoritative |
| `docs/DEV-LOG.md` | Implementation log / change history | Historical record, not canonical |

## Stale or Conflicting Claims

1. **Stale current-session identity**
   - `docs/ocean-bedrock-agent-roster.json` → `currentSession.agent = "Navigator"`, `callsign = "NAV"`.
   - This is stale for the live P0 run: the active session here is Sonar.
   - Treat `currentSession` as a boot snapshot, not live truth.

2. **Failed-job count contradiction**
   - `docs/HANDOFF.md` → section `What is live` says `ingest jobs failed: 9`.
   - `docs/OCEAN-TRIAGE-OPS.md` → section `Disposition of the 9 historical failed rows` says those 9 rows were already completed/cancelled and the next triage reported `failedJobs=0`.
   - These claims cannot both be current. One of them is stale.

3. **Tree snapshot drift**
   - `docs/WIKI.md` → section `What is currently on the shared filesystem` includes `/docs/README.md`.
   - That file is not present in the current repo docs inventory gathered for this pass.
   - The WIKI tree should be treated as a snapshot, not as live repo inventory.

4. **Conceptual bucket naming vs live mounts**
   - `docs/OCEAN-BEDROCK.md` → section `Practical meaning for this repo` mentions an `ocean-longhouse` R2 bucket and `OCEAN_LONGHOUSE_CACHE` KV namespace.
   - `docs/HANDOFF.md` → live mount list shows different active R2 prefixes/buckets and no `ocean-longhouse` mount.
   - This reads as conceptual/legacy naming and should be clarified before it is used as an implementation claim.

5. **Historical docs carrying live-state language**
   - `docs/DEV-LOG.md` and `docs/WIKI.md` both include time-sensitive counts/snapshots.
   - They are useful history, but they should not be used as current source-of-truth when they disagree with `HANDOFF.md` or the triage runbook.

## Recommended Doc Patch Set

Atomic follow-ups only; no canonical rewrites in P0.

1. Add a freshness/authority note to `docs/WIKI.md` and `docs/DEV-LOG.md` so they are clearly snapshot/history docs.
2. Resolve the failed-job count mismatch between `docs/HANDOFF.md` and `docs/OCEAN-TRIAGE-OPS.md`.
3. Update or remove the stale `currentSession` field in `docs/ocean-bedrock-agent-roster.json`.
4. Clarify whether `docs/OCEAN-BEDROCK.md` is describing a conceptual bucket/namespace or an actually deployed mount.
5. Keep `README.md` as the top-level index and use `docs/HANDOFF.md` for live state only.

## Grade

**B**

## Blocks

- None for this packet.
- P0 constraint respected: no canonical docs were rewritten.
