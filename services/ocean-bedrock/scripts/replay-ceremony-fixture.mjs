#!/usr/bin/env node
// Ocean Bedrock Coworker Onboarding — ceremony fixture replay.
//
// Streams the fixture ceremony event timeline into the Ocean local app using
// the `ocean.ceremony.event.v1` protocol. Simulates a skill runner without
// requiring a real Claude Code / Codex / Pi setup.
//
// Usage:
//   node scripts/replay-ceremony-fixture.mjs --dry-run
//   node scripts/replay-ceremony-fixture.mjs --base http://127.0.0.1:8765
//   node scripts/replay-ceremony-fixture.mjs --runtime codex --name "Codex"
//   node scripts/replay-ceremony-fixture.mjs --simulate-ack --run-sync
//
// Modes:
//   --dry-run         Print each event as JSONL to stdout. No network. Exits 0
//                     after self-verifying acceptance counts.
//   (default, live)   POST /api/ceremony/runs, stream frames to
//                     /api/ceremony/runs/:runId/events, then complete the run.
//
// Human ack gate (default, live):
//   After emitting `ceremony.await_user gate=preview_ack`, the replay polls
//   `GET /api/state` until `syncUnlocked === true`, i.e. a real human clicks
//   the unlock button in the Ocean stage. The replay NEVER clicks it for them.
//   Use --ack-timeout <sec> to bound the wait (default 300s); 0 = wait forever.
//
// Fixture-only conveniences (NOT part of the skill, demo/CI only):
//   --simulate-ack    Drive POST /api/preview then POST /api/preview/ack to
//                     unlock sync without a human (best-effort; ignored if the
//                     app is not configured with folders).
//   --run-sync        Actually call POST /api/sync after unlock. Without this,
//                     sync is represented by ceremony events only.
//
// This file is fixture tooling. The standardized SKILL never auto-acks.

import { buildFixtureFrames, meetsAcceptance, ACCEPTANCE_MINIMUMS } from './ocean-ceremony-fixture.mjs';

const ALLOWED_RUNTIMES = new Set(['claude-code', 'codex', 'pi', 'omp', 'other']);

function parseArgs(argv) {
  const args = {
    dryRun: false,
    base: 'http://127.0.0.1:8765',
    runtime: 'other',
    name: 'Ocean Fixture Replay',
    owner: 'ocean-coworker',
    delayMs: 120,
    ackTimeoutSec: 300,
    simulateAck: false,
    runSync: false,
    complete: true,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => argv[i + 1];
    switch (a) {
      case '--dry-run': args.dryRun = true; break;
      case '--base': args.base = next(); i += 1; break;
      case '--runtime': args.runtime = next(); i += 1; break;
      case '--name': args.name = next(); i += 1; break;
      case '--owner': args.owner = next(); i += 1; break;
      case '--delay': args.delayMs = Number(next()); i += 1; break;
      case '--no-delay': args.delayMs = 0; break;
      case '--ack-timeout': args.ackTimeoutSec = Number(next()); i += 1; break;
      case '--simulate-ack': args.simulateAck = true; break;
      case '--run-sync': args.runSync = true; break;
      case '--no-complete': args.complete = false; break;
      case '-h':
      case '--help': {
        const help = [
          'Ocean ceremony fixture replay',
          '',
          'Options:',
          '  --dry-run              Print events as JSONL, no network',
          '  --base <url>           Local app base URL (default http://127.0.0.1:8765)',
          '  --runtime <r>          claude-code|codex|pi|omp|other (default other)',
          '  --name <name>          Runner display name',
          '  --owner <name>         Owner name for the ceremony run',
          '  --delay <ms>           Inter-frame delay (default 120; --no-delay = 0)',
          '  --ack-timeout <sec>    Max seconds to wait for human ack (default 300; 0 = forever)',
          '  --simulate-ack         Fixture-only: auto-drive preview/ack to unlock',
          '  --run-sync             Fixture-only: actually call POST /api/sync',
          '  --no-complete          Do not POST /api/ceremony/runs/:runId/complete',
          '  -h, --help             Show this help',
        ].join('\n');
        console.log(help);
        process.exit(0);
      }
      default:
        console.error(`Unknown argument: ${a}`);
        process.exit(2);
    }
  }
  if (!ALLOWED_RUNTIMES.has(args.runtime)) {
    console.error(`Invalid --runtime: ${args.runtime}. Must be one of: ${[...ALLOWED_RUNTIMES].join(', ')}`);
    process.exit(2);
  }
  if (!args.base) {
    console.error('--base is required');
    process.exit(2);
  }
  return args;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function postJSON(url, body) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  let json = null;
  if (text) {
    try { json = JSON.parse(text); } catch { json = null; }
  }
  if (!res.ok) {
    const err = new Error(`POST ${url} -> ${res.status}: ${text || res.statusText}`);
    err.status = res.status;
    err.body = json;
    throw err;
  }
  return json;
}

