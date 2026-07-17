// Gate-2 S2 Bedrock blockers (gate2-s2-bedrock-blockers-freeze-v4.md) —
// DB-free proofs. Route-level positives that need a real database live in
// scripts/rooms-postgres-smoke.mjs (quadruple-gated).
//
// B1: idempotent redeem is F3-only (503 in file/JSONL mode); legacy stays
//     byte-compatible.
// B2: strict room message payload schema + author/actor equality (pure
//     validator; membership freshness is proven in the PG harness).
// B3: sequences are canonical decimal u64 STRINGS in memory/row/wire with
//     hash-only bare-integer canonicalization — historical numeric JSONL
//     verifies green with zero migration; unsafe legacy numerics fail closed.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import os from 'node:os';
import { promises as fs } from 'node:fs';

import { createLedgerStore, LedgerValidationError } from '../src/ledger.mjs';
import { createOceanBedrockServer, validateRoomMessagePayload } from '../src/server.mjs';

let tmpRoot;
let server;
let baseUrl;
let adminToken;
let state;

before(async () => {
  tmpRoot = await fs.mkdtemp(path.join(os.tmpdir(), 's2-blockers-test-'));
  const created = createOceanBedrockServer({
    root: path.join(tmpRoot, 'server'),
    authStore: 'file',
    ledgerStore: 'jsonl',
    instance: 's2-blockers-test',
  });
  server = created.server;
  state = created.state;
  await created.ready;
  const minted = await state.auth.addToken({ name: 'test-admin', role: 'admin', scopes: ['/'] });
  adminToken = minted.token;
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  baseUrl = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  await new Promise((resolve) => server.close(resolve));
  await fs.rm(tmpRoot, { recursive: true, force: true });
});

function jsonlStore(name) {
  return createLedgerStore({ store: 'jsonl', ledgerFile: path.join(tmpRoot, `${name}.jsonl`) });
}

const MEMBER_A = '9c2f4a52-0000-4000-8000-00000000000a';
const MEMBER_B = '9c2f4a52-0000-4000-8000-00000000000b';

// ── B3: canonical string sequences ─────────────────────────────────────

test('B3: appended events carry canonical decimal STRING sequences', async () => {
  const store = jsonlStore('b3-strings');
  const first = await store.append({ event_type: 'demo', payload: {} });
  const second = await store.append({ event_type: 'demo', payload: {} });
  assert.equal(first.sequence, '1');
  assert.equal(second.sequence, '2');
  assert.equal(typeof (await store.read({ limit: 10 }))[0].sequence, 'string');
});

test('B3: next-sequence arithmetic is exact above 2^53 (BigInt, no rounding)', async () => {
  const store = jsonlStore('b3-bigseq');
  const seeded = await store.append({ event_type: 'demo', payload: {} });
  const raw = await fs.readFile(store.ledgerFile, 'utf8');
  const line = JSON.parse(raw.trim());
  // Force the chain head past 2^53: only sequence/hash feed the next append.
  line.sequence = '9007199254740993'; // 2^53 + 1 — unrepresentable in Number
  await fs.writeFile(store.ledgerFile, `${JSON.stringify(line)}\n`);
  const next = await store.append({ event_type: 'demo', payload: {} });
  assert.equal(next.sequence, '9007199254740994');
  assert.equal(seeded.sequence, '1');
});

test('B3: sequence space past canonical u64 fails closed', async () => {
  const store = jsonlStore('b3-u64max');
  await store.append({ event_type: 'demo', payload: {} });
  const raw = await fs.readFile(store.ledgerFile, 'utf8');
  const line = JSON.parse(raw.trim());
  line.sequence = '18446744073709551615'; // u64::MAX
  await fs.writeFile(store.ledgerFile, `${JSON.stringify(line)}\n`);
  await assert.rejects(
    store.append({ event_type: 'demo', payload: {} }),
    LedgerValidationError,
  );
});

test('B3: historical numeric-JSONL fixture verifies green with zero migration', async () => {
  const store = jsonlStore('b3-historical');
  await store.append({ event_type: 'demo', payload: { n: 1 } });
  await store.append({ event_type: 'demo', payload: { n: 2 }, tags: ['x'] });
  await store.append({
    event_type: 'decision.recorded',
    correlation_id: 'cor-hist-1',
    payload: { decision: 'historic' },
  });
  // Rewrite the file exactly as the PRE-change code serialized it: bare
  // numeric sequence literals. Hashes are untouched — if hash-only
  // canonicalization drifted by one byte, verify() would go red.
  const lines = (await fs.readFile(store.ledgerFile, 'utf8')).trim().split('\n').map((line) => {
    const event = JSON.parse(line);
    event.sequence = Number(event.sequence);
    return JSON.stringify(event);
  });
  await fs.writeFile(store.ledgerFile, `${lines.join('\n')}\n`);
  assert.match((await fs.readFile(store.ledgerFile, 'utf8')), /"sequence":3[,}]/);
  const verdict = await store.verify();
  assert.equal(verdict.ok, true, JSON.stringify(verdict.errors));
  assert.equal(verdict.events, 3);
  // The reader hands canonical strings to every consumer.
  const events = await store.read({ limit: 10 });
  assert.deepEqual(events.map((event) => event.sequence), ['1', '2', '3']);
});

