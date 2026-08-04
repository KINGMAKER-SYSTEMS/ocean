// Ocean Bedrock Epoch 01 — P0 external tracking, GitHub, Linear, and review rules.
// Declarative policy imported/linked by the per-agent P0 workflows.

export type P0Tracker = 'pi-messenger-kanban' | 'github' | 'linear';
export type P0ReviewRound = 'R0-self-check' | 'R1-domain-peer' | 'R2-cross-grade-swap' | 'R3-refinement-loop' | 'R4-navigator-merge-gate';

export interface P0GithubRules {
  repository: string;
  defaultBranch: 'master';
  branchPrefix: 'epoch01/';
  workerBranchRules: string[];
  pullRequestRules: string[];
  githubActionsRules: string[];
  protectedFiles: string[];
}

export interface P0LinearRules {
  workspaceHint: string;
  teamName: 'ocean-os';
  projectName: 'Ocean OS — System Stabilization';
  issuePrefix: 'OBE-P0';
  requiredFields: string[];
  lifecycleRules: string[];
}

export interface P0KanbanRules {
  board: 'pi-messenger crew task board';
  sourceOfTruth: 'workflows/ocean-epoch-01-p0.workflow.ts';
  mirrorRules: string[];
}

export interface P0ReviewSwapRound {
  round: P0ReviewRound;
  purpose: string;
  assignments: Record<string, string>;
  passCondition: string;
  failCondition: string;
}

export const p0GithubRules: P0GithubRules = {
  repository: 'RisingTides/rt-nas',
  defaultBranch: 'master',
  branchPrefix: 'epoch01/',
  workerBranchRules: [
    'Navigator may inspect master during P0 but worker agents must use their own worktree branch.',
    'Anchor branch: epoch01/anchor-p0, worktree: .worktrees/epoch01-anchor.',
    'Keel branch: epoch01/keel-p0, worktree: .worktrees/epoch01-keel.',
    'Sonar branch: epoch01/sonar-p0, worktree: .worktrees/epoch01-sonar.',
    'Beacon branch: epoch01/beacon-p0, worktree: .worktrees/epoch01-beacon.',
    'Worker branches in P0 write review packets only under workflows/runs/epoch01-p0/ unless Navigator/human approves a patch.',
    'No force-push, history rewrite, or direct push to master during P0.',
  ],
  pullRequestRules: [
    'One PR per worker branch only if that worker produces files that must merge; otherwise Navigator cherry-picks/copies review packets into the merge decision.',
    'PR title format: OBE-P0(<agent>): <short outcome>.',
    'PR body must link Linear issue, pi-messenger task, worktree branch, and completion packet.',
    'PR must include Review Matrix with A/B/C/BLOCK grades.',
    'PR cannot merge with any C or BLOCK grade unless the human explicitly waives in writing.',
    'Navigator is the only merge captain for P0.',
  ],
  githubActionsRules: [
    'Required before merge when code/scripts changed: npm run smoke.',
    'Required before merge when DB/source/runtime claims changed: npm run db:check or documented reason it cannot run locally.',
    'Required before merge when TypeScript workflow files changed: peer syntax/readability review; do not add build tooling during P0 unless approved.',
    'Do not edit .github/workflows/* during P0 without Keel review plus human approval.',
    'Do not disable, bypass, or mark failing GitHub Actions as acceptable without Beacon review plus human approval.',
    'Do not run workflow_dispatch against production-affecting workflows during P0.',
  ],
  protectedFiles: [
    '.github/workflows/**',
    'db/**',
    'src/mounts/**',
    'src/auth.mjs',
    'src/server.mjs',
    'scripts/migrate.mjs',
    'scripts/check-postgres.mjs',
  ],
};

export const p0LinearRules: P0LinearRules = {
  workspaceHint: 'Use LINEAR_API_KEY if available; never print it.',
  teamName: 'ocean-os',
  projectName: 'Ocean OS — System Stabilization',
  issuePrefix: 'OBE-P0',
  requiredFields: [
    'Agent owner',
    'Worktree branch/path',
    'pi-messenger task id',
    'GitHub branch/PR if any',
    'Completion packet path',
    'Review grade',
    'Blocks/refinement rounds',
  ],
  lifecycleRules: [
    'Create one Linear issue per P0 agent workflow plus one Navigator orchestration issue.',
    'Linear status mirrors pi-messenger kanban: todo -> in progress -> review -> done/blocked.',
    'Every C/BLOCK review adds a Linear comment with failing criterion and next refinement owner.',
    'Do not close Linear issue until completion packet exists and Beacon/Navigator accept evidence.',
    'If GitHub PR exists, link it in the Linear issue before review.',
  ],
};

