import path from 'node:path';
import { promises as fs } from 'node:fs';

import { getPool, recordObjectWrite } from './metadata.mjs';
import { semanticStatus } from './semantic.mjs';
import { notifyTriage } from './notify.mjs';

function dayStamp(date = new Date()) {
  return date.toISOString().slice(0, 10);
}

function slotStamp(date = new Date()) {
  return date.toISOString().slice(11, 13);
}

function json(value) {
  return JSON.stringify(value, null, 2);
}

async function countRow(db, sql, values = []) {
  const result = await db.query(sql, values);
  return Number(result.rows[0]?.count || 0);
}

async function collectDbSnapshot(db) {
  if (!db) return { enabled: false };
  const staleJobDays = Number(process.env.OCEAN_TRIAGE_STALE_JOB_DAYS || 3);
  const [objects, chunks, embeddedChunks, pendingChunks, graphNodes, graphEdges, queuedJobs, failedJobs, staleFailedJobs, failedSyncRuns, sourceRecords] = await Promise.all([
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.objects WHERE deleted_at IS NULL`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.embedding_chunks`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.embedding_chunks WHERE vectorize_id IS NOT NULL`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.embedding_chunks WHERE vectorize_id IS NULL`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.graph_nodes`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.graph_edges`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.ingest_jobs WHERE status = 'queued'`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.ingest_jobs WHERE status = 'failed'`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.ingest_jobs WHERE status = 'failed' AND updated_at < now() - ($1 || ' days')::interval`, [staleJobDays]),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.source_sync_runs WHERE status = 'failed'`),
    countRow(db, `SELECT count(*)::int AS count FROM longhouse.source_records WHERE deleted_at IS NULL`),
  ]);

  const recentJobs = await db.query(
    `SELECT id, job_type, status, virtual_path, attempts, error, updated_at
     FROM longhouse.ingest_jobs
     WHERE status IN ('failed', 'queued')
     ORDER BY updated_at DESC
     LIMIT 20`,
  );
  const recentSyncRuns = await db.query(
    `SELECT id, status, source_instance_id, scanned_count, changed_count, uploaded_count, error_count, error, finished_at, started_at
     FROM longhouse.source_sync_runs
     ORDER BY started_at DESC
     LIMIT 20`,
  ).catch(() => ({ rows: [] }));

  return {
    enabled: true,
    counts: { objects, chunks, embeddedChunks, pendingChunks, graphNodes, graphEdges, queuedJobs, failedJobs, staleFailedJobs, failedSyncRuns, sourceRecords },
    recentJobs: recentJobs.rows,
    recentSyncRuns: recentSyncRuns.rows,
  };
}

export function triageFindings(snapshot, ledgerVerify, semantic) {
  const failedAlert = Number(process.env.OCEAN_TRIAGE_FAILED_ALERT ?? 0);
  const queueAlert = Number(process.env.OCEAN_TRIAGE_QUEUE_ALERT ?? 50);
  const staleJobDays = Number(process.env.OCEAN_TRIAGE_STALE_JOB_DAYS || 3);
  const findings = [];
  if (ledgerVerify && ledgerVerify.ok === false) findings.push({ severity: 'high', area: 'ledger', message: 'Ocean Ledger hash-chain verification failed.' });
  if (snapshot.enabled) {
    if (snapshot.counts.failedJobs > failedAlert) findings.push({ severity: 'medium', area: 'ingest', message: `${snapshot.counts.failedJobs} ingest jobs are failed.` });
    if (snapshot.counts.staleFailedJobs > 0) findings.push({ severity: 'medium', area: 'ingest', message: `${snapshot.counts.staleFailedJobs} ingest jobs have been failed for >${staleJobDays} days without retry.` });
    if (snapshot.counts.queuedJobs > queueAlert) findings.push({ severity: 'medium', area: 'ingest', message: `${snapshot.counts.queuedJobs} ingest jobs are queued.` });
    if (snapshot.counts.pendingChunks > 0) findings.push({ severity: 'low', area: 'semantic', message: `${snapshot.counts.pendingChunks} chunks are not embedded/vectorized yet.` });
    if (snapshot.counts.failedSyncRuns > 0) findings.push({ severity: 'medium', area: 'sources', message: `${snapshot.counts.failedSyncRuns} source sync runs have failed.` });
    if (snapshot.counts.graphNodes === 0 && snapshot.counts.objects > 0) findings.push({ severity: 'low', area: 'graph', message: 'Objects exist but graph extraction has not populated nodes yet.' });
  }
  if (!semantic.enabled) findings.push({ severity: 'medium', area: 'semantic', message: 'Semantic environment is not fully configured.' });
  if (!findings.length) findings.push({ severity: 'info', area: 'ocean-context', message: 'No immediate triage issues detected.' });
  return findings;
}

