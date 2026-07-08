#!/usr/bin/env node
// Verifies the Ocean local app ceremony protocol (Build Lane 1).
//
// Starts the local app on an ephemeral port with isolated state, creates a
// ceremony run, posts two events, connects over WebSocket, verifies the
// snapshot + a live event frame, then completes the run. Exits 0 and prints
// the expected ok JSON on success.
//
// See docs/superpowers/plans/2026-07-07-ocean-skill-onboarding-ceremony.md.
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { spawn } from 'node:child_process';
import { WebSocket } from 'ws';

const repoRoot = path.resolve(new URL('..', import.meta.url).pathname);

async function waitForUrl(child) {
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(
      () => reject(new Error(`local app did not start in time: ${output}`)),
      20_000,
    );
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

async function postJson(base, pathname, body) {
  const response = await fetch(`${base}${pathname}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
  const data = await response.json();
  if (!response.ok || !data.ok) {
    throw new Error(`${pathname} failed (${response.status}): ${JSON.stringify(data)}`);
  }
  return data;
}

// Connects a ceremony WS client and exposes waitForFrame(predicate).
function connectCeremonyWs(base, runId, after = 0) {
  const wsUrl = `${base.replace(/^http/, 'ws')}/api/ceremony/ws?runId=${encodeURIComponent(runId)}&after=${after}`;
  const ws = new WebSocket(wsUrl);
  const queue = [];
  const pending = [];
  ws.on('message', (raw) => {
    let frame;
    try {
      frame = JSON.parse(raw.toString());
    } catch {
      return;
    }
    const matchIdx = pending.findIndex(([pred]) => pred(frame));
    if (matchIdx >= 0) {
      const [, resolve] = pending.splice(matchIdx, 1)[0];
      resolve(frame);
    } else {
      queue.push(frame);
    }
  });
  const opened = new Promise((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  const waitForFrame = (pred, timeoutMs = 8000) => new Promise((resolve, reject) => {
    const existingIdx = queue.findIndex((f) => pred(f));
    if (existingIdx >= 0) {
      resolve(queue.splice(existingIdx, 1)[0]);
      return;
    }
    const timer = setTimeout(
      () => reject(new Error(`timed out waiting for frame matching ${pred}`)),
      timeoutMs,
    );
    pending.push([pred, (frame) => {
      clearTimeout(timer);
      resolve(frame);
    }]);
  });
  const close = () => {
    try { ws.close(); } catch { /* ignore */ }
  };
  return { ws, opened, waitForFrame, close };
}

const tmpRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'ocean-ceremony-protocol-'));
const config = path.join(tmpRoot, 'bootstrap.json');
const state = path.join(tmpRoot, 'app-state.json');
const ingestState = path.join(tmpRoot, 'ingest-state.json');
await fs.writeFile(
  config,
  `${JSON.stringify({
    ownerLogin: 'protocol-check',
    ownerName: 'protocol-check',
    serverUrl: 'https://ocean-bedrock-production.up.railway.app',
    folders: [],
    schedule: { enabled: false, intervalMinutes: 60 },
  }, null, 2)}\n`,
  { mode: 0o600 },
);

const child = spawn(process.execPath, [
  path.join(repoRoot, 'scripts/ocean-local-app.mjs'),
  '--config', config,
  '--state', state,
  '--ingest-state', ingestState,
  '--port', '0',
  '--no-open',
], { cwd: repoRoot, stdio: ['ignore', 'pipe', 'pipe'] });

let createdRun = false;
let postedEvents = 0;
let wsReceived = false;

async function run() {
  const base = await waitForUrl(child);

  // 1. Create a ceremony run.
  const create = await postJson(base, '/api/ceremony/runs', {
    runner: { runtime: 'omp', name: 'protocol-check' },
    ownerName: 'protocol-check',
    mode: 'onboarding-import-v1',
  });
  if (!create.run?.runId || create.run.status !== 'running' || create.run.events !== 0) {
    throw new Error(`unexpected create response: ${JSON.stringify(create)}`);
  }
  createdRun = true;
  const { runId } = create.run;

  // 2. Post one event before connecting (exercises single-event path + server seq/ts).
  const e1 = await postJson(base, `/api/ceremony/runs/${runId}/events`, {
    type: 'agent.source_detected',
    phase: 'scan',
    status: 'complete',
    message: 'Claude Code setup detected',
    seq: 999, // runner seq must be ignored
    ts: '1999-01-01T00:00:00.000Z', // runner ts demoted to payload.runnerTs
    payload: { sourceId: 'claude-code', label: 'Claude Code', counts: { skills: 1 } },
  });
  if (e1.accepted !== 1 || e1.nextSeq !== 2) {
    throw new Error(`unexpected e1 response: ${JSON.stringify(e1)}`);
  }
  postedEvents += e1.accepted;

  // 3. Connect WS and verify hello + snapshot contain E1 with a server-assigned seq.
  const conn = connectCeremonyWs(base, runId, 0);
  await conn.opened;
  const hello = await conn.waitForFrame((f) => f.kind === 'hello');
  if (hello.schema !== 'ocean.ceremony.ws.v1' || hello.runId !== runId) {
    throw new Error(`unexpected hello frame: ${JSON.stringify(hello)}`);
  }
  const snapshot = await conn.waitForFrame((f) => f.kind === 'snapshot');
  if (snapshot.runId !== runId || snapshot.lastSeq !== 1) {
    throw new Error(`unexpected snapshot: ${JSON.stringify(snapshot)}`);
  }
  if (!Array.isArray(snapshot.events) || snapshot.events.length !== 1) {
    throw new Error(`snapshot expected 1 event, got ${snapshot.events?.length}`);
  }
  const snap = snapshot.events[0];
  if (snap.seq !== 1 || snap.type !== 'agent.source_detected' || snap.schema !== 'ocean.ceremony.event.v1') {
    throw new Error(`snapshot event mismatch: ${JSON.stringify(snap)}`);
  }
  // Server reassigned seq (1) and ts; runner ts preserved only as payload.runnerTs.
  if (snap.payload?.runnerTs !== '1999-01-01T00:00:00.000Z') {
    throw new Error(`runner ts not preserved as payload.runnerTs: ${JSON.stringify(snap.payload)}`);
  }
  if (snap.ts === '1999-01-01T00:00:00.000Z') {
    throw new Error('server did not reassign ts');
  }

  // 4. Post a second event live and verify the broadcast event frame arrives.
  const e2 = await postJson(base, `/api/ceremony/runs/${runId}/events`, {
    type: 'agent.skill_detected',
    phase: 'scan',
    status: 'progress',
    message: 'Skill found: axum',
    payload: { sourceId: 'claude-code', name: 'axum' },
  });
  if (e2.accepted !== 1 || e2.nextSeq !== 3) {
    throw new Error(`unexpected e2 response: ${JSON.stringify(e2)}`);
  }
  postedEvents += e2.accepted;

  const live = await conn.waitForFrame((f) => f.kind === 'event' && f.event?.seq === 2);
  if (live.runId !== runId || live.event.type !== 'agent.skill_detected') {
    throw new Error(`unexpected live frame: ${JSON.stringify(live)}`);
  }
  wsReceived = true;
  conn.close();

  // 5. GET replay buffer returns both renderable events in order.
  const replayRes = await fetch(`${base}/api/ceremony/runs/${runId}`);
  const replay = await replayRes.json();
  if (!replay.ok || replay.run.lastSeq !== 2 || replay.events.length !== 2) {
    throw new Error(`replay mismatch: ${JSON.stringify(replay)}`);
  }
  if (replay.events[0].seq !== 1 || replay.events[1].seq !== 2) {
    throw new Error(`replay ordering mismatch: ${JSON.stringify(replay.events)}`);
  }

  // 6. Private events are stored but never rendered (not in replay, not in snapshot).
  await postJson(base, `/api/ceremony/runs/${runId}/events`, {
    type: 'ceremony.toast',
    phase: 'scan',
    status: 'info',
    visibility: 'private',
    message: 'internal-only',
    payload: { secret: 'SHOULD_NOT_RENDER' },
  });
  const replayAfterPrivate = await fetch(`${base}/api/ceremony/runs/${runId}`).then((r) => r.json());
  if (replayAfterPrivate.events.some((e) => e.visibility === 'private' || e.payload?.secret)) {
    throw new Error(`private payload leaked into replay: ${JSON.stringify(replayAfterPrivate.events)}`);
  }

  // 7. Complete the run; server broadcasts run_completed (broadcast: true).
  const complete = await postJson(base, `/api/ceremony/runs/${runId}/complete`, {});
  if (!complete.ok || complete.run.status !== 'completed' || complete.broadcast !== true) {
    throw new Error(`unexpected complete response: ${JSON.stringify(complete)}`);
  }
  // Completing again must not rebroadcast (broadcast: false).
  const completeAgain = await postJson(base, `/api/ceremony/runs/${runId}/complete`, {});
  if (completeAgain.broadcast !== false) {
    throw new Error(`expected broadcast:false on second complete, got ${JSON.stringify(completeAgain)}`);
  }

  // 8. Unknown run id is rejected.
  const missingRes = await fetch(`${base}/api/ceremony/runs/cer_does_not_exist`);
  if (missingRes.status !== 404) {
    throw new Error(`expected 404 for unknown run, got ${missingRes.status}`);
  }

  return { ok: true, createdRun, postedEvents, wsReceived };
}

try {
  const result = await run();
  if (!result.createdRun || result.postedEvents !== 2 || !result.wsReceived) {
    throw new Error(`assertion failed: ${JSON.stringify(result)}`);
  }
  console.log(JSON.stringify({
    ok: true,
    createdRun: result.createdRun,
    postedEvents: result.postedEvents,
    wsReceived: result.wsReceived,
  }, null, 2));
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: error.message, stack: error.stack }, null, 2));
  process.exitCode = 1;
} finally {
  try { child.kill('SIGTERM'); } catch { /* ignore */ }
  try { await fs.rm(tmpRoot, { recursive: true, force: true }); } catch { /* ignore */ }
}
