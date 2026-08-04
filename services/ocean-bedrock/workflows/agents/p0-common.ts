// Shared P0 agent workflow contracts for Ocean Bedrock Epoch 01.
// These are declarative runbooks, not an execution engine.

export type P0Agent = 'Navigator' | 'Anchor' | 'Keel' | 'Sonar' | 'Beacon';
export type P0Grade = 'A' | 'B' | 'C' | 'BLOCK';
export type P0Mode = 'read-only-review' | 'orchestrate' | 'evidence-review';

export interface P0CommandStep {
  id: string;
  title: string;
  intent: string;
  suggestedCommands?: string[];
  writes: string[];
  acceptance: string[];
  blocksOn: string[];
}

export interface P0AgentWorkflow {
  id: string;
  epoch: 'ocean-bedrock-epoch-01';
  phase: 'P0-clean-room';
  agent: P0Agent;
  callsign: string;
  worktree: {
    path: string;
    branch: string;
    createFrom: 'HEAD';
  };
  mode: P0Mode;
  mission: string;
  readOnlyInputs: string[];
  allowedWrites: string[];
  forbiddenActions: string[];
  steps: P0CommandStep[];
  completionPacket: {
    path: string;
    requiredSections: string[];
  };
  externalTracking: {
    githubBranch: string;
    githubPrTitle: string;
    linearIssueTitle: string;
    piMessengerTaskTitle: string;
    trackingPolicy: 'workflows/agents/p0-tracking-policy.ts';
  };
  peerReview: {
    reviewedBy: P0Agent[];
    gradeScale: Record<P0Grade, string>;
  };
}

export const p0SharedBlockHooks = {
  secretBlock: 'If any raw credential/token/private key/env dump appears, stop and report only path + generic secret type. Do not quote value.',
  scopeBlock: 'If work requires touching outside /root/rt-nas/rt-nas or another mesh, stop and ask Navigator/human.',
  deployRiskBlock: 'If work implies Railway/Cloudflare production action, stop for Keel + human approval.',
  dataLossBlock: 'If work deletes non-junk data, changes DB schema, storage routing, or R2/local objects, stop for Anchor + Beacon + human approval.',
  refinementLoop: 'If peer grade is C/BLOCK, revise only the blocked slice and rerun the same reviewer before continuing.',
} as const;

export const p0SnapshotInputs = [
  'workflows/runs/epoch01-p0/P0-SNAPSHOT.md',
  'workflows/runs/epoch01-p0/P0-FILE-INVENTORY.md',
  'workflows/runs/epoch01-p0/P0-CLASSIFICATION-PROPOSAL.md',
] as const;

export const p0GradeScale: Record<P0Grade, string> = {
  A: 'Accepted. No blocking issue.',
  B: 'Accepted with explicit follow-up.',
  C: 'Needs revision before merge.',
  BLOCK: 'Unsafe or out of scope. Stop dependent work.',
};
