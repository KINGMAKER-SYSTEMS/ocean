// Ocean Bedrock Coworker Onboarding — ceremony fixture data.
//
// Provides a self-contained, fake-but-realistic dataset that lets a skill
// runner (or the replay script) drive the Ocean onboarding ceremony end to end
// WITHOUT requiring a real Claude Code, Codex, or Pi setup on the machine.
//
// The replay script (scripts/replay-ceremony-fixture.mjs) consumes
// `buildFixtureFrames()` and streams the events to the Ocean local app using
// the `ocean.ceremony.event.v1` protocol defined in
// docs/superpowers/plans/2026-07-07-ocean-skill-onboarding-ceremony.md.
//
// Event bodies here intentionally OMIT `schema`, `runId`, `seq`, and `ts`:
// per the protocol the Ocean local app assigns those server-side. The runner
// only sends `type`, `phase`, `status`, `message`, `payload`, and optionally
// `visibility` / `runner`.

export const FIXTURE_RUNNER_DEFAULT = Object.freeze({
  runtime: 'other',
  name: 'Ocean Fixture Replay',
});

export const FIXTURE_OWNER_DEFAULT = 'ocean-coworker';

// --- Peer agent config sources (3 — satisfies "3 source_detected") ----------
// These represent the three peer local config sources the skill scans. They are
// treated as equals: none is privileged, and none carries auth material.

export const FIXTURE_SOURCES = Object.freeze([
  {
    sourceId: 'claude-code',
    label: 'Claude Code',
    logo: 'claude',
    root: '~/.claude',
    counts: { agents: 8, skills: 6, projects: 18, mcpServers: 3 },
  },
  {
    sourceId: 'codex',
    label: 'Codex',
    logo: 'codex',
    root: '~/.codex',
    counts: { agents: 0, skills: 0, projects: 12, mcpServers: 1 },
  },
  {
    sourceId: 'pi',
    label: 'Pi',
    logo: 'pi',
    root: '~/.pi',
    counts: { agents: 4, skills: 3, projects: 7, mcpServers: 1 },
  },
]);

// --- Skills (6 — satisfies "5 skill_detected") -----------------------------
export const FIXTURE_SKILLS = Object.freeze([
  { sourceId: 'claude-code', name: 'axum', relPath: 'skills/axum/SKILL.md', color: 'cyan' },
  { sourceId: 'claude-code', name: 'leptos', relPath: 'skills/leptos/SKILL.md', color: 'cyan' },
  { sourceId: 'claude-code', name: 'supabase', relPath: 'skills/supabase/SKILL.md', color: 'cyan' },
  { sourceId: 'claude-code', name: 'tailwind', relPath: 'skills/tailwind/SKILL.md', color: 'cyan' },
  { sourceId: 'pi', name: 'tg-reply', relPath: 'skills/tg-reply/SKILL.md', color: 'violet' },
  { sourceId: 'pi', name: 'subagent-framework', relPath: 'skills/subagent-framework/SKILL.md', color: 'violet' },
]);

// --- Projects (6 — satisfies "5 project_detected") -------------------------
export const FIXTURE_PROJECTS = Object.freeze([
  { sourceId: 'claude-code', name: 'ocean-bedrock', root: '~/dev/ocean-bedrock', configDocs: ['AGENTS.md', 'CLAUDE.md'], sessions: 14 },
  { sourceId: 'codex', name: 'lifted-ship-clipppers', root: '~/lifted-ship-clipppers', configDocs: ['AGENTS.md'], sessions: 9 },
  { sourceId: 'codex', name: 'tides-sheets', root: '~/tides-sheets', configDocs: ['AGENTS.md'], sessions: 21 },
  { sourceId: 'claude-code', name: 'campaign-hub', root: '~/dev/campaign-hub', configDocs: ['AGENTS.md', 'CLAUDE.md'], sessions: 6 },
  { sourceId: 'pi', name: 'content-posting-lab', root: '~/dev/content-posting-lab', configDocs: ['AGENTS.md'], sessions: 4 },
  { sourceId: 'claude-code', name: 'rt-nas-crew', root: '~/dev/rt-nas-crew', configDocs: ['AGENTS.md'], sessions: 2 },
]);

// --- MCP servers (4 — satisfies "2 mcp_detected") --------------------------
export const FIXTURE_MCPS = Object.freeze([
  { sourceId: 'claude-code', name: 'context7', transport: 'http', scope: 'user' },
  { sourceId: 'claude-code', name: 'railway', transport: 'stdio', scope: 'project' },
  { sourceId: 'claude-code', name: 'supabase', transport: 'stdio', scope: 'project' },
  { sourceId: 'codex', name: 'filesystem', transport: 'stdio', scope: 'project' },
]);

