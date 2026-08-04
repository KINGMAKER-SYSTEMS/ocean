#!/usr/bin/env node
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { spawn } from 'node:child_process';

const repoRoot = path.resolve(new URL('..', import.meta.url).pathname);

async function waitForUrl(child) {
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error(`local app did not start in time: ${output}`)), 15_000);
    const onData = (chunk) => {
      output += chunk.toString();
      const match = output.match(/http:\/\/127\.0\.0\.1:(\d+)\//);
      if (match) {
        clearTimeout(timer);
        resolve(`http://127.0.0.1:${match[1]}`);
      }
    };
    child.stdout.on('data', onData);
    child.stderr.on('data', onData);
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('exit', (code) => {
      if (!output.match(/http:\/\/127\.0\.0\.1:(\d+)\//)) {
        clearTimeout(timer);
        reject(new Error(`local app exited before start: code=${code} output=${output}`));
      }
    });
  });
}

async function api(base, pathname, body = undefined, expectedStatus = 200) {
  const response = await fetch(`${base}${pathname}`, body === undefined ? {} : {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await response.json();
  if (response.status !== expectedStatus) {
    throw new Error(`${pathname} returned ${response.status}, expected ${expectedStatus}: ${JSON.stringify(data)}`);
  }
  return data;
}

const root = await fs.mkdtemp(path.join(os.tmpdir(), 'ocean-local-app-ack-'));
const source = path.join(root, 'source');
const config = path.join(root, 'bootstrap.json');
const state = path.join(root, 'app-state.json');
const ingestState = path.join(root, 'ingest-state.json');
await fs.mkdir(source, { recursive: true });
await fs.writeFile(path.join(source, 'notes.md'), '# safe note\n', 'utf8');
await fs.writeFile(path.join(source, '.env'), 'SHOULD_NOT_UPLOAD=true\n', 'utf8');
await fs.writeFile(config, `${JSON.stringify({
  version: 1,
  serverUrl: 'http://127.0.0.1:1',
  token: 'dummy-token-for-ack-check',
  ownerName: 'ack-check',
  ownerSlug: 'ack-check',
  deviceName: 'test-device',
  deviceSlug: 'test-device',
  remoteRoot: '/coworkers/ack-check/test-device',
  feedMode: 'knowledge-docs',
  allowedExtensions: ['.md'],
  defaultIgnores: [],
  maxFileBytes: 10 * 1024 * 1024,
  sources: [{
    id: 'src_ack_check',
    label: 'source',
    localPath: source,
    remotePrefix: '/coworkers/ack-check/test-device/source',
    enabled: true,
  }],
  schedule: { enabled: false, intervalMinutes: 60 },
}, null, 2)}\n`, { mode: 0o600 });

const child = spawn(process.execPath, [
  path.join(repoRoot, 'scripts/ocean-local-app.mjs'),
  '--config', config,
  '--state', state,
  '--ingest-state', ingestState,
  '--port', '0',
  '--no-open',
], { cwd: repoRoot, stdio: ['ignore', 'pipe', 'pipe'] });

try {
  const base = await waitForUrl(child);

  const blocked = await api(base, '/api/sync', {}, 409);
  if (blocked.code !== 'PREVIEW_ACK_REQUIRED') throw new Error(`expected PREVIEW_ACK_REQUIRED before preview, got ${JSON.stringify(blocked)}`);

  const preview = await api(base, '/api/preview', {});
  if (!preview.preview?.ok) throw new Error(`preview failed: ${JSON.stringify(preview)}`);
  if (!preview.preview?.result?.totals?.changed) throw new Error(`preview did not detect changed file: ${JSON.stringify(preview.preview?.result)}`);

  const stillBlocked = await api(base, '/api/sync', {}, 409);
  if (stillBlocked.code !== 'PREVIEW_ACK_REQUIRED') throw new Error(`expected PREVIEW_ACK_REQUIRED before ack, got ${JSON.stringify(stillBlocked)}`);

  const ack = await api(base, '/api/preview/ack', {});
  if (!ack.state?.syncUnlocked) throw new Error(`ack did not unlock current plan: ${JSON.stringify(ack)}`);

  const schedule = await api(base, '/api/schedule', { enabled: true, intervalMinutes: 5 });
  if (!schedule.schedule?.enabled) throw new Error(`schedule did not enable after ack: ${JSON.stringify(schedule)}`);

  console.log(JSON.stringify({ ok: true, blockedBeforePreview: true, blockedBeforeAck: true, syncUnlockedAfterAck: true }, null, 2));
} finally {
  child.kill('SIGTERM');
  setTimeout(() => child.kill('SIGKILL'), 1000).unref();
  await fs.rm(root, { recursive: true, force: true });
}
