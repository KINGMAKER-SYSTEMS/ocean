import type { P0AgentWorkflow } from './p0-common';
import { p0GradeScale, p0SharedBlockHooks, p0SnapshotInputs } from './p0-common';

export const beaconP0Workflow: P0AgentWorkflow = {
  id: 'beacon-p0-evidence-and-grade-loop',
  epoch: 'ocean-bedrock-epoch-01',
  phase: 'P0-clean-room',
  agent: 'Beacon',
  callsign: 'BCN',
  worktree: { path: '/root/rt-nas/rt-nas/.worktrees/epoch01-beacon', branch: 'epoch01/beacon-p0', createFrom: 'HEAD' },
  mode: 'evidence-review',
  mission: 'Verify evidence completeness, run safe checks when requested, collect grades, and enforce refinement loops before merge.',
  readOnlyInputs: [...p0SnapshotInputs, 'workflows/runs/epoch01-p0/*-P0-*.md', 'package.json'],
  allowedWrites: ['workflows/runs/epoch01-p0/BEACON-P0-EVIDENCE-GRADES.md'],
  forbiddenActions: [
    p0SharedBlockHooks.deployRiskBlock,
    p0SharedBlockHooks.dataLossBlock,
    'Do not run destructive tests.',
    'Do not run production deploys or mutate external services.',
    'Do not edit product/docs files; write evidence packet only.',
  ],
  steps: [
    {
      id: 'BCN-P0.1',
      title: 'Evidence completeness review',
      intent: 'Confirm each agent packet has commands/evidence sufficient for its grade.',
      suggestedCommands: ['ls -la workflows/runs/epoch01-p0', 'git status --short', 'npm run smoke # only if Navigator/human requests safe local smoke'],
      writes: ['workflows/runs/epoch01-p0/BEACON-P0-EVIDENCE-GRADES.md'],
      acceptance: ['missing evidence is listed by task', 'safe checks include exact command + result', 'no production mutation performed'],
      blocksOn: ['deployRiskBlock', 'dataLossBlock'],
    },
    {
      id: 'BCN-P0.2',
      title: 'Cross-review grade matrix',
      intent: 'Compile Anchor/Keel/Sonar/Navigator grades and trigger refinement for C/BLOCK.',
      writes: ['workflows/runs/epoch01-p0/BEACON-P0-EVIDENCE-GRADES.md'],
      acceptance: ['grade matrix complete', 'C/BLOCK entries include failing criterion', 'merge recommendation is explicit'],
      blocksOn: ['refinementLoop'],
    },
  ],
  completionPacket: {
    path: 'workflows/runs/epoch01-p0/BEACON-P0-EVIDENCE-GRADES.md',
    requiredSections: ['Evidence Reviewed', 'Safe Checks Run', 'Grade Matrix', 'Refinement Loops', 'Merge Recommendation'],
  },
  externalTracking: {
    githubBranch: 'epoch01/beacon-p0',
    githubPrTitle: 'OBE-P0(BCN): Evidence and grade loop',
    linearIssueTitle: 'OBE-P0 BCN — Evidence and grade loop',
    piMessengerTaskTitle: 'OBE-P0 BCN — Evidence + grade loop',
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts',
  },
  peerReview: { reviewedBy: ['Navigator'], gradeScale: p0GradeScale },
};

export default beaconP0Workflow;
