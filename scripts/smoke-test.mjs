#!/usr/bin/env node
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';

import { createOceanBedrockServer } from '../src/server.mjs';

const root = await fs.mkdtemp(path.join(os.tmpdir(), 'ocean-bedrock-smoke-'));
const sourceRegistryDatabaseUrl = process.env.DATABASE_URL || null;
// Keep the local HTTP smoke fully ephemeral. DB-backed schema checks are read-only below.
delete process.env.DATABASE_URL;
process.env.OCEAN_BEDROCK_BOOTSTRAP_TOKEN = 'smoke-admin-token';
process.env.OCEAN_LEDGER_STORE = 'jsonl';

const { server, ready } = createOceanBedrockServer({ root, instance: 'smoke-test' });
await ready;
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
const base = `http://127.0.0.1:${address.port}`;
const token = 'smoke-admin-token';

async function verifySourceAdapterTables(databaseUrl) {
  if (!databaseUrl) return null;
  const { Pool } = await import('pg');
  const pool = new Pool({ connectionString: databaseUrl });
  try {
    const result = await pool.query(`
      SELECT
        to_regclass('longhouse.source_adapters') AS source_adapters_table,
        to_regclass('longhouse.source_instances') AS source_instances_table,
        to_regclass('longhouse.source_streams') AS source_streams_table,
        to_regclass('longhouse.source_sync_runs') AS source_sync_runs_table,
        to_regclass('longhouse.source_records') AS source_records_table
    `);
    const row = result.rows[0];
    const ready = Boolean(
      row.source_adapters_table
      && row.source_instances_table
      && row.source_streams_table
      && row.source_sync_runs_table
      && row.source_records_table,
    );
    let adapters = 0;
    if (ready) {
      const count = await pool.query(`
        SELECT count(*)::int AS count
        FROM longhouse.source_adapters
        WHERE id = ANY($1::text[])
      `, [[
        'local_folder',
        'github',
        'telegram',
        'slack',
        'notion',
        'linear',
        'google_drive',
        'r2',
      ]]);
      adapters = count.rows[0]?.count || 0;
    }
    return { ready, adapters };
  } finally {
    await pool.end();
  }
}

async function api(pathname, options = {}) {
  const response = await fetch(`${base}${pathname}`, {
    ...options,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(options.headers || {}),
    },
  });
  const contentType = response.headers.get('content-type') || '';
  const body = contentType.includes('application/json') ? await response.json() : await response.text();
  if (!response.ok) {
    throw new Error(`${options.method || 'GET'} ${pathname} failed: ${response.status} ${JSON.stringify(body)}`);
  }
  return { response, body };
}

