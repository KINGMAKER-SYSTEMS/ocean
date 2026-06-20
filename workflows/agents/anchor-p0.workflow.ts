import type { P0AgentWorkflow } from './p0-common';
import { p0GradeScale, p0SharedBlockHooks, p0SnapshotInputs } from './p0-common';

export const anchorP0Workflow: P0AgentWorkflow = {
  id: 'anchor-p0-source-of-truth-secret-review',
  epoch: 'ocean-bedrock-epoch-01',
  phase: 'P0-clean-room',
  agent: 'Anchor',
  callsign: 'ANK',
  worktree: { path: '/root/rt-nas/rt-nas/.worktrees/epoch01-anchor', branch: 'epoch01/anchor-p0', createFrom: 'HEAD' },
  mode: 'read-only-review',
  mission: 'Review the P0 packet for source-of-truth, token/scope, storage, DB, and secret-safety risks without editing shared files.',
  readOnlyInputs: [...p0SnapshotInputs, 'README.md', 'docs/HANDOFF.md', 'docs/RT-NAS-CREW-BOOT.md', 'docs/ocean-bedrock-agent-roster.json'],
  allowedWrites: ['workflows/runs/epoch01-p0/ANCHOR-P0-REVIEW.md'],
  forbiddenActions: [
    p0SharedBlockHooks.secretBlock,
    p0SharedBlockHooks.dataLossBlock,
    'Do not run migrations.',
    'Do not alter token files or inspect raw token values.',
    'Do not edit product/docs files; write review packet only.',
  ],
  steps: [
    {
      id: 'ANK-P0.1',
      title: 'Secret and credential risk review',
      intent: 'Check candidate commit files for secret-like material and report safely.',
      suggestedCommands: [
        'git status --short',
        'grep -RInE "(api[_-]?key|token|secret|private key|DATABASE_URL|BEGIN .*PRIVATE)" README.md docs scripts workflows 2>/dev/null || true',
      ],
      writes: ['workflows/runs/epoch01-p0/ANCHOR-P0-REVIEW.md'],
      acceptance: ['paths checked are listed', 'no secret values quoted', 'BLOCK if secret-like content is real'],
      blocksOn: ['secretBlock'],
    },
    {
      id: 'ANK-P0.2',
      title: 'Source-of-truth and persistence review',
      intent: 'Verify proposed P0 files do not contradict live DB/storage/token authority rules.',
      writes: ['workflows/runs/epoch01-p0/ANCHOR-P0-REVIEW.md'],
      acceptance: ['token scope claims are consistent with HANDOFF/README', 'DB/storage claims are marked live vs planned', 'grade assigned'],
      blocksOn: ['dataLossBlock'],
    },
  ],
  completionPacket: {
    path: 'workflows/runs/epoch01-p0/ANCHOR-P0-REVIEW.md',
    requiredSections: ['Checked Inputs', 'Secret Scan Result', 'Source-of-Truth Findings', 'Recommended Classification Changes', 'Grade', 'Blocks'],
  },
  externalTracking: {
    githubBranch: 'epoch01/anchor-p0',
    githubPrTitle: 'OBE-P0(ANK): Source-of-truth and secret review',
    linearIssueTitle: 'OBE-P0 ANK — Source-of-truth and secret review',
    piMessengerTaskTitle: 'OBE-P0 ANK — Source-of-truth + secret review',
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts',
  },
  peerReview: { reviewedBy: ['Beacon', 'Navigator'], gradeScale: p0GradeScale },
};

export default anchorP0Workflow;