async function getJSON(url) {
  const res = await fetch(url);
  const text = await res.text();
  let json = null;
  if (text) {
    try { json = JSON.parse(text); } catch { json = null; }
  }
  if (!res.ok) {
    const err = new Error(`GET ${url} -> ${res.status}: ${text || res.statusText}`);
    err.status = res.status;
    throw err;
  }
  return json;
}

function isPreviewAckFrame(frame) {
  const events = frame.kind === 'single' ? [frame.event] : frame.events;
  return events.some(
    (e) => e.type === 'ceremony.await_user' && e.payload && e.payload.gate === 'preview_ack',
  );
}

function isSyncResumeFrame(frame) {
  // The frame emitted right after the human ack is observed. The replay waits
  // for syncUnlocked BEFORE posting this frame.
  return frame.kind === 'single' && frame.event && frame.event.type === 'ocean.ack_observed';
}

async function waitForSyncUnlocked(base, timeoutSec, onTick) {
  const deadline = timeoutSec > 0 ? Date.now() + timeoutSec * 1000 : Infinity;
  let lastMsg = 0;
  for (;;) {
    let state;
    try {
      state = await getJSON(`${base}/api/state`);
    } catch (err) {
      if (Date.now() > deadline) throw new Error(`ack wait aborted; /api/state error: ${err.message}`);
      await sleep(2000);
      continue;
    }
    if (state && state.syncUnlocked === true) return true;
    if (Date.now() > deadline) return false;
    if (onTick && Date.now() - lastMsg > 4000) {
      lastMsg = Date.now();
      onTick(state, Math.max(0, deadline - Date.now()));
    }
    await sleep(1500);
  }
}

async function bestEffortUnlock(base) {
  // Fixture-only convenience: drive the existing dry-run preview + ack so a
  // demo can complete hands-free. Returns true if sync appears unlocked.
  try {
    await postJSON(`${base}/api/preview`, {});
  } catch (err) {
    console.error(`  [simulate-ack] preview not available (${err.message}); continuing`);
  }
  try {
    await postJSON(`${base}/api/preview/ack`, {});
  } catch (err) {
    console.error(`  [simulate-ack] ack not available (${err.message}); continuing`);
  }
  try {
    const state = await getJSON(`${base}/api/state`);
    return Boolean(state && state.syncUnlocked);
  } catch {
    return false;
  }
}

async function dryRun({ frames, counts }) {
  if (!meetsAcceptance({ frames, counts })) {
    console.error('Fixture does not meet Lane 3 acceptance minimums.');
    process.exit(1);
  }
  for (const frame of frames) {
    if (frame.kind === 'single') {
      process.stdout.write(`${JSON.stringify(frame.event)}\n`);
    } else {
      for (const event of frame.events) {
        process.stdout.write(`${JSON.stringify(event)}\n`);
      }
    }
  }
  console.error('\n[dry-run] Event counts:');
  for (const [type, n] of Object.entries(counts).sort()) {
    console.error(`  ${type}: ${n}`);
  }
  console.error('[dry-run] Acceptance minimums satisfied. No network calls made.');
}