// --- Docs (rendered as doc_detected — real filenames only) -----------------
export const FIXTURE_DOCS = Object.freeze([
  { sourceId: 'claude-code', name: 'CLAUDE.md', relPath: '~/.claude/CLAUDE.md' },
  { sourceId: 'codex', name: 'AGENTS.md', relPath: '~/.codex/AGENTS.md' },
  { sourceId: 'pi', name: 'AGENTS.md', relPath: '~/.pi/AGENTS.md' },
  { sourceId: 'codex', name: 'config.toml', relPath: '~/.codex/config.toml' },
]);

// --- Sessions (counts only — contents never imported raw) ------------------
export const FIXTURE_SESSIONS = Object.freeze([
  { sourceId: 'claude-code', project: 'ocean-bedrock', count: 14 },
  { sourceId: 'codex', project: 'tides-sheets', count: 21 },
  { sourceId: 'pi', project: 'content-posting-lab', count: 4 },
]);

// --- Secrets that MUST be skipped (never imported, never rendered) ----------
// These are emitted as `agent.secret_skipped` with visibility=redacted so the
// stage shows a label/count only. No token bytes ever leave the machine.
export const FIXTURE_SECRETS_SKIPPED = Object.freeze([
  { sourceId: 'claude-code', relPath: '.credentials.json', reason: 'auth-token-file', displayName: 'claude credentials' },
  { sourceId: 'codex', relPath: 'auth.json', reason: 'auth-token-file', displayName: 'codex auth.json' },
  { sourceId: 'codex', relPath: '.env', reason: 'secret-pattern:env-file', displayName: 'codex .env' },
  { sourceId: 'pi', relPath: 'secrets/keys.pem', reason: 'secret-pattern:pem-key', displayName: 'pi keys.pem' },
]);

// ---------------------------------------------------------------------------
// Frame builder
// ---------------------------------------------------------------------------

function ev(type, phase, status, message, payload, visibility) {
  const event = { type, phase, status, message, payload };
  if (visibility) event.visibility = visibility;
  return event;
}

function single(event) {
  return { kind: 'single', event };
}

function batch(events) {
  return { kind: 'batch', events };
}

/**
 * Build the ordered ceremony event stream as "frames". Each frame is either a
 * single event (`{ kind:'single', event }`) or a small batch
 * (`{ kind:'batch', events:[...] }`). Batching exercises the local app's
 * batch-POST path; singles exercise the single-event path.
 *
 * The returned counts object lets the replay self-verify the Lane 3 acceptance
 * minimums without parsing the frames.
 *
 * @param {{ runner?: object, ownerName?: string }} [options]
 * @returns {{ frames: Array, counts: Record<string, number>, runner: object, ownerName: string }}
 */
