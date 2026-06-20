import type { P0AgentWorkflow } from './p0-common';
import { p0GradeScale, p0SharedBlockHooks, p0SnapshotInputs } from './p0-common';

export const sonarP0Workflow: P0AgentWorkflow = {
  id: 'sonar-p0-docs-context-map',
  epoch: 'ocean-bedrock-epoch-01',
  phase: 'P0-clean-room',
  agent: 'Sonar',
  callsign: 'SNR',
  worktree: { path: '/root/rt-nas/rt-nas/.worktrees/epoch01-sonar', branch: 'epoch01/sonar-p0', createFrom: 'HEAD' },
  mode: 'read-only-review',
  mission: 'Map Epoch 01 documentation sources, detect stale/conflicting claims, and propose canonical doc hierarchy without editing shared docs.',
  readOnlyInputs: [...p0SnapshotInputs, 'README.md', 'docs/HANDOFF.md', 'docs/RT-NAS-CREW-BOOT.md', 'docs/OCEAN-BEDROCK-AGENT-ROSTER.md', 'docs/agents/*.md'],
  allowedWrites: ['workflows/runs/epoch01-p0/SONAR-P0-DOCS-MAP.md'],
  forbiddenActions: [
    p0SharedBlockHooks.scopeBlock,
    'Do not rewrite canonical docs in P0 review mode.',
    'Do not redefine Ocean Longhouse, ocean-daemon, or Bedrock boundaries.',
    'Do not edit product/docs files; write review packet only.',
  ],
  steps: [
    {
      id: 'SNR-P0.1',
      title: 'Build canonical docs map',
      intent: 'Identify glossary, runbook, live handoff, crew docs, and future charter locations.',
      suggestedCommands: ['find docs -maxdepth 2 -type f | sort', 'grep -RIn "Ocean Longhouse\|ocean-daemon\|ocean-bedrock" README.md docs | head -120'],
      writes: ['workflows/runs/epoch01-p0/SONAR-P0-DOCS-MAP.md'],
      acceptance: ['doc hierarchy proposed', 'working note remains glossary', 'HANDOFF remains live-state doc'],
      blocksOn: ['scopeBlock'],
    },
    {
      id: 'SNR-P0.2',
      title: 'Conflict/staleness review',
      intent: 'Flag docs that conflict with current live state or P0 strategy.',
      writes: ['workflows/runs/epoch01-p0/SONAR-P0-DOCS-MAP.md'],
      acceptance: ['conflicts are quoted by path/section, not rewritten', 'recommendations are atomic', 'grade assigned'],
      blocksOn: ['refinementLoop'],
    },
  ],
  completionPacket: {
    path: 'workflows/runs/epoch01-p0/SONAR-P0-DOCS-MAP.md',
    requiredSections: ['Canonical Docs Map', 'Stale or Conflicting Claims', 'Recommended Doc Patch Set', 'Grade', 'Blocks'],
  },
  externalTracking: {
    githubBranch: 'epoch01/sonar-p0',
    githubPrTitle: 'OBE-P0(SNR): Canonical docs and context map',
    linearIssueTitle: 'OBE-P0 SNR — Docs and context map',
    piMessengerTaskTitle: 'OBE-P0 SNR — Docs/context map',
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts',
  },
  peerReview: { reviewedBy: ['Anchor', 'Navigator'], gradeScale: p0GradeScale },
};

export default sonarP0Workflow;
