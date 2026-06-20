# Ocean Bedrock Epoch 01 — P0 Classification Proposal

**Created:** 2026-06-20T03:34:17Z
**Owner:** Navigator (NAV) — ZenDragon
**Status:** Proposal (awaiting Anchor, Keel, Sonar, Beacon peer review)
**Block hooks:** secret-block, scope-block, deploy-risk-block, data-loss-block

---

## Classification Reference

| Classification | Meaning |
|----------------|---------|
| **commit** | Canonical project artifact. Safe to commit. |
| **ignore** | Local/generated tool state. Add to .gitignore. |
| **delete** | Junk or accidental test artifact. Remove from working tree. |
| **defer** | Needs human decision. Hold until approval. |
| **inspect** | Possible secret/high-risk/unverified content. Review before classification. |

---

## Proposed: commit (18 paths)

### docs/ — Canonical crew documentation

| # | Path | Rationale |
|---|------|-----------|
| 1 | docs/OCEAN-BEDROCK-AGENT-ROSTER.md | Canonical crew roster — declares agent names, roles, and responsible lanes across the Ocean Bedrock system. Cross-referenced by README.md. |
| 2 | docs/RT-NAS-CREW-BOOT.md | Project-scoped crew boot/run instructions. Describes how agents discover and join the mesh. Cross-referenced by README.md. |
| 3 | docs/ocean-bedrock-agent-roster.json | Machine-readable agent roster JSON. Complements the Markdown roster for automated consumption. |
| 4-8 | docs/agents/ANK.md, BCN.md, KEL.md, NAV.md, SNR.md | Individual agent lane instructions — one per P0 agent. Required for crew operation and coordination. Each declares scope, role, expertise, default responsibilities, and approval boundaries. |

### scripts/ — Launch helper

| # | Path | Rationale |
|---|------|-----------|
| 9 | scripts/rt-nas-crew-launch.sh | Crew launch helper script. Starts the agent panes and initializes the crew mesh. |

### workflows/ — P0 agent workflow contracts

| # | Path | Rationale |
|---|------|-----------|
| 10 | workflows/ocean-epoch-01-p0.workflow.ts | Epoch 01 P0 clean room workflow — full TypeScript declaration of agent lanes, tasks, block hooks, peer review rules, worktree policy, and merge policy. |
| 11 | workflows/agents/navigator-p0.workflow.ts | Navigator's P0 agent workflow contract. Steps: snapshots, worktrees, collect reviews, merge decision. |
| 12 | workflows/agents/anchor-p0.workflow.ts | Anchor's P0 agent workflow contract. Secret/safety scan, source-of-truth review. |
| 13 | workflows/agents/keel-p0.workflow.ts | Keel's P0 agent workflow contract. Runtime/local-state review, .wrangler/ classification. |
| 14 | workflows/agents/sonar-p0.workflow.ts | Sonar's P0 agent workflow contract. Docs hierarchy, glossary consistency. |
| 15 | workflows/agents/beacon-p0.workflow.ts | Beacon's P0 agent workflow contract. Evidence collection, review grading. |
| 16 | workflows/agents/p0-common.ts | Shared P0 types and block hooks. Imported by all agent workflows. |
| 17 | workflows/agents/p0-tracking-policy.ts | External tracking policy for GitHub, Linear, and pi-messenger kanban. |
| 18 | workflows/runs/epoch01-p0/P0-TRACKING-MAP.md | Tracking map linking Linear issue (OCEAN-353), GitHub branch (epoch01/navigator-p0), and workflow references. |

---

## Proposed: commit (conditional) — Modified file

| # | Path | Proposed | Rationale |
|---|------|----------|-----------|
| 19 | README.md | commit (after inspect) | Modification adds 2 lines cross-linking new canonical docs. **Action required:** Human or Anchor (ANK) should review the diff to confirm the links are accurate and intentional before committing. |

---

## Proposed: ignore — Local Cloudflare state

| # | Path | Proposed | Rationale |
|---|------|----------|-----------|
| 20 | .wrangler/ | ignore | Local Cloudflare Workers emulation SQLite databases. Developer tool state only. **Action required:** Keel (KEL) should confirm .wrangler/ has no deploy-critical config. If confirmed, add to .gitignore. |

---

## Proposed: delete — Junk artifact