function renderReport({ generatedAt, generatedBy, dbSnapshot, ledgerVerify, semantic, findings, recentLedgerEvents }) {
  return `# Ocean Context Daily Triage — ${dayStamp(new Date(generatedAt))}\n\nGenerated: ${generatedAt}  \nGenerated by: ${generatedBy || 'ocean-bedrock'}\n\n## Summary\n\n${findings.map((finding) => `- **${finding.severity.toUpperCase()}** [${finding.area}] ${finding.message}`).join('\n')}\n\n## Semantic / Vectorize\n\n\`\`\`json\n${json(semantic)}\n\`\`\`\n\n## Knowledge DB snapshot\n\n\`\`\`json\n${json(dbSnapshot)}\n\`\`\`\n\n## Ledger verification\n\n\`\`\`json\n${json(ledgerVerify || { skipped: true })}\n\`\`\`\n\n## Recent ledger events\n\n\`\`\`json\n${json(recentLedgerEvents || [])}\n\`\`\`\n\n## Operating notes\n\n- Ocean Ledger is the append-only timeline of events and context snapshots.\n- Ocean Context is the actionable layer built from files, chunks, vectors, graph edges, source records, and ledger traces.\n- Daily triage should detect stale indexing, failed source syncs, ledger integrity issues, failed ingest jobs, and missing toolbox/runtime propagation.\n`;
}

