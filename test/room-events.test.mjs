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
  assert.deepEqual(view.timers, { poll: false, heartbeat: false, yields: 0 }, 'no live timers after close');
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
  assert.deepEqual(view.timers, { poll: false, heartbeat: false, yields: 0 });
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

// ── corrective-child proofs (codex gate on 52ac00e) ───────────────────

test('blocked writer still reauths: heartbeat enqueue-then-reauth is drain-independent', async () => {
  const timers = new FakeTimers();
  // socket accepts hello then blocks forever — heartbeat frame will queue
  const fake = makeAdapters({ rows: [row('1')], writeReturns: (f) => f.event === 'hello' });
  let reauths = 0;
  fake.adapters.reauthenticate = async () => { reauths += 1; return { ok: true }; };
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  await timers.fire(15000);
  await settle();
  assert.equal(reauths, 1, 'full reauth ran despite the socket never draining');
  // Under backpressure the heartbeat frame correctly QUEUES behind the drain
  // (only terminal frames take the bounded jump) — submission, not delivery,
  // is what must be drain-independent.
  assert.ok(stream.inspect().controlQueueLength >= 1, 'heartbeat is queued in the serialized writer');
  assert.equal(fake.written.some((f) => f.event === 'heartbeat'), false, 'not written while blocked');
  stream.abort();
});

test('waitDrain rejection is a transport close, never treated as writable', async () => {
  const timers = new FakeTimers();
  const fake = makeAdapters({ rows: [row('1'), row('2')], writeReturns: (f) => f.event === 'hello' });
  fake.adapters.waitDrain = () => Promise.reject(new Error('socket destroyed'));
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  assert.deepEqual(fake.closes, ['transport']);
  assert.equal(fake.written.some((f) => f.event === 'revoked'), false, 'never falsely revoked');
  assert.equal(timers.count(), 0);
});

test('invalid H from readHead is a transport close before hello', async () => {
  for (const badHead of ['abc', '', '01', 42]) {
    const timers = new FakeTimers();
    const fake = makeAdapters({ rows: [] });
    fake.adapters.readHead = async () => badHead;
    const stream = createRoomEventStream({
      roomId: 'warroom', selectedCursor: '0',
      adapters: { ...fake.adapters, timers },
    });
    await stream.start();
    await settle();
    assert.deepEqual(fake.closes, ['transport'], `head ${JSON.stringify(badHead)}`);
    assert.equal(fake.written.length, 0, 'no hello emitted on malformed head');
    assert.equal(timers.count(), 0);
  }
});

test('one writer, one drain waiter: heartbeat during blocked pump neither loses frames nor doubles waiters', async () => {
  const timers = new FakeTimers();
  let outstandingDrains = 0;
  let maxOutstandingDrains = 0;
  let blocked = true;                 // closure flag — switchable after creation
  const drainReleases = [];
  const fake = makeAdapters({
    rows: [row('1'), row('2')],
    // hello always accepted; everything else blocks until unblocked
    writeReturns: (f) => f.event === 'hello' || !blocked,
  });
  fake.adapters.waitDrain = () => {
    if (!blocked) return Promise.resolve();
    return new Promise((resolve) => {
      outstandingDrains += 1;
      maxOutstandingDrains = Math.max(maxOutstandingDrains, outstandingDrains);
      drainReleases.push(() => { outstandingDrains -= 1; resolve(); });
    });
  };
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  await stream.start();
  await settle();
  await timers.fire(15000);   // heartbeat submits a control frame while writer is drain-blocked
  await settle();
  assert.equal(maxOutstandingDrains, 1, 'exactly one drain waiter at any moment');
  // unblock: everything still flows, in order, through the single writer
  blocked = false;
  drainReleases.splice(0).forEach((release) => release());
  await settle();
  const events = fake.written.map((f) => f.event);
  assert.equal(events[0], 'hello');
  assert.ok(events.includes('heartbeat') && events.filter((e) => e === 'room_event').length === 2,
    `all frames delivered once: ${events.join(',')}`);
  assert.equal(maxOutstandingDrains, 1, 'still exactly one waiter across the whole run');
  stream.abort();
  assert.equal(timers.count(), 0);
});