export const p0KanbanRules: P0KanbanRules = {
  board: 'pi-messenger crew task board',
  sourceOfTruth: 'workflows/ocean-epoch-01-p0.workflow.ts',
  mirrorRules: [
    'Create pi-messenger tasks for NAV, ANK, KEL, SNR, BCN P0 workflows.',
    'For named-agent worktree waves, pi-messenger is kanban/tracking only: do not use work/autonomous/lobby workers.',
    'Each named agent must manually start its own task before writing its packet; if auto-lobby starts, stop/reset immediately and record a process block.',
    'Task title format: OBE-P0 <CALLSIGN> — <mission short name>.',
    'Task content includes Linear issue URL/id, worktree path, branch, completion packet path, and block hooks.',
    'Do not mark pi-messenger task done until completion packet exists and required peer grade is A/B.',
    'If Linear or GitHub updates fail, pi-messenger remains the local visible board and records the block.',
  ],
};

export const p0ReviewSwapRounds: P0ReviewSwapRound[] = [
  {
    round: 'R0-self-check',
    purpose: 'Each agent validates its own packet before asking for review.',
    assignments: {
      Anchor: 'Anchor self-checks secret/source-of-truth packet.',
      Keel: 'Keel self-checks runtime/local-state packet.',
      Sonar: 'Sonar self-checks docs/context map.',
      Beacon: 'Beacon self-checks evidence matrix.',
      Navigator: 'Navigator self-checks merge decision packet.',
    },
    passCondition: 'Packet has required sections and explicit grade recommendation.',
    failCondition: 'Missing required section or unsubstantiated claim.',
  },
  {
    round: 'R1-domain-peer',
    purpose: 'Closest domain peer reviews for correctness.',
    assignments: {
      Anchor: 'Reviewed by Keel for runtime impact of source/storage claims.',
      Keel: 'Reviewed by Anchor for persistence/secret implications of runtime claims.',
      Sonar: 'Reviewed by Anchor for source-of-truth correctness.',
      Beacon: 'Reviewed by Sonar for clarity/completeness of evidence language.',
      Navigator: 'Reviewed by Beacon for evidence and gate completeness.',
    },
    passCondition: 'Reviewer assigns A or B.',
    failCondition: 'Reviewer assigns C or BLOCK and opens refinement loop.',
  },
  {
    round: 'R2-cross-grade-swap',
    purpose: 'Second reviewer catches blind spots with a rotating swap.',
    assignments: {
      Anchor: 'Reviewed by Sonar for doc/claim clarity.',
      Sonar: 'Reviewed by Keel for runtime/deploy ambiguity.',
      Keel: 'Reviewed by Beacon for evidence sufficiency.',
      Beacon: 'Reviewed by Anchor for safety gates and source-of-truth accuracy.',
      Navigator: 'Reviewed by Anchor + Keel if merge packet touches safety/runtime decisions.',
    },
    passCondition: 'No second-review C/BLOCK remains.',
    failCondition: 'Any C/BLOCK triggers R3 for the blocked slice only.',
  },
  {
    round: 'R3-refinement-loop',
    purpose: 'Repair only the failing slice, then rerun the reviewer who blocked it.',
    assignments: {
      'Blocked owner': 'Revises blocked section only.',
      'Blocking reviewer': 'Regrades only the revised blocked section.',
      Beacon: 'Confirms evidence of revision is recorded.',
      Navigator: 'Decides whether dependent task can resume.',
    },
    passCondition: 'Blocking reviewer changes grade to A/B or human waives.',
    failCondition: 'Repeated BLOCK escalates to human and dependent tasks stop.',
  },
  {
    round: 'R4-navigator-merge-gate',
    purpose: 'Final P0 merge/hold decision.',
    assignments: {
      Navigator: 'Assembles merge decision and asks human for final approval if commit/merge is proposed.',
      Beacon: 'Verifies grade matrix and command evidence.',
    },
    passCondition: 'All required grades are A/B, or waiver is explicit; merge/hold note is written.',
    failCondition: 'Any unresolved C/BLOCK or missing evidence holds P0 open.',
  },
];

export const p0ExternalTrackingPolicy = {
  github: p0GithubRules,
  linear: p0LinearRules,
  kanban: p0KanbanRules,
  reviewSwapRounds: p0ReviewSwapRounds,
} as const;

export default p0ExternalTrackingPolicy;
