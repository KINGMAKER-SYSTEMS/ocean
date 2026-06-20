import type { P0AgentWorkflow } from './p0-common';
import { p0GradeScale, p0SharedBlockHooks } from './p0-common';

export const navigatorP0Workflow: P0AgentWorkflow = {
  id: 'navigator-p0-clean-room-orchestration',
  epoch: 'ocean-bedrock-epoch-01',
  phase: 'P0-clean-room',
  agent: 'Navigator',
  callsign: 'NAV',
  worktree: {
    path: '/root/rt-nas/rt-nas',
    branch: 'master',
    createFrom: 'HEAD',
  },
  mode: 'orchestrate',
  mission: 'Create the shared P0 snapshot, spawn non-colliding worktrees, collect peer reviews, and decide merge/hold with human approval.',
  readOnlyInputs: [
    '/root/notes/20260620-030738-ocean-bedrock-epoch-01-working-note.md',
    '/root/notes/20260619-231701-ocean-bedrock-epoch-01-p0-clean-room-runbook.md',
    'workflows/ocean-epoch-01-p0.workflow.ts',
  ],
  allowedWrites: [
    'workflows/runs/epoch01-p0/P0-SNAPSHOT.md',
    'workflows/runs/epoch01-p0/P0-FILE-INVENTORY.md',
    'workflows/runs/epoch01-p0/P0-CLASSIFICATION-PROPOSAL.md',
    'workflows/runs/epoch01-p0/P0-MERGE-DECISION.md',
    '.worktrees/epoch01-* only through git worktree creation/removal',
  ],
  forbiddenActions: [
    p0SharedBlockHooks.secretBlock,
    p0SharedBlockHooks.deployRiskBlock,
    p0SharedBlockHooks.dataLossBlock,
    'Do not ask agents to edit master directly.',
    'Do not commit until peer grades are A/B or human waives.',
  ],
  steps: [
    {
      id: 'NAV-P0.0',
      title: 'Create P0 baseline snapshot',
      intent: 'Capture current master state once so every agent reviews the same evidence packet.',
      suggestedCommands: [
        'git status --short',
        'git log --oneline -8',
        'find docs scripts workflows -maxdepth 3 -type f | sort',
        'tmux list-panes -t rt-nas-mesh:1 -F "#{pane_index} #{pane_current_command} #{pane_current_path}"',
      ],
      writes: ['workflows/runs/epoch01-p0/P0-SNAPSHOT.md', 'workflows/runs/epoch01-p0/P0-FILE-INVENTORY.md'],
      acceptance: ['snapshot names timestamp, branch, dirty state, active crew, and all candidate paths'],
      blocksOn: ['scopeBlock'],
    },
    {
      id: 'NAV-P0.1',
      title: 'Create isolated worktrees',
      intent: 'Give each agent a non-colliding branch/worktree from HEAD and copy the P0 packet into each worktree.',
      suggestedCommands: [
        'git worktree add .worktrees/epoch01-anchor -b epoch01/anchor-p0 HEAD',
        'git worktree add .worktrees/epoch01-keel -b epoch01/keel-p0 HEAD',
        'git worktree add .worktrees/epoch01-sonar -b epoch01/sonar-p0 HEAD',
        'git worktree add .worktrees/epoch01-beacon -b epoch01/beacon-p0 HEAD',
        'mkdir -p .worktrees/epoch01-{anchor,keel,sonar,beacon}/workflows/runs/epoch01-p0',
        'cp workflows/runs/epoch01-p0/P0-*.md .worktrees/epoch01-anchor/workflows/runs/epoch01-p0/ # repeat per worktree',
      ],
      writes: ['.worktrees/epoch01-anchor', '.worktrees/epoch01-keel', '.worktrees/epoch01-sonar', '.worktrees/epoch01-beacon'],
      acceptance: ['each worktree exists', 'each worktree has snapshot packet', 'each agent has unique branch'],
      blocksOn: ['scopeBlock'],
    },
    {
      id: 'NAV-P0.2',
      title: 'Collect reviews and produce merge decision',
      intent: 'Merge only unique report artifacts, not product edits, unless human approves the final hygiene patch.',
      writes: ['workflows/runs/epoch01-p0/P0-MERGE-DECISION.md'],
      acceptance: ['Anchor/Keel/Sonar/Beacon packets present', 'grades are A/B or block loop is invoked', 'human-facing decision is explicit'],
      blocksOn: ['refinementLoop', 'dataLossBlock', 'deployRiskBlock'],
    },
  ],
  completionPacket: {
    path: 'workflows/runs/epoch01-p0/P0-MERGE-DECISION.md',
    requiredSections: ['Summary', 'Agent Grades', 'Approved Patch Set', 'Blocks', 'Human Decision Needed', 'Next Phase'],
  },
  externalTracking: {
    githubBranch: 'epoch01/navigator-p0',
    githubPrTitle: 'OBE-P0(NAV): Clean room orchestration and merge gate',
    linearIssueTitle: 'OBE-P0 NAV — Clean Room Orchestration',
    piMessengerTaskTitle: 'OBE-P0 NAV — Clean room orchestration',
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts',
  },
  peerReview: {
    reviewedBy: ['Beacon'],
    gradeScale: p0GradeScale,
  },
};

export default navigatorP0Workflow;