test('B3: unsafe legacy numeric sequence is corruption, not a coercion', async () => {
  const store = jsonlStore('b3-unsafe');
  await store.append({ event_type: 'demo', payload: {} });
  const raw = await fs.readFile(store.ledgerFile, 'utf8');
  const line = JSON.parse(raw.trim());
  const doctored = JSON.stringify(line).replace('"sequence":"1"', '"sequence":9007199254740993');
  await fs.writeFile(store.ledgerFile, `${doctored}\n`);
  await assert.rejects(store.read({ limit: 10 }), LedgerValidationError);
});

test('B3: mixed legacy-numeric and new-string lines verify as one chain', async () => {
  const store = jsonlStore('b3-mixed');
  await store.append({ event_type: 'demo', payload: { n: 1 } });
  const legacy = (await fs.readFile(store.ledgerFile, 'utf8')).trim();
  const legacyLine = JSON.parse(legacy);
  legacyLine.sequence = Number(legacyLine.sequence);
  await fs.writeFile(store.ledgerFile, `${JSON.stringify(legacyLine)}\n`);
  await store.append({ event_type: 'demo', payload: { n: 2 } });
  const verdict = await store.verify();
  assert.equal(verdict.ok, true, JSON.stringify(verdict.errors));
  assert.equal(verdict.events, 2);
});

// ── B2: strict room message payload authentication (pure validator) ────

function goodPayload(overrides = {}) {
  return {
    client_event_id: 'client-evt-1',
    author_member_id: MEMBER_A,
    body: 'hello room',
    ...overrides,
  };
}

test('B2: exact payload with matching author/actor passes and lowercases ids', () => {
  const result = validateRoomMessagePayload(
    goodPayload({ author_member_id: MEMBER_A.toUpperCase(), mention_member_ids: [MEMBER_B] }),
    MEMBER_A,
  );
  assert.equal(result.authorMemberId, MEMBER_A);
  assert.deepEqual(result.mentionMemberIds, [MEMBER_B]);
});

test('B2: extra payload keys are a strict 400', () => {
  assert.throws(
    () => validateRoomMessagePayload(goodPayload({ sneaky: true }), MEMBER_A),
    /rejected: sneaky/,
  );
});

test('B2: author/actor mismatch is member_actor_mismatch', () => {
  assert.throws(
    () => validateRoomMessagePayload(goodPayload(), MEMBER_B),
    /member_actor_mismatch/,
  );
});

test('B2: missing/typed fields fail closed', () => {
  assert.throws(() => validateRoomMessagePayload(null, MEMBER_A), /payload object/);
  assert.throws(() => validateRoomMessagePayload(goodPayload({ client_event_id: '' }), MEMBER_A), /client_event_id/);
  assert.throws(() => validateRoomMessagePayload(goodPayload({ body: 42 }), MEMBER_A), /body string/);
  assert.throws(() => validateRoomMessagePayload(goodPayload({ author_member_id: 7 }), MEMBER_A), /UUID/);
});

test('B2: duplicate mentions are a 400; non-array mentions are a 400', () => {
  assert.throws(
    () => validateRoomMessagePayload(goodPayload({ mention_member_ids: [MEMBER_B, MEMBER_B] }), MEMBER_A),
    /unique/,
  );
  assert.throws(
    () => validateRoomMessagePayload(goodPayload({ mention_member_ids: MEMBER_B }), MEMBER_A),
    /array/,
  );
});

// ── B1: idempotent redeem route posture (file/JSONL mode) ──────────────

async function redeem(body) {
  return fetch(`${baseUrl}/api/v1/invites/redeem`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
}

test('B1: extended redeem form is F3-only — explicit 503 in file/JSONL mode', async () => {
  const response = await redeem({
    code: 'whatever',
    redemption_id: MEMBER_A,
    token: 'A'.repeat(43),
  });
  assert.equal(response.status, 503);
  const payload = await response.json();
  assert.match(payload.error, /Federation is unavailable/);
  // Presence of EITHER extended key routes to the F3 gate.
  const tokenOnly = await redeem({ code: 'whatever', token: 'A'.repeat(43) });
  assert.equal(tokenOnly.status, 503);
});

test('B1: legacy {code}-only redeem stays byte-compatible end-to-end', async () => {
  const created = await fetch(`${baseUrl}/api/v1/invites`, {
    method: 'POST',
    headers: { authorization: `Bearer ${adminToken}`, 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'legacy-redeem', role: 'contributor', scopes: ['/docs'] }),
  });
  assert.equal(created.status, 201);
  const invite = await created.json();
  const redeemed = await redeem({ code: invite.code });
  assert.equal(redeemed.status, 201);
  const payload = await redeemed.json();
  assert.equal(typeof payload.token, 'string');
  assert.equal(payload.record.role, 'contributor');
  assert.equal(payload.invite.consumedByTokenId, payload.record.id);
  assert.equal(payload.record.hash, undefined, 'no hash leakage');
  // One-shot: legacy reuse is still a 403.
  const reused = await redeem({ code: invite.code });
  assert.equal(reused.status, 403);
});

test('B1: scalar/null/array bodies are 400, never a TypeError 500', async () => {
  for (const bad of [null, 42, ['code']]) {
    const response = await redeem(bad);
    assert.equal(response.status, 400, JSON.stringify(bad));
  }
});
