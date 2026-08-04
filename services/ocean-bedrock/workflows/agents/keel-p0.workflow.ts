import type { P0AgentWorkflow } from './p0-common';
import { p0GradeScale, p0SharedBlockHooks, p0SnapshotInputs } from './p0-common';

export const keelP0Workflow: P0AgentWorkflow = {
  id: 'keel-p0-runtime-local-state-review',
  epoch: 'ocean-bedrock-epoch-01',
  phase: 'P0-clean-room',
  agent: 'Keel',
  callsign: 'KEL',
  worktree: { path: '/root/rt-nas/rt-nas/.worktrees/epoch01-keel', branch: 'epoch01/keel-p0', createFrom: 'HEAD' },
  mode: 'read-only-review',
  mission: 'Review runtime/deploy implications of P0 files, especially local Cloudflare/Wrangler state, without deploying or mutating production.',
  readOnlyInputs: [...p0SnapshotInputs, 'README.md', 'docs/HANDOFF.md', '.wrangler/ if copied into packet or inspected read-only on master by Navigator'],
  allowedWrites: ['workflows/runs/epoch01-p0/KEEL-P0-REVIEW.md'],
  forbiddenActions: [
    p0SharedBlockHooks.deployRiskBlock,
    p0SharedBlockHooks.secretBlock,
    'Do not run railway up, wrangler deploy, wrangler delete, or env mutation commands.',
    'Do not edit product/docs files; write review packet only.',
  ],
  steps: [
    {
      id: 'KEL-P0.1',
      title: 'Classify runtime/local tool state',
      intent: 'Decide whether .wrangler and README runtime changes are commit/ignore/defer.',
      suggestedCommands: [
        'git status --short',
        'find .wrangler -maxdepth 3 -type f -print 2>/dev/null || true',
        'git diff -- README.md',
      ],
      writes: ['workflows/runs/epoch01-p0/KEEL-P0-REVIEW.md'],
      acceptance: ['.wrangler recommendation is explicit', 'README runtime diff is classified', 'no production command run'],
      blocksOn: ['deployRiskBlock', 'secretBlock'],
    },
    {
      id: 'KEL-P0.2',
      title: 'Runtime claim consistency check',
      intent: 'Ensure P0 docs/scripts do not overstate deployed state or create deploy ambiguity.',
      writes: ['workflows/runs/epoch01-p0/KEEL-P0-REVIEW.md'],
      acceptance: ['live vs planned runtime claims separated', 'deployment facts remain HANDOFF-owned', 'grade assigned'],
      blocksOn: ['deployRiskBlock'],
    },
  ],
  completionPacket: {
    path: 'workflows/runs/epoch01-p0/KEEL-P0-REVIEW.md',
    requiredSections: ['Checked Inputs', 'Runtime Findings', '.wrangler Classification', 'README Runtime Diff', 'Grade', 'Blocks'],
  },
  externalTracking: {
    githubBranch: 'epoch01/keel-p0',
    githubPrTitle: 'OBE-P0(KEL): Runtime and local state review',
    linearIssueTitle: 'OBE-P0 KEL — Runtime and local state review',
    piMessengerTaskTitle: 'OBE-P0 KEL — Runtime/local state review',
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts',
  },
  peerReview: { reviewedBy: ['Beacon', 'Navigator'], gradeScale: p0GradeScale },
};

export default keelP0Workflow;