async function liveRun(args, built) {
  const { frames, counts, runner, ownerName } = built;

  // 1. Create the ceremony run.
  const created = await postJSON(`${args.base}/api/ceremony/runs`, {
    runner,
    ownerName,
    mode: 'onboarding-import-v1',
  });
  if (!created || !created.ok || !created.run || !created.run.runId) {
    throw new Error(`Failed to create ceremony run: ${JSON.stringify(created)}`);
  }
  const { runId } = created.run;
  console.error(`[live] Ceremony run created: ${runId}`);

  const eventsUrl = `${args.base}/api/ceremony/runs/${runId}/events`;
  let accepted = 0;
  let waitingForAck = false;

  for (const frame of frames) {
    // Pause at the human ack gate: do not post the await_user frame until we
    // are about to actually wait (we post it first, then wait, then resume).
    if (isPreviewAckFrame(frame)) {
      waitingForAck = true;
    }

    const body = frame.kind === 'single' ? frame.event : { events: frame.events };
    const res = await postJSON(eventsUrl, body);
    if (!res || !res.ok) {
      throw new Error(`Event POST failed: ${JSON.stringify(res)}`);
    }
    accepted += res.accepted || (frame.kind === 'single' ? 1 : frame.events.length);
    const summary = frame.kind === 'single'
      ? frame.event.type
      : `batch(${frame.events.length}x ${frame.events[0] && frame.events[0].type})`;
    console.error(`[live] -> ${summary}  (accepted=${res.accepted}, nextSeq=${res.nextSeq})`);

    // After posting the await_user gate, block until the human unlocks sync
    // (or the fixture convenience unlocks it).
    if (waitingForAck) {
      waitingForAck = false;
      console.error('[live] Paused at preview ack gate. Waiting for human acknowledgement...');
      if (args.simulateAck) {
        console.error('[live] --simulate-ack set: driving preview/ack to unlock (fixture convenience).');
        const ok = await bestEffortUnlock(args.base);
        if (!ok) {
          console.error('[live] simulate-ack did not unlock via /api/state; will still wait/poll.');
        }
      }
      const unlocked = await waitForSyncUnlocked(args.base, args.ackTimeoutSec, (state, remainingMs) => {
        const remain = args.ackTimeoutSec > 0
          ? Math.round(remainingMs / 1000)
          : null;
        console.error(`[live]   still waiting for syncUnlocked=true (currently=${state && state.syncUnlocked}${remain !== null ? `, ${remain}s left` : ''})...`);
      });
      if (!unlocked) {
        console.error('[live] Ack timeout reached. Continuing to completion without a confirmed human ack (ceremony events only).');
      } else {
        console.error('[live] syncUnlocked=true detected. Resuming after ack gate.');
      }
    }

    // Before posting the post-ack resume frame, optionally perform a real sync.
    if (isSyncResumeFrame(frame) && args.runSync) {
      try {
        const syncRes = await postJSON(`${args.base}/api/sync`, {});
        console.error(`[live] --run-sync: POST /api/sync ok=${syncRes && syncRes.ok}`);
      } catch (err) {
        console.error(`[live] --run-sync: POST /api/sync failed (${err.message}); continuing`);
      }
    }

    if (args.delayMs > 0) await sleep(args.delayMs);
  }

  // 2. Complete the run.
  if (args.complete) {
    try {
      const done = await postJSON(`${args.base}/api/ceremony/runs/${runId}/complete`, {});
      console.error(`[live] Run completed: ok=${done && done.ok}, broadcast=${done && done.broadcast}`);
    } catch (err) {
      console.error(`[live] complete call failed: ${err.message}`);
    }
  }

  console.error(`\n[live] Done. runId=${runId} accepted=${accepted}`);
  console.error('[live] Event counts:');
  for (const [type, n] of Object.entries(counts).sort()) {
    console.error(`  ${type}: ${n}`);
  }
  if (!meetsAcceptance({ frames, counts })) {
    console.error('[live] WARNING: fixture did not meet Lane 3 acceptance minimums.');
    process.exitCode = 1;
  }
}


async function main() {
  const args = parseArgs(process.argv.slice(2));
  const built = buildFixtureFrames({
    runner: { runtime: args.runtime, name: args.name },
    ownerName: args.owner,
  });


  if (args.dryRun) {
    await dryRun(built);
    return;
  }

  console.error(`[live] Streaming ceremony fixture -> ${args.base}`);
  console.error(`[live] runtime=${args.runtime} owner=${args.owner} frames=${built.frames.length}`);
  try {
    await liveRun(args, built);
  } catch (err) {
    if (err && (err.code === 'ECONNREFUSED' || /\bECONNREFUSED\b/.test(err.message))) {
      console.error(`\n[live] Could not reach ${args.base}. Is the local app running?`);
      console.error('[live] Start it with:  npm run ocean:app');
      console.error('[live] Or run in dry-run mode:  node scripts/replay-ceremony-fixture.mjs --dry-run');
      process.exit(1);
    }
    console.error(`\n[live] Error: ${err.message}`);
    process.exit(1);
  }
}

main();