try {
  await api('/api/v1/info');
  const toolbox = await api('/api/v1/toolbox/manifest');
  if (!toolbox.body.mcp?.tools?.includes('bedrock_semantic_search')) throw new Error('toolbox manifest missing semantic MCP tool');
  const noDbSources = await fetch(`${base}/api/v1/sources/adapters`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (noDbSources.status !== 503) throw new Error(`expected source adapters to require DB in ephemeral smoke, got ${noDbSources.status}`);
  const noDbSemantic = await fetch(`${base}/api/v1/semantic/search?q=hello&path=/docs`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (noDbSemantic.status !== 503) throw new Error(`expected semantic search to require DB in ephemeral smoke, got ${noDbSemantic.status}`);
  await api('/api/v1/mkdir', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: '/docs/smoke' }),
  });
  await api('/api/v1/file?path=/docs/smoke/hello.md', {
    method: 'PUT',
    headers: { 'content-type': 'text/markdown' },
    body: '# hello\n\nshared filesystem works\n',
  });
  const readBack = await api('/api/v1/file?path=/docs/smoke/hello.md&inline=1');
  if (!String(readBack.body).includes('shared filesystem works')) throw new Error('readback did not match');
  const lock = await api('/api/v1/locks', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: '/docs/smoke/hello.md', note: 'smoke test' }),
  });
  await api(`/api/v1/locks/${lock.body.id}`, { method: 'DELETE' });
  await api('/api/v1/ledger/events', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      event_type: 'decision.recorded',
      correlation_id: 'cor-smoke-001',
      lab: 'smoke',
      virtual_path: '/docs/smoke/hello.md',
      payload: { decision: 'Smoke test ledger event' },
      clearance: 'UNCLASSIFIED',
    }),
  });
  await api('/api/v1/ledger/snapshots', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      name: 'smoke-context',
      correlation_id: 'cor-smoke-001',
      virtual_path: '/docs/smoke/hello.md',
      files: ['/docs/smoke/hello.md'],
      events: [],
      summary: 'Smoke test context snapshot',
      clearance: 'CONFIDENTIAL',
    }),
  });
  const trace = await api('/api/v1/ledger/trace?correlation_id=cor-smoke-001');
  if (trace.body.events.length < 2) throw new Error('ledger trace returned too few events');
  const verify = await api('/api/v1/ledger/verify');
  if (!verify.body.ok) throw new Error(`ledger verify failed: ${JSON.stringify(verify.body.errors)}`);
  const search = await api('/api/v1/search?q=shared&path=/docs');
  if (!search.body.results.length) throw new Error('search returned no results');
  await api('/api/v1/tokens', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'smoke-agent', role: 'agent', scopes: ['/docs'], ttlDays: 1 }),
  });
  const invite = await api('/api/v1/invites', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'smoke-coworker', role: 'contributor', scopes: ['/docs'], ttlMinutes: 5 }),
  });
  if (!invite.body.code || invite.body.invite?.hash) throw new Error('invite create did not return safe one-time code payload');
  const redeemedResponse = await fetch(`${base}/api/v1/invites/redeem`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ code: invite.body.code }),
  });
  const redeemed = await redeemedResponse.json();
  if (!redeemedResponse.ok || !redeemed.token || redeemed.record?.role !== 'contributor') throw new Error(`invite redeem failed: ${JSON.stringify(redeemed)}`);
  const reused = await fetch(`${base}/api/v1/invites/redeem`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ code: invite.body.code }),
  });
  if (reused.status !== 403) throw new Error(`expected invite reuse to fail with 403, got ${reused.status}`);
  const sourceRegistry = await verifySourceAdapterTables(sourceRegistryDatabaseUrl);
  if (sourceRegistry && (!sourceRegistry.ready || sourceRegistry.adapters !== 8)) {
    throw new Error(`source adapter schema check failed: ${JSON.stringify(sourceRegistry)}`);
  }
  // Gate-2 F3: with file auth + JSONL ledger, EVERY federated room surface is
  // an explicit 503 (no URL leakage) while all the ordinary local features
  // exercised above stayed green.
  const federatedAttempts = [
    ['POST', '/api/v1/rooms/smokeroom/register', '{}'],
    ['POST', '/api/v1/rooms/smokeroom/members/self', undefined],
    ['POST', '/api/v1/rooms/smokeroom/members/agents', JSON.stringify({ agents: [{ display_name: 'x', registration_key: 'k' }] })],
    ['GET', '/api/v1/rooms/smokeroom/members', undefined],
    ['DELETE', '/api/v1/rooms/smokeroom/members/9c2f4a52-0000-4000-8000-000000000001', undefined],
    ['POST', '/api/v1/invites', JSON.stringify({ room_id: 'smokeroom' })],
  ];
  for (const [method, apiPath, body] of federatedAttempts) {
    const response = await fetch(`${base}${apiPath}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body,
    });
    if (response.status !== 503) throw new Error(`expected F3 503 for ${method} ${apiPath}, got ${response.status}`);
    const payload = await response.text();
    if (payload.includes('postgres://')) throw new Error(`F3 503 leaked a database URL on ${method} ${apiPath}`);
  }
  console.log('ocean-bedrock smoke test passed (incl. F3 federated 503s)');
} finally {
  await new Promise((resolve) => server.close(resolve));
  await fs.rm(root, { recursive: true, force: true });
}
