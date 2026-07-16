// Gate-2 S1B pure tests: producer-content comparator, room source_sequence
// format, after_sequence validation, and JSONL-store conflict + cursor
// semantics end-to-end (JSONL is DB-free). Postgres parity is proven in
// scripts/rooms-postgres-smoke.mjs. Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import os from 'node:os';
import { promises as fs } from 'node:fs';

import {
  createLedgerStore,
  producerContentEquals,
  validateRoomSourceSequence,
  parseAfterSequence,
  LedgerConflictError,
  LedgerValidationError,
} from '../src/ledger.mjs';

// A complete normalized event, as a real store row always is (rowToEvent
// yields every column). The v2 comparator covers ALL hash-bearing fields, so
// the retry input must carry the same server-injected actor identity — which
// it does in reality, because the server derives actor_* from the token.
const BASE_EVENT = {
  id: 'evt_x', schema_version: 1, sequence: 7, event_type: 'message',
  correlation_id: 'warroom', lab: 'longhouse', actor_type: 'agent',
  actor_id: 'tok_a', actor_name: 'alice', source_id: 'longhouse',
  source_sequence: null, source_ref: null, virtual_path: '/rooms/warroom',
  object_id: null, payload: { text: 'hello' }, visible_context: null,
  context_snapshot: null, clearance: 'UNCLASSIFIED', tags: [],
  timestamp: '2026-07-16T18:00:00.000Z', received_at: '2026-07-16T18:00:01.000Z',
  prev_hash: 'p', hash: 'h',
};
const BASE_INPUT = {
  event_type: 'message', correlation_id: 'warroom', virtual_path: '/rooms/warroom',
  payload: { text: 'hello' }, clearance: 'UNCLASSIFIED',
  actor_type: 'agent', actor_id: 'tok_a', actor_name: 'alice',
};

// ── comparator matrix ─────────────────────────────────────────────────

test('comparator: exact retry matches', () => {
  assert.equal(producerContentEquals(BASE_EVENT, BASE_INPUT), true);
});

test('comparator: ONLY chain/re-stamp fields are excluded (idempotent retry)', () => {
  for (const overrides of [
    { timestamp: '2026-07-16T19:59:59.000Z' },
    { received_at: '2026-07-16T19:59:59.000Z' },
    { id: 'evt_retry' },
  ]) {
    assert.equal(producerContentEquals(BASE_EVENT, { ...BASE_INPUT, ...overrides }), true,
      `expected match despite ${Object.keys(overrides).join(',')}`);
  }
});

test('comparator: EVERY hash-bearing delta conflicts (frozen v2 inverted pins)', () => {
  for (const overrides of [
    { payload: { text: 'DIFFERENT' } },
    { event_type: 'note' },
    { correlation_id: 'otherroom' },
    { virtual_path: '/rooms/otherroom' },
    { tags: ['x'] },
    { clearance: 'CONFIDENTIAL' },
    { lab: 'other-lab' },
    { source_ref: 'ref-1' },
    // v2: these were WRONGLY excluded in the rejected v1 comparator
    { actor_id: 'tok_other' },
    { actor_name: 'imposter' },
    { actor_type: 'service' },   // a VALID type — the delta itself must conflict
    { schema_version: 2 },
    { visible_context: { hidden: true } },
    { context_snapshot: { files: ['/x'] } },
    { object_id: '9c2f4a52-0000-4000-8000-000000000009' },
  ]) {
    // No catch-as-false escape: a normalizer throw here would be a test
    // failure, not a false green — the comparator itself must return false.
    assert.equal(
      producerContentEquals(BASE_EVENT, { ...BASE_INPUT, ...overrides }),
      false,
      `expected conflict on ${Object.keys(overrides).join(',')}`,
    );
  }
});

test('comparator: camelCase input aliases normalize before comparing', () => {
  assert.equal(producerContentEquals(BASE_EVENT, {
    eventType: 'message', correlationId: 'warroom', virtualPath: '/rooms/warroom',
    payload: { text: 'hello' }, clearance: 'UNCLASSIFIED',
    actorType: 'agent', actorId: 'tok_a', actorName: 'alice',
  }), true);
});

// ── room source_sequence format (B2) ──────────────────────────────────

test('room producer streams require canonical decimal u64 STRING', () => {
  validateRoomSourceSequence('room:warroom:member:m1:producer:p1', '1');
  validateRoomSourceSequence('room:warroom:member:m1:producer:p1', '18446744073709551615');
  for (const bad of ['01', '', ' 1', '1.0', '-1', 'abc', '18446744073709551616', null, undefined,
    1, 0, 42, 1.5, true, ['1']]) {
    // numbers are REJECTED, never coerced (frozen v2)
    assert.throws(
      () => validateRoomSourceSequence('room:warroom:member:m1:producer:p1', bad),
      LedgerValidationError,
      `expected reject: ${JSON.stringify(bad)}`,
    );
  }
});