| # | Path | Proposed | Rationale |
|---|------|----------|-----------|
| 21 | test-nonexistent.txt | delete | Likely accidental test artifact. No references in any tracked file. No documented purpose. **⚠️ Data-loss block:** Confirm with Anchor + Beacon that no external process depends on this file before deleting. |

---

## Proposed Hygiene Patch Set

### If all classifications are accepted and peer grades are A/B:

```
# Phase 1 — Delete junk
rm test-nonexistent.txt

# Phase 2 — Add ignore rule
echo "\n# Local Cloudflare emulation state\n.wrangler/" >> .gitignore

# Phase 3 — Stage canonical docs, scripts, workflows
git add docs/OCEAN-BEDROCK-AGENT-ROSTER.md
git add docs/RT-NAS-CREW-BOOT.md
git add docs/ocean-bedrock-agent-roster.json
git add docs/agents/
git add scripts/rt-nas-crew-launch.sh
git add workflows/ocean-epoch-01-p0.workflow.ts
git add workflows/agents/
git add workflows/runs/epoch01-p0/P0-TRACKING-MAP.md

# Phase 4 — Stage README.md after human/inspect approval
git add README.md

# Phase 5 — Commit with explicit message
git commit -m "feat(P0): commit canonical crew roster, agent workflows, and tracking map

- Add OCEAN-BEDROCK-AGENT-ROSTER.md with agent lane declarations
- Add RT-NAS-CREW-BOOT.md with crew operating instructions
- Add agent lane docs (ANK, BCN, KEL, NAV, SNR)
- Add machine-readable agent roster JSON
- Add rt-nas-crew-launch.sh launch helper
- Add P0 agent workflow contracts (all 5 agents + common + tracking)
- Add P0 epoch-01 workflow declaration
- Add P0-TRACKING-MAP.md for external tracking
- Remove test-nonexistent.txt (junk artifact)
- Add .wrangler/ to .gitignore (local Cloudflare state)
- Update README.md with links to canonical crew docs

Task: OBE-P0 NAV — Clean room orchestration"
```

---

## Items Requiring Peer Review

| Item | Reviewer | Question |
|------|----------|----------|
| .wrangler/ classification (ignore) | Keel (KEL) | Is `.wrangler/` purely local state, or should any part be preserved for deployment reproducibility? |
| README.md diff inspection | Anchor (ANK) | Does the README modification introduce accurate claims about canonical docs? Is it safe to commit? |
| Agent lane docs consistency | Sonar (SNR) | Are the agent lane docs (docs/agents/*.md) consistent with the working note glossary? Do they preserve Ocean Longhouse/Bedrock boundaries? |
| test-nonexistent.txt deletion safety | Anchor (ANK) + Beacon (BCN) | Confirm no external process depends on this file before deletion. |
| Secret scan of commit candidates | Anchor (ANK) | Scan all commit-classified paths for raw credentials, tokens, or env dumps before approving merge. |

---

## Block Hook Status

| Hook | Status | Notes |
|------|--------|-------|
| 🔒 secret-block | ✅ Scan pending | Formal scan needed from Anchor before commit. No raw secrets spotted in first pass. |
| 🔒 scope-block | ✅ Clear | All proposed operations strictly within `/root/rt-nas/rt-nas`. |
| 🔒 deploy-risk-block | ✅ Clear | No deploy commands, env changes, or production routing affected by this patch set. |
| 🔒 data-loss-block | ⚠️ Conditional | `test-nonexistent.txt` delete is low risk (clearly junk), but requires Anchor + Beacon approval per P0 policy. |
| 🔄 refinement-loop | 📋 Pending | Will trigger if any peer assigns C or BLOCK grade. |

---

## Escalation Path

If any peer assigns C or BLOCK:

1. Create block note with specific failing criterion.
2. Revise only the blocked slice (e.g., reclassify .wrangler/ from ignore to defer).
3. Rerun the same reviewer who issued the block.
4. Do not advance dependent tasks until grade is A/B or human explicitly waives.

---

## Proposed Merge Order (per P0 merge policy)

```
1. Documentation-only: docs/*, workflows/* (type-only TS)
2. Scripts/helpers: scripts/rt-nas-crew-launch.sh
3. Junk removal: test-nonexistent.txt (delete)
4. .gitignore update: .wrangler/ (ignore)
```
