# Ocean Bedrock Epoch 01 — P0 Tracking Map

Generated: 2026-06-20T03:33:27Z
Owner: Navigator

## Source files

- Master workflow: `workflows/ocean-epoch-01-p0.workflow.ts`
- Per-agent workflows: `workflows/agents/*-p0.workflow.ts`
- Shared tracking policy: `workflows/agents/p0-tracking-policy.ts`
- NoteDash P0 runbook: `/root/notes/20260619-231701-ocean-bedrock-epoch-01-p0-clean-room-runbook.md`

## Linear project

- Team: `ocean-os` (`OCEAN`)
- Project: Ocean OS — System Stabilization

## Task mapping

| pi task | Agent | Linear | GitHub branch | Worktree | Completion packet |
| --- | --- | --- | --- | --- | --- |
| task-1 | Navigator / NAV | [OCEAN-353](https://linear.app/rising-tides-agents/issue/OCEAN-353/obe-p0-nav-clean-room-orchestration) | `epoch01/navigator-p0` | master for P0 observation | `workflows/runs/epoch01-p0/P0-MERGE-DECISION.md` |
| task-2 | Anchor / ANK | [OCEAN-354](https://linear.app/rising-tides-agents/issue/OCEAN-354/obe-p0-ank-source-of-truth-secret-review) | `epoch01/anchor-p0` | `.worktrees/epoch01-anchor` | `workflows/runs/epoch01-p0/ANCHOR-P0-REVIEW.md` |
| task-3 | Keel / KEL | [OCEAN-355](https://linear.app/rising-tides-agents/issue/OCEAN-355/obe-p0-kel-runtimelocal-state-review) | `epoch01/keel-p0` | `.worktrees/epoch01-keel` | `workflows/runs/epoch01-p0/KEEL-P0-REVIEW.md` |
| task-4 | Sonar / SNR | [OCEAN-356](https://linear.app/rising-tides-agents/issue/OCEAN-356/obe-p0-snr-docscontext-map) | `epoch01/sonar-p0` | `.worktrees/epoch01-sonar` | `workflows/runs/epoch01-p0/SONAR-P0-DOCS-MAP.md` |
| task-5 | Beacon / BCN | [OCEAN-357](https://linear.app/rising-tides-agents/issue/OCEAN-357/obe-p0-bcn-evidence-grade-loop) | `epoch01/beacon-p0` | `.worktrees/epoch01-beacon` | `workflows/runs/epoch01-p0/BEACON-P0-EVIDENCE-GRADES.md` |

## Review swap rounds

Defined in `workflows/agents/p0-tracking-policy.ts`:

1. R0 self-check
2. R1 domain peer
3. R2 cross-grade swap
4. R3 refinement loop for C/BLOCK
5. R4 Navigator merge gate

## GitHub / Actions gates

- No direct worker edits to master.
- No force-push/history rewrite.
- No GitHub Actions bypass.
- `.github/workflows/**` changes require Keel + human approval.
- Required before merge when code/scripts changed: `npm run smoke`.
- Required before merge when DB/runtime claims changed: `npm run db:check` or documented blocker.