test('non-room sources keep arbitrary text source_sequence (regression)', () => {
  validateRoomSourceSequence('github', 'commit-abc123');
  validateRoomSourceSequence('longhouse-api', null);
  validateRoomSourceSequence('telegram', '2026-07-16T00:00:00Z/42');
});

// ── after_sequence validation (B3) ────────────────────────────────────

test('after_sequence accepts canonical decimal STRINGS incl. "0"', () => {
  assert.equal(parseAfterSequence('0'), '0');
  assert.equal(parseAfterSequence('42'), '42');
  assert.equal(parseAfterSequence('18446744073709551615'), '18446744073709551615');
});

test('after_sequence rejects non-strings and non-canonical values', () => {
  for (const bad of ['', '01', '-1', '1.5', 'abc', '1e3', '18446744073709551616', null, undefined, {},
    1, 42, 0, true]) {
    // numbers are REJECTED, never coerced (frozen v2)
    assert.throws(() => parseAfterSequence(bad), LedgerValidationError, `expected reject: ${JSON.stringify(bad)}`);
  }
});

// ── JSONL store end-to-end (DB-free parity for B1 + B3) ───────────────

test('JSONL store: producer conflict + cursor semantics', async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'ledger-s1b-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const store = createLedgerStore({ store: 'jsonl', ledgerFile: path.join(dir, 'ledger.jsonl') });

  const tuple = { source_id: 'room:warroom:member:m1:producer:p1', source_sequence: '1' };
  const first = await store.append({ ...BASE_INPUT, ...tuple });
  assert.equal(first.sequence, 1);

  // exact retry → same event, nothing appended
  const retry = await store.append({ ...BASE_INPUT, ...tuple, timestamp: '2026-07-16T20:00:00.000Z' });
  assert.equal(retry.id, first.id);

  // different content, same tuple → typed conflict with details
  await assert.rejects(
    store.append({ ...BASE_INPUT, payload: { text: 'DIFFERENT' }, ...tuple }),
    (error) => error instanceof LedgerConflictError
      && error.details.existing_event_id === first.id
      && error.details.source_sequence === '1',
  );

  // non-canonical room sequence → validation error before any write
  await assert.rejects(
    store.append({ ...BASE_INPUT, source_id: tuple.source_id, source_sequence: '01' }),
    LedgerValidationError,
  );

  // build a stream, then walk it with the cursor
  for (let i = 2; i <= 5; i += 1) {
    await store.append({ ...BASE_INPUT, payload: { text: `m${i}` }, source_id: tuple.source_id, source_sequence: String(i) });
  }
  const pageOne = await store.read({ after_sequence: '0', limit: 3 });
  assert.deepEqual(pageOne.map((event) => event.sequence), [1, 2, 3], 'ascending from 0, bounded');
  const pageTwo = await store.read({ after_sequence: String(pageOne.at(-1).sequence), limit: 3 });
  assert.deepEqual(pageTwo.map((event) => event.sequence), [4, 5], 'strict > resumes without overlap');

  // absent cursor: existing tail-window behavior unchanged
  const tail = await store.read({ limit: 2 });
  assert.deepEqual(tail.map((event) => event.sequence), [4, 5]);
});

test('B3 pre-I/O gate: invalid cursor rejects BEFORE any file I/O', async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'ledger-s1b-preio-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const ledgerFile = path.join(dir, 'never-created.jsonl');
  const store = createLedgerStore({ store: 'jsonl', ledgerFile });
  await assert.rejects(store.read({ after_sequence: 42 }), LedgerValidationError);
  await assert.rejects(store.read({ after_sequence: 'abc' }), LedgerValidationError);
  // empty string is NOT "absent" — only null/undefined are; it must reject
  // through the strict parser like any other non-canonical value
  await assert.rejects(store.read({ after_sequence: '' }), LedgerValidationError);
  // the pre-I/O gate means the ledger file was never even created
  await assert.rejects(fs.access(ledgerFile), { code: 'ENOENT' });
});

test('B3 pre-I/O gate: Postgres read validates cursor before pool access', async () => {
  // An unreachable DATABASE_URL would hang/throw on pool connect — the
  // validation error firing proves the cursor gate precedes getPool.
  const store = createLedgerStore({ store: 'postgres', databaseUrl: 'postgres://invalid-host.invalid:5432/none_test' });
  await assert.rejects(store.read({ after_sequence: 42 }), LedgerValidationError);
  await assert.rejects(store.read({ after_sequence: '01' }), LedgerValidationError);
  await assert.rejects(store.read({ after_sequence: '' }), LedgerValidationError);
});