export function buildFixtureFrames(options = {}) {
  const runner = { ...FIXTURE_RUNNER_DEFAULT, ...(options.runner || {}) };
  const ownerName = options.ownerName || FIXTURE_OWNER_DEFAULT;
  const frames = [];
  const counts = {};

  const bump = (type) => {
    counts[type] = (counts[type] || 0) + 1;
  };

  const track = (event) => {
    bump(event.type);
    return event;
  };

  // --- Boot / scene -------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.run_started',
    'boot', 'started',
    `Onboarding ceremony started for ${ownerName}`,
    { ownerName, runner, mode: 'onboarding-import-v1' },
  ))));

  frames.push(single(track(ev(
    'ceremony.scene_set',
    'boot', 'progress',
    'Welcome to Ocean',
    { scene: 'welcome', focus: 'globe', tone: 'calm_build' },
  ))));

  // --- Scan scene ---------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'scan', 'progress',
    'Scanning your agent constellations',
    { scene: 'constellation_scan', focus: 'globe', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ceremony.loading_set',
    'scan', 'progress',
    'Looking for Claude Code, Codex, and Pi configs',
    { label: 'Scanning local agent configs', indeterminate: true },
  ))));

  // 3 source_detected events (acceptance: >= 3)
  for (const src of FIXTURE_SOURCES) {
    frames.push(single(track(ev(
      'agent.source_detected',
      'scan', 'complete',
      `${src.label} setup detected`,
      {
        sourceId: src.sourceId,
        label: src.label,
        logo: src.logo,
        root: src.root,
        counts: { ...src.counts },
      },
    ))));
  }

  // Secrets skipped — emitted early so the user sees safety in action.
  frames.push(batch(FIXTURE_SECRETS_SKIPPED.map((s) => track(ev(
    'agent.secret_skipped',
    'scan', 'info',
    `Sensitive ${s.displayName} skipped`,
    {
      sourceId: s.sourceId,
      relPath: s.relPath,
      reason: s.reason,
      displayName: s.displayName,
    },
    'redacted',
  )))));

  // Skills (acceptance: >= 5) — batched to exercise batch POST.
  frames.push(batch(FIXTURE_SKILLS.map((skill) => track(ev(
    'agent.skill_detected',
    'scan', 'progress',
    `Skill found: ${skill.name}`,
    {
      sourceId: skill.sourceId,
      name: skill.name,
      relPath: skill.relPath,
      display: { orbit: 'skills', color: skill.color },
      import: {
        eligible: true,
        targetPath: `/context/ocean-bedrock/imports/${skill.sourceId}/skills/${skill.name}/SKILL.md`,
      },
    },
  )))));

  // Projects (acceptance: >= 5) — batched.
  frames.push(batch(FIXTURE_PROJECTS.map((proj) => track(ev(
    'agent.project_detected',
    'scan', 'progress',
    `Project found: ${proj.name}`,
    {
      sourceId: proj.sourceId,
      name: proj.name,
      root: proj.root,
      configDocs: [...proj.configDocs],
      sessions: proj.sessions,
      import: {
        eligible: true,
        targetPath: `/sessions/${ownerName}/imported/${proj.sourceId}/${proj.name}/`,
      },
    },
  )))));

  // MCP servers (acceptance: >= 2) — batched.
  frames.push(batch(FIXTURE_MCPS.map((mcp) => track(ev(
    'agent.mcp_detected',
    'scan', 'progress',
    `MCP server found: ${mcp.name}`,
    {
      sourceId: mcp.sourceId,
      name: mcp.name,
      transport: mcp.transport,
      scope: mcp.scope,
      import: { eligible: true },
    },
  )))));

  // Docs — real filenames only.
  frames.push(batch(FIXTURE_DOCS.map((doc) => track(ev(
    'agent.doc_detected',
    'scan', 'progress',
    `Doc found: ${doc.name}`,
    {
      sourceId: doc.sourceId,
      name: doc.name,
      relPath: doc.relPath,
      import: { eligible: true },
    },
  )))));

  // Sessions — counts only, never raw contents.
  frames.push(batch(FIXTURE_SESSIONS.map((session) => track(ev(
    'agent.session_detected',
    'scan', 'progress',
    `Sessions found: ${session.count} for ${session.project}`,
    {
      sourceId: session.sourceId,
      project: session.project,
      count: session.count,
      import: { eligible: true, targetPath: `/sessions/${ownerName}/imported/${session.sourceId}/${session.project}/` },
    },
  )))));

  // --- Review / source review scene --------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'review', 'progress',
    'Reviewing discovered sources',
    { scene: 'source_review', focus: 'sources', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ceremony.toast',
    'review', 'info',
    'Auth files and secrets were skipped automatically',
    { tone: 'success', text: 'No tokens or keychain items imported.' },
  ))));

  // --- Import plan + preview ---------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'import', 'progress',
    'Preparing import preview',
    { scene: 'import_stream', focus: 'stream', tone: 'calm_build' },
  ))));

  const queuedItems = [
    ...FIXTURE_SKILLS.map((s) => ({ kind: 'skill', sourceId: s.sourceId, name: s.name, relPath: s.relPath })),
    ...FIXTURE_PROJECTS.map((p) => ({ kind: 'project', sourceId: p.sourceId, name: p.name, root: p.root })),
    ...FIXTURE_MCPS.map((m) => ({ kind: 'mcp', sourceId: m.sourceId, name: m.name })),
  ];

  frames.push(single(track(ev(
    'import.plan_started',
    'import', 'started',
    `Import plan built for ${queuedItems.length} items`,
    { itemCount: queuedItems.length, sources: FIXTURE_SOURCES.map((s) => s.sourceId) },
  ))));

  frames.push(batch(queuedItems.map((item) => track(ev(
    'import.item_queued',
    'import', 'progress',
    `Queued ${item.kind}: ${item.name}`,
    {
      kind: item.kind,
      sourceId: item.sourceId,
      name: item.name,
      relPath: item.relPath || item.root,
      import: { eligible: true },
    },
  )))));

  frames.push(batch(FIXTURE_SECRETS_SKIPPED.map((s) => track(ev(
    'import.item_skipped',
    'import', 'info',
    `Skipped sensitive ${s.displayName}`,
    {
      sourceId: s.sourceId,
      relPath: s.relPath,
      reason: s.reason,
    },
    'redacted',
  )))));

  frames.push(single(track(ev(
    'import.plan_completed',
    'import', 'complete',
    'Import preview ready for review',
    { queued: queuedItems.length, skipped: FIXTURE_SECRETS_SKIPPED.length },
  ))));

  // --- Connect / invite + folders (uses existing local app endpoints) -----
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'connect', 'progress',
    'Connecting Ocean',
    { scene: 'connect_ocean', focus: 'globe', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ocean.invite_redeemed',
    'connect', 'complete',
    'Invite redeemed',
    { ownerName },
  ))));

  frames.push(single(track(ev(
    'ocean.folder_added',
    'connect', 'progress',
    'Folder staged for import: ~/dev/ocean-bedrock',
    { path: '~/dev/ocean-bedrock', label: 'ocean-bedrock' },
  ))));

  // --- Preview gate (HUMAN ack) ------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'preview', 'progress',
    'Dry-run preview ready',
    { scene: 'preview_gate', focus: 'ack_button', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ocean.preview_started',
    'preview', 'started',
    'Dry-run preview generated',
    { itemCount: queuedItems.length },
  ))));

  frames.push(single(track(ev(
    'ocean.preview_completed',
    'preview', 'complete',
    'Preview complete — awaiting your review',
    { queued: queuedItems.length, skipped: FIXTURE_SECRETS_SKIPPED.length },
  ))));

  frames.push(single(track(ev(
    'ocean.ack_required',
    'preview', 'blocked',
    'Acknowledge the preview to unlock sync',
    { gate: 'preview_ack' },
  ))));

  frames.push(single(track(ev(
    'ceremony.highlight',
    'preview', 'info',
    'Review the preview, then click the unlock button',
    { target: 'ack_button' },
  ))));

  // THE human ack gate. The skill (and this fixture) MUST pause here and wait
  // for a real human click surfaced through /api/state syncUnlocked=true.
  frames.push(single(track(ev(
    'ceremony.await_user',
    'ack', 'blocked',
    'Waiting for you to acknowledge the preview',
    {
      gate: 'preview_ack',
      instructions: 'In the Ocean stage, click “I reviewed this preview — unlock sync”. The runner will resume automatically.',
    },
  ))));

  // The replay resumes here only after syncUnlocked becomes true (or after a
  // --simulate-ack fixture convenience). The skill itself never auto-clicks.
  frames.push(single(track(ev(
    'ocean.ack_observed',
    'ack', 'complete',
    'Preview acknowledged by user — sync unlocked',
    { gate: 'preview_ack' },
  ))));

  // --- Sync ---------------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'sync', 'progress',
    'Syncing your context into Ocean',
    { scene: 'sync_stream', focus: 'stream', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ocean.sync_started',
    'sync', 'started',
    'Sync started',
    { items: queuedItems.length },
  ))));

  frames.push(single(track(ev(
    'ceremony.loading_set',
    'sync', 'progress',
    'Streaming import into Ocean',
    { label: 'Syncing', progress: 1, total: queuedItems.length },
  ))));

  frames.push(single(track(ev(
    'ocean.sync_completed',
    'sync', 'complete',
    'Sync complete',
    { items: queuedItems.length },
  ))));

  // --- Complete -----------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'complete', 'complete',
    'Onboarding complete',
    { scene: 'complete', focus: 'globe', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ceremony.run_completed',
    'complete', 'complete',
    `Onboarding ceremony complete for ${ownerName}`,
    { ownerName, sources: FIXTURE_SOURCES.map((s) => s.sourceId) },
  ))));

  return { frames, counts, runner, ownerName };
}

/**
 * Lane 3 acceptance minimums. The replay asserts these against `counts`.
 */
export const ACCEPTANCE_MINIMUMS = Object.freeze({
  'agent.source_detected': 3,
  'agent.skill_detected': 5,
  'agent.project_detected': 5,
  'agent.mcp_detected': 2,
  // ceremony.await_user with gate=preview_ack is tracked separately (gate check)
  'ceremony.run_completed': 1,
});

/**
 * True if the built frames satisfy the Lane 3 acceptance minimums, including
 * the single `ceremony.await_user` event whose payload.gate === 'preview_ack'.
 */
export function meetsAcceptance({ frames, counts }) {
  for (const [type, min] of Object.entries(ACCEPTANCE_MINIMUMS)) {
    if ((counts[type] || 0) < min) return false;
  }
  const previewAck = frames.some(
    (f) => {
      const events = f.kind === 'single' ? [f.event] : f.events;
      return events.some(
        (e) => e.type === 'ceremony.await_user' && e.payload && e.payload.gate === 'preview_ack',
      );
    },
  );
  return previewAck;
}
