// Gate-2 S1C DB-free protocol tests — the state machine runs against
// injected fake adapters and manual timers; no sockets, no database. Server
// gate composition and real-stream races are proven in the S1C section of
// scripts/rooms-postgres-smoke.mjs (slice 2).
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  selectCursor,
  createRoomEventStream,
  helloFrame,
  resyncFrame,
  QUEUE_BOUND,
  PAGE_LIMIT,
} from '../src/room-events.mjs';
import { LedgerValidationError } from '../src/ledger.mjs';

// ── manual timers ─────────────────────────────────────────────────────

async function microdrain(turns = 30) {
  for (let i = 0; i < turns; i += 1) await Promise.resolve();
}

class FakeTimers {
  constructor() { this.next = 1; this.pending = new Map(); }
  setTimeout = (fn, ms) => { const id = this.next++; this.pending.set(id, { fn, ms }); return id; };
  clearTimeout = (id) => { this.pending.delete(id); };
  count() { return this.pending.size; }
  findZero() { return [...this.pending.entries()].find(([, t]) => t.ms === 0); }
  // Run 0ms yield timers until none remain. NEVER awaits the callback —
  // callbacks are resolve() handles whose continuations schedule the next
  // yield only after several microtask turns, so drain between checks.
  async runZeroDelays() {
    for (let guard = 0; guard < 10000; guard += 1) {
      await microdrain();
      let zero = this.findZero();
      if (!zero) {
        await microdrain();
        zero = this.findZero();
        if (!zero) return;
      }
      this.pending.delete(zero[0]);
      zero[1].fn();
    }
    throw new Error('runZeroDelays guard tripped');
  }
  // Fire one pending timer of the given delay WITHOUT awaiting its callback
  // (a tick that internally awaits a 0ms yield would deadlock the test).
  // Microtasks are drained so immediate work inside the tick completes.
  async fire(ms) {
    const entry = [...this.pending.entries()].find(([, t]) => t.ms === ms);
    assert.ok(entry, `no pending ${ms}ms timer`);
    this.pending.delete(entry[0]);
    entry[1].fn();
    await microdrain();
  }
}

function row(sequence, overrides = {}) {
  return { sequence, correlation_id: 'warroom', virtual_path: '/rooms/warroom', payload: {}, ...overrides };
}

// A ledger fake: rows appear in `rows`; readPage pages strictly ascending.
function makeAdapters({ rows = [], visible = () => true, writeReturns = () => true } = {}) {
  const written = [];
  const closes = [];
  let drainResolvers = [];
  const adapters = {
    readHead: async () => (rows.length ? String(rows[rows.length - 1].sequence) : '0'),
    readPage: async (after, limit) => rows
      .filter((r) => BigInt(r.sequence) > BigInt(after))
      .slice(0, limit),
    isRowVisible: visible,
    reauthenticate: async () => ({ ok: true }),
    write: (frame) => { written.push(frame); return writeReturns(frame); },
    waitDrain: () => new Promise((resolve) => drainResolvers.push(resolve)),
    close: (kind) => closes.push(kind),
    random: () => 0.5,   // zero jitter
  };
  return {
    adapters, written, closes, rows,
    drain: () => { const r = drainResolvers; drainResolvers = []; r.forEach((fn) => fn()); },
  };
}

async function settle() {
  await microdrain();
}

// ── cursor selection (frozen precedence) ──────────────────────────────

test('selectCursor: Last-Event-ID wins, after_sequence fallback, default 0', () => {
  assert.equal(selectCursor({ lastEventId: '7', afterSequence: '3' }), '7');
  assert.equal(selectCursor({ afterSequence: '3' }), '3');
  assert.equal(selectCursor({}), '0');
  assert.equal(selectCursor({ lastEventId: '0' }), '0');
});

test('selectCursor: malformed SELECTED cursor throws (server maps to 400)', () => {
  for (const bad of [{ lastEventId: 'abc' }, { lastEventId: '' }, { lastEventId: '01' },
    { afterSequence: 'abc' }, { afterSequence: 42 }, { lastEventId: 'bad', afterSequence: '3' }]) {
    assert.throws(() => selectCursor(bad), LedgerValidationError, JSON.stringify(bad));
  }
});

// ── frame shapes (frozen payloads) ────────────────────────────────────

test('frames: exact hello/resync payload shapes; only room_event has an id', () => {
  assert.deepEqual(helloFrame('warroom', '9'), {
    event: 'hello', data: { room_id: 'warroom', snapshot_high_water: '9' },
  });
  assert.deepEqual(resyncFrame('5'), {
    event: 'resync_required', data: { after_sequence: '5' },
  });
  assert.equal('id' in helloFrame('warroom', '9'), false);
});

// ── protocol: H → hello → snapshot → single live poll ─────────────────

test('happy path: hello(H), snapshot (cursor,H], live picks up post-H appends', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({ rows: [row('1'), row('2'), row('3')] });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '1',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();

  assert.deepEqual(fake.written[0], helloFrame('warroom', '3'));
  const ids = fake.written.filter((f) => f.event === 'room_event').map((f) => f.id);
  assert.deepEqual(ids, ['2', '3'], 'snapshot emits (1,3] only');

  // appends land AFTER H while stream is live; the next poll gets them
  fake.rows.push(row('4'), row('6'));
  await timers.fire(1000);
  await settle();
  const allIds = fake.written.filter((f) => f.event === 'room_event').map((f) => f.id);
  assert.deepEqual(allIds, ['2', '3', '4', '6'], 'strictly increasing, sparse ids fine');
  stream.abort();
});

