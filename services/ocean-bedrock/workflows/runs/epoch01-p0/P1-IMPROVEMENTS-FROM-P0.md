# P1 Improvements From P0 Findings

Generated: 2026-06-20T04:03:00Z
Owner: Navigator

P0 succeeded, but exposed process and documentation improvements that should be baked into P1 before broader execution.

## Process improvements

### 1. Manual tracking only for named-agent worktree waves

Finding: pi-messenger `plan` caused lobby workers to auto-attempt tasks even though P0 was designed for manual named-agent worktree execution.

P1 rule:

- Use pi-messenger as kanban/tracking only unless explicitly launching generic crew workers.
- For named agents, create tasks manually or from a prebuilt plan, then require each named agent to `task.start` their own task.
- Do not call `work` or autonomous mode for named-agent worktree waves.
- If auto-lobby starts, stop/reset immediately and record it as a process block.

### 2. Candidate files must be available to review worktrees

Finding: worktrees created from HEAD did not contain untracked candidate files, so Anchor had to scan master read-only.

P1 rule:

- Before assigning reviewers, create a review packet that includes either:
  - patch/diff files for all untracked and modified candidates, or
  - a copied candidate bundle inside each worktree.
- Reviewers should not need to inspect master working tree unless explicitly instructed.

### 3. Branch names must be declared once

Finding: top-level P0 workflow had stale post-P0 branch names while per-agent workflows and tracking map had the real `epoch01/*-p0` branches.

P1 rule:

- Keep branch/worktree mapping in one shared policy file and import/reference it from per-agent workflows.
- Treat duplicated branch names as a review blocker.

## Documentation follow-ups

### 4. Roster currentSession should not be canonical live truth

Finding: roster JSON hardcoded `currentSession: Navigator`, which becomes stale for other sessions.

P1 action:

- Change `currentSession` to `authoredFromSession` or remove from canonical roster.
- Make live identity come from pi-messenger status, not roster docs.

### 5. Historical docs need freshness labels

Finding: `docs/WIKI.md` and `docs/DEV-LOG.md` include time-sensitive snapshots/counts.

P1 action:

- Add freshness/snapshot notes to historical docs.
- Resolve failed-job count mismatch between `docs/HANDOFF.md` and `docs/OCEAN-TRIAGE-OPS.md`.

### 6. Conceptual vs live infrastructure names need clarity

Finding: `docs/OCEAN-BEDROCK.md` mentions conceptual `ocean-longhouse` bucket/KV names while `docs/HANDOFF.md` lists current live mounts.

P1 action:

- Clarify conceptual examples vs deployed mounts.
- Keep live infrastructure facts in `docs/HANDOFF.md`.

## Safety follow-ups

### 7. Secret scan should become repeatable tooling

Finding: Anchor did manual pattern scans successfully.

P1 action:

- Add a repeatable secret-scan command or script.
- Consider gitleaks/pre-commit guard later.

### 8. Cleanup actions need explicit approval breadcrumbs

Finding: deleting `test-nonexistent.txt` was safe but still correctly triggered the data-loss gate.

P1 rule:

- Even obvious junk deletion should record: path, size, refs check, reviewer approval, human approval.

## Carry-forward into P1 planning

P1 should start only after these lessons are reflected in the P1 workflow/runbook and tracking policy.