export async function runOceanContextTriage(state, principal = {}, options = {}) {
  const generatedAt = new Date().toISOString();
  const day = dayStamp(new Date(generatedAt));
  const slot = slotStamp(new Date(generatedAt));
  const seqKey = `triage:${day}:${slot}`;

  const db = getPool();

  if (!options.force && db) {
    const existing = await db.query(
      `SELECT id, virtual_path FROM longhouse.ledger_events
       WHERE source_id = 'ocean-bedrock' AND source_sequence = $1
       LIMIT 1`,
      [seqKey],
    );
    if (existing.rows.length) {
      const row = existing.rows[0];
      return { ok: true, skipped: 'already-ran-this-slot', reportPath: row.virtual_path, eventId: row.id };
    }
  }

  const [dbSnapshot, ledgerVerify, recentLedgerEvents] = await Promise.all([
    collectDbSnapshot(db),
    state.ledger?.verify ? state.ledger.verify().catch((error) => ({ ok: false, error: error.message })) : Promise.resolve(null),
    state.ledger?.read ? state.ledger.read({ limit: Number(options.ledgerLimit || 25) }).catch(() => []) : Promise.resolve([]),
  ]);
  const semantic = semanticStatus();
  const findings = triageFindings(dbSnapshot, ledgerVerify, semantic);
  const reportPath = options.reportPath || `/context/ocean-bedrock/triage/${day}.md`;
  const report = renderReport({ generatedAt, generatedBy: principal.name || principal.id || 'system', dbSnapshot, ledgerVerify, semantic, findings, recentLedgerEvents });

  const diskPath = path.join(state.filesRoot, reportPath.replace(/^\//, ''));
  await fs.mkdir(path.dirname(diskPath), { recursive: true });
  await fs.writeFile(diskPath, report, 'utf8');
  const stat = await fs.stat(diskPath);
  await recordObjectWrite(state, reportPath, { type: 'file', size: stat.size, etag: `"${stat.size.toString(16)}-${Math.floor(stat.mtimeMs).toString(16)}"` }, principal);

  const event = await state.ledger.append({
    event_type: 'ocean_context.triage.completed',
    correlation_id: options.correlationId || `cor-ocean-context-triage-${day}`,
    lab: 'ocean-context',
    actor_type: 'agent',
    actor_id: principal.id || 'ocean-bedrock',
    actor_name: principal.name || 'ocean-bedrock',
    source_id: 'ocean-bedrock',
    source_sequence: seqKey,
    virtual_path: reportPath,
    payload: { generatedAt, reportPath, findings, dbCounts: dbSnapshot.counts || null, semantic },
    clearance: options.clearance || 'CONFIDENTIAL',
    tags: ['ocean-context', 'triage', 'knowledge-base'],
  });

  await notifyTriage(state, { day, slot, findings, dbCounts: dbSnapshot.counts || null }, principal).catch((err) => {
    console.error('[triage] notification failed (non-fatal):', err.message);
  });

  return { ok: true, generatedAt, reportPath, findings, dbSnapshot, ledgerVerify, semantic, event };
}

export function toolboxManifest({ baseUrl = 'https://ocean-bedrock-production.up.railway.app', principal = null } = {}) {
  return {
    name: 'Ocean Toolbox',
    version: '0.1.0',
    generatedAt: new Date().toISOString(),
    bedrockUrl: baseUrl,
    purpose: 'Authorize a coworker computer for Ocean, install MCP access, sync opt-in context, and expose Rising Tides agent skills/toolbox safely.',
    auth: {
      model: 'scoped bearer token',
      recommendedRole: 'contributor',
      recommendedScopes: ['/coworkers/<person>', '/sessions/<person>', '/context/ocean-bedrock'],
      principal: principal ? { id: principal.id, name: principal.name, role: principal.role, scopes: principal.scopes } : null,
    },
    mcp: {
      serverName: 'ocean-bedrock',
      command: 'npm run mcp',
      env: { OCEAN_BEDROCK_URL: baseUrl, OCEAN_BEDROCK_TOKEN: '<scoped-token>' },
      // Keep in sync with the `tools` array in scripts/ocean-bedrock-mcp.mjs
      // (pinned by test/toolbox-manifest.test.mjs).
      tools: ['bedrock_info', 'bedrock_list', 'bedrock_read', 'bedrock_write', 'bedrock_mkdir', 'bedrock_search', 'bedrock_semantic_search', 'bedrock_graph_neighborhood', 'bedrock_toolbox_manifest', 'bedrock_triage_daily', 'bedrock_lock', 'bedrock_unlock', 'bedrock_trace', 'bedrock_snapshot'],
      resources: ['ocean-bedrock://docs', 'ocean-bedrock://context', 'ocean-bedrock://coworkers', 'ocean-bedrock://sessions', 'ocean-bedrock://handoffs'],
    },
    localCompanion: {
      run: 'npm run ocean:app',
      configPath: '~/.config/ocean-bedrock/bootstrap.json',
      statePath: '~/.local/state/ocean-bedrock/ingest-state.json',
      behavior: 'User picks opt-in folders; app syncs bytes over HTTP and lineage through source/sync endpoints.',
    },
    skills: [
      { name: 'writers-room', purpose: 'drafting, editorial workflows, content synthesis' },
      { name: 'cloudflare', purpose: 'Workers, R2, DNS, Vectorize/Workers AI operations' },
      { name: 'use-railway', purpose: 'Railway deploys, services, databases, logs' },
      { name: 'telegram', purpose: 'Telegram bot and notification workflows' },
      { name: 'brave-search', purpose: 'current web research and documentation lookup' },
      { name: 'pi-messenger-crew', purpose: 'multi-agent task planning, handoffs, work waves' },
    ],
    stagedFlow: [
      'Operator issues scoped token or invite for coworker/device.',
      'Coworker downloads/opens local companion GUI.',
      'Device stores token locally outside the repo and registers selected folders as source streams.',
      'Ocean Bedrock indexes text chunks, embeds to Vectorize, extracts graph links, and records ledger events.',
      'Agents use MCP/tools/context snapshots to act on behalf of the coworker within token scope.',
    ],
  };
}