test('appends landing DURING snapshot are not missed (durable pull)', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({ rows: [row('1'), row('2')] });
  // readHead captures H=2, then a row lands before snapshot pages
  const baseReadHead = fake.adapters.readHead;
  fake.adapters.readHead = async () => { const h = await baseReadHead(); fake.rows.push(row('5')); return h; };
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  // hello advertises H=2 (captured before the append) …
  assert.equal(fake.written[0].data.snapshot_high_water, '2');
  // … and the FIRST live poll — which runs at the tail of start() — already
  // picks up the durable mid-snapshot append: nothing is missed.
  const ids = fake.written.filter((f) => f.event === 'room_event').map((f) => f.id);
  assert.deepEqual(ids, ['1', '2', '5'], 'snapshot emitted (0,2]; live owns 5');
  stream.abort();
});

test('filtered rows advance readCursor ONLY and are not gaps', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({
    rows: [row('1'), row('2', { clearance: 'TOP_SECRET' }), row('3')],
    visible: (r) => r.clearance !== 'TOP_SECRET',
  });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  const ids = fake.written.filter((f) => f.event === 'room_event').map((f) => f.id);
  assert.deepEqual(ids, ['1', '3'], 'no visible row omitted; invisible row not emitted');
  const view = stream.inspect();
  assert.equal(view.readCursor, '3');
  assert.equal(view.lastAcceptedId, '3', 'accepted id moves on emitted rows only');
  stream.abort();
});

// ── backpressure, overflow, bounded close ─────────────────────────────

test('overflow: queue holds bound AND next visible row exists -> resync with lastAcceptedId, close', async () => {
  const timers = new FakeTimers();
  // socket accepts the hello + first room_event then blocks forever
  let writes = 0;
  const total = QUEUE_BOUND + PAGE_LIMIT + 10;
  const fake = makeAdapters({
    rows: Array.from({ length: total }, (_, i) => row(String(i + 1))),
    writeReturns: () => { writes += 1; return writes <= 2; },
  });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  const startPromise = stream.start();
  await settle();
  await timers.runZeroDelays();   // snapshot page yields keep paging while pump is blocked
  await settle();
  await startPromise.catch(() => {});
  const resync = fake.written.find((f) => f.event === 'resync_required');
  assert.ok(resync, 'resync_required emitted (best-effort)');
  const lastAccepted = fake.written.filter((f) => f.event === 'room_event').map((f) => f.id).at(-1);
  assert.equal(resync.data.after_sequence, lastAccepted, 'advertises last ACCEPTED id, never past it');
  assert.deepEqual(fake.closes, ['resync']);
  const view = stream.inspect();
  assert.equal(view.closed, true);
  assert.deepEqual(view.timers, { poll: false, heartbeat: false }, 'no live timers after close');
});

test('abort during blocked drain releases the wait; zero timers; start() resolves', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({ rows: [row('1')], writeReturns: (f) => f.event !== 'room_event' });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  const startPromise = stream.start();
  await settle();
  stream.abort();
  await settle();
  await startPromise;   // must resolve, not hang, after abort released the drain wait
  const view = stream.inspect();
  assert.equal(view.closed, true);
  assert.deepEqual(view.timers, { poll: false, heartbeat: false });
  assert.equal(timers.count(), 0, 'no timer left behind');
});

// ── heartbeat: full reauth outcome mapping ────────────────────────────

test('heartbeat denial -> terminal revoked; I/O throw -> transport close, never revoked', async () => {
  for (const [behavior, expectedClose, expectRevokedFrame] of [
    [async () => ({ ok: false, reason: 'membership_revoked' }), 'revoked', true],
    [async () => { throw new Error('db down'); }, 'transport', false],
  ]) {
    const timers = new FakeTimers();
    const fake = makeAdapters({ rows: [row('1')] });
    fake.adapters.reauthenticate = behavior;
    const stream = createRoomEventStream({
      roomId: 'warroom', selectedCursor: '0',
      adapters: { ...fake.adapters, timers },
    });
    await stream.start();
    await settle();
    await timers.fire(15000);
    await settle();
    assert.deepEqual(fake.closes, [expectedClose]);
    assert.equal(fake.written.some((f) => f.event === 'revoked'), expectRevokedFrame,
      `revoked frame presence for ${expectedClose}`);
    assert.equal(timers.count(), 0);
  }
});

test('heartbeat fires during sustained catch-up (no starvation)', async () => {
  const timers = new FakeTimers();
  // enough rows that live catch-up needs multiple full pages
  const total = PAGE_LIMIT * 3;
  const fake = makeAdapters({ rows: Array.from({ length: total }, (_, i) => row(String(i + 1))) });
  let reauths = 0;
  fake.adapters.reauthenticate = async () => { reauths += 1; return { ok: true }; };
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: String(total),   // snapshot empty
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  // catch-up: make new rows appear in full pages, then interleave a heartbeat
  for (let i = total + 1; i <= total + PAGE_LIMIT; i += 1) fake.rows.push(row(String(i)));
  await timers.fire(1000);          // poll tick reads a FULL page -> schedules 0ms yield
  await settle();
  await timers.fire(15000);         // heartbeat must be fireable mid-catch-up
  await settle();
  assert.equal(reauths, 1, 'reauth ran during catch-up');
  await timers.runZeroDelays();     // let catch-up finish
  await settle();
  stream.abort();
  assert.equal(timers.count(), 0);
});

test('double start throws; abort is idempotent', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({ rows: [] });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await assert.rejects(stream.start(), /already started/);
  stream.abort();
  stream.abort();
  assert.deepEqual(fake.closes, ['done']);
});