test('abort during a snapshot/catch-up yield cancels tracked yields and start resolves', async () => {
  const timers = new FakeTimers();
  const total = PAGE_LIMIT * 2;   // multi-page snapshot -> tracked yield between pages
  const fake = makeAdapters({ rows: Array.from({ length: total }, (_, i) => row(String(i + 1))) });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  const startPromise = stream.start();
  await settle();
  assert.equal(stream.inspect().timers.yields, 1, 'a tracked yield is pending mid-snapshot');
  stream.abort();
  await settle();
  await startPromise;   // the suspended loop was woken, observed closed, exited
  const view = stream.inspect();
  assert.equal(view.closed, true);
  assert.deepEqual(view.timers, { poll: false, heartbeat: false, yields: 0 });
  assert.equal(timers.count(), 0, 'no timer of any kind left behind');
});

test('terminal write throw closes with the DECIDED kind, not transport', async () => {
  // revoked: heartbeat denial queues terminal revoked; the revoked write throws
  {
    const timers = new FakeTimers();
    const fake = makeAdapters({ rows: [row('1')] });
    fake.adapters.reauthenticate = async () => ({ ok: false, reason: 'membership_revoked' });
    const baseWrite = fake.adapters.write;
    fake.adapters.write = (frame) => {
      if (frame.event === 'revoked') throw new Error('socket died mid-write');
      return baseWrite(frame);
    };
    const stream = createRoomEventStream({
      roomId: 'warroom', selectedCursor: '0',
      adapters: { ...fake.adapters, timers },
    });
    await stream.start();
    await settle();
    await timers.fire(15000);
    await settle();
    assert.deepEqual(fake.closes, ['revoked'], 'decided kind survives the write throw');
    assert.equal(timers.count(), 0);
  }
  // resync: overflow queues terminal resync; the resync write throws
  {
    const timers = new FakeTimers();
    let writes = 0;
    const total = QUEUE_BOUND + PAGE_LIMIT + 10;
    const fake = makeAdapters({
      rows: Array.from({ length: total }, (_, i) => row(String(i + 1))),
    });
    const baseWrite = fake.adapters.write;
    fake.adapters.write = (frame) => {
      if (frame.event === 'resync_required') throw new Error('socket died mid-write');
      writes += 1;
      baseWrite(frame);
      return writes <= 2;
    };
    const stream = createRoomEventStream({
      roomId: 'warroom', selectedCursor: '0',
      adapters: { ...fake.adapters, timers },
    });
    const startPromise = stream.start();
    await settle();
    await timers.runZeroDelays();
    await settle();
    await startPromise.catch(() => {});
    assert.deepEqual(fake.closes, ['resync'], 'decided kind survives the write throw');
    assert.equal(timers.count(), 0);
  }
});

test('disconnect with a deferred in-flight read: resolve after abort writes nothing', async () => {
  const timers = new FakeTimers();
  let resolveRead = null;
  const fake = makeAdapters({ rows: [] });
  fake.adapters.readPage = () => new Promise((resolve) => { resolveRead = resolve; });
  const stream = createRoomEventStream({
    roomId: 'warroom', selectedCursor: '0',
    adapters: { ...fake.adapters, timers },
  });
  const startPromise = stream.start();
  await settle();   // hello written; live pollTick's readPage is now deferred
  const writesBeforeAbort = fake.written.length;
  stream.abort();
  await settle();
  resolveRead([row('1'), row('2')]);   // the read resolves AFTER close
  await settle();
  await startPromise;
  assert.equal(fake.written.length, writesBeforeAbort, 'no write after close from a deferred read');
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
