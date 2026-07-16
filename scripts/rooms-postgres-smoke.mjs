#!/usr/bin/env node
// Gate-2 S1A disposable-Postgres protocol gate (gate2-s1a-freeze.md v4).
//
// QUADRUPLE GATE — refuses to run unless ALL hold:
//   1. TEST_DATABASE_URL is set (DATABASE_URL is NEVER a fallback);
//   2. ROOMS_PG_SMOKE_ALLOW_RESET=1 (explicit disposable opt-in);
//   3. the database name ends in `_test`;
//   4. the target has NO preexisting `longhouse` schema (unshared proof).
// Cleanup is CREATE-ONLY: migrations are applied and gates proven; nothing
// preexisting is dropped/truncated/deleted. Discard the disposable
// container/database afterwards (printed, never automated).

import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { Pool } from 'pg';

import { createOceanBedrockServer } from '../src/server.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// ── quadruple gate (all four BEFORE the first CREATE) ─────────────────
const testDatabaseUrl = process.env.TEST_DATABASE_URL;
if (!testDatabaseUrl) {
  console.error('rooms-postgres-smoke: TEST_DATABASE_URL is not set (DATABASE_URL is never used). Refusing.');
  process.exit(1);
}
if (process.env.ROOMS_PG_SMOKE_ALLOW_RESET !== '1') {
  console.error('rooms-postgres-smoke: set ROOMS_PG_SMOKE_ALLOW_RESET=1 to confirm the target is a disposable test database. Refusing.');
  process.exit(1);
}
const dbName = (() => {
  try { return new URL(testDatabaseUrl).pathname.replace(/^\//, ''); } catch { return ''; }
})();
if (!dbName.endsWith('_test')) {
  console.error('rooms-postgres-smoke: the database name must end in "_test". Refusing.');
  process.exit(1);
}
// Interference guard: this process must see ONLY the test database.
delete process.env.DATABASE_URL;
delete process.env.OCEAN_AUTH_DATABASE_URL;
delete process.env.OCEAN_AUTH_STORE;
delete process.env.OCEAN_LEDGER_STORE;

const gatePool = new Pool({ connectionString: testDatabaseUrl });
const schemaExists = await gatePool.query(
  "SELECT EXISTS(SELECT 1 FROM information_schema.schemata WHERE schema_name = 'longhouse') AS present",
);
if (schemaExists.rows[0].present) {
  console.error('smoke aborted: target has preexisting longhouse schema');
  await gatePool.end();
  process.exit(1);
}

let passed = 0; let failed = 0;
function check(name, condition, detail = '') {
  if (condition) { passed += 1; console.log(`  ✓ ${name}`); }
  else { failed += 1; console.error(`  ✗ ${name}${detail ? ` — ${detail}` : ''}`); }
}

// SQLSTATE-verified negative: passes ONLY on the intended constraint class,
// never on syntax/connection errors.
async function expectSqlError(name, codes, sql, params = []) {
  try {
    await gatePool.query(sql, params);
    check(name, false, 'statement unexpectedly succeeded');
  } catch (error) {
    check(name, codes.includes(error.code), `SQLSTATE ${error.code} not in ${codes.join('/')}: ${error.message}`);
  }
}

let server = null;
let splitServer = null;
let state = null;
let splitState = null;
const tmpRoots = [];

try {
  // ── migrations 001..006, then 006 again (idempotency) ───────────────
  const MIGRATIONS = [
    'db/001_longhouse_core.sql', 'db/002_ocean_ledger.sql', 'db/003_source_adapters.sql',
    'db/004_source_capabilities_truth.sql', 'db/005_auth_registry.sql', 'db/006_federated_rooms.sql',
  ];
  for (const file of MIGRATIONS) {
    await gatePool.query(await fs.readFile(path.resolve(repoRoot, file), 'utf8'));
  }
  await gatePool.query(await fs.readFile(path.resolve(repoRoot, 'db/006_federated_rooms.sql'), 'utf8'));
  console.log('migrations 001..006 applied (006 twice — idempotent)');

  // ── boot server against the test database ───────────────────────────
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'rooms-pg-smoke-'));
  tmpRoots.push(root);
  const created = createOceanBedrockServer({
    root, instance: 'rooms-pg-smoke',
    databaseUrl: testDatabaseUrl, authStore: 'postgres', ledgerStore: 'postgres',
  });
  server = created.server; state = created.state;
  await created.ready;
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;

  async function api(token, method, apiPath, body) {
    const response = await fetch(`${base}${apiPath}`, {
      method,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    let payload = null;
    try { payload = await response.json(); } catch { /* non-JSON */ }
    return { status: response.status, body: payload };
  }

  check('federation available on single shared PG', state.federationAvailable === true);

  const admin1 = await state.auth.addToken({ name: 'owner-principal', role: 'admin', scopes: ['/'] });
  const admin2 = await state.auth.addToken({ name: 'rival-principal', role: 'admin', scopes: ['/'] });
  const ROOM = 'warroom';

  // ── R-1 registration ─────────────────────────────────────────────────
  console.log('R-1 registration:');
  const reg1 = await api(admin1.token, 'POST', `/api/v1/rooms/${ROOM}/register`, { title: 'War Room' });
  check('register 201 {room_id, owner}', reg1.status === 201 && reg1.body.room_id === ROOM && reg1.body.owner.role_in_room === 'owner');
  const ownerMemberId = reg1.body.owner?.member_id;
  const reg1Retry = await api(admin1.token, 'POST', `/api/v1/rooms/${ROOM}/register`, {});
  check('same-principal retry 200 same owner', reg1Retry.status === 200 && reg1Retry.body.owner.member_id === ownerMemberId);
  const reg2 = await api(admin2.token, 'POST', `/api/v1/rooms/${ROOM}/register`, {});
  check('different-principal claim 409', reg2.status === 409);

  // same-principal CONCURRENT register: exactly one 201 + one 200, same owner
  const [sameA, sameB] = await Promise.all([
    api(admin1.token, 'POST', '/api/v1/rooms/samereg/register', {}),
    api(admin1.token, 'POST', '/api/v1/rooms/samereg/register', {}),
  ]);
  check('concurrent same-principal register 201+200 same owner',
    [sameA.status, sameB.status].sort().join(',') === '200,201'
      && sameA.body.owner?.member_id === sameB.body.owner?.member_id,
    `${sameA.status}/${sameB.status}`);

  // different-principal CONCURRENT register: one winner, one 409 — winner derived, not assumed
  const [raceA, raceB] = await Promise.all([
    api(admin1.token, 'POST', '/api/v1/rooms/raceroom/register', {}),
    api(admin2.token, 'POST', '/api/v1/rooms/raceroom/register', {}),
  ]);
  check('concurrent different-principal register: one winner one 409', [raceA.status, raceB.status].sort().join(',') === '201,409');
  const raceOwnerToken = raceA.status === 201 ? admin1.token : admin2.token;

  check('invalid room id 400 in PG mode', (await api(admin1.token, 'POST', '/api/v1/rooms/Bad%20Room/register', {})).status === 400);
  check('register scalar body 400', (await api(admin1.token, 'POST', `/api/v1/rooms/${ROOM}/register`, 42)).status === 400);
  await gatePool.query("INSERT INTO longhouse.ledger_correlations (id, title, metadata) VALUES ('ticket-77', 'not a room', '{}'::jsonb)");
  check('unrelated correlation cannot be claimed', (await api(admin1.token, 'POST', '/api/v1/rooms/ticket-77/register', {})).status === 409);

  // ── R-2 invite derivation ────────────────────────────────────────────
  console.log('R-2 owner invites:');
  const invite1 = await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM, recipient_name: 'coworker' });
  check('owner invite 201 {code, invite}', invite1.status === 201 && typeof invite1.body.code === 'string' && Boolean(invite1.body.invite));
  check('invite payload has no hash/bearer material', !('hash' in (invite1.body.invite || {}))
    && !JSON.stringify(invite1.body).includes('sha256:'));
  check('derived contributor role', invite1.body.invite?.role === 'contributor');
  check('derived exact scope', JSON.stringify(invite1.body.invite?.scopes) === JSON.stringify([`/rooms/${ROOM}`]));
  const ttlMs = new Date(invite1.body.invite?.expiresAt).getTime() - Date.now();
  check('default TTL ~24h', ttlMs > 23.5 * 3600e3 && ttlMs < 24.5 * 3600e3, `got ${Math.round(ttlMs / 3600e3)}h`);
  check('supplied role rejected 400', (await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM, role: 'admin' })).status === 400);
  check('supplied scopes rejected 400', (await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM, scopes: ['/'] })).status === 400);
  check('ttl out of bounds 400', (await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM, ttl_minutes: 10081 })).status === 400);
  check('ttl string 400', (await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM, ttl_minutes: '60' })).status === 400);
  check('non-owner invite 403', (await api(admin2.token, 'POST', '/api/v1/invites', { room_id: ROOM })).status === 403);

  // ── R-3/R-4 redeem + self-join ───────────────────────────────────────
  console.log('R-3/R-4 redeem + self-join:');
  const redeem = await api('', 'POST', '/api/v1/invites/redeem', { code: invite1.body.code });
  check('redeem 201 contributor token', redeem.status === 201 && redeem.body.record.role === 'contributor');
  const joinToken = redeem.body.token;
  check('double redeem 403', (await api('', 'POST', '/api/v1/invites/redeem', { code: invite1.body.code })).status === 403);

  const join1 = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/self`);
  check('self-join 201 {member}', join1.status === 201 && join1.body.member.actor_type === 'user' && join1.body.member.role_in_room === 'member');
  const memberId = join1.body.member?.member_id;
  const dbHuman = await gatePool.query(
    'SELECT owner_principal_token_id FROM longhouse.room_members WHERE room_id=$1 AND member_id=$2', [ROOM, memberId],
  );
  check('DB human row bound to redeemed principal id', dbHuman.rows[0]?.owner_principal_token_id === redeem.body.record.id);
  const joinRetry = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/self`);
  check('self-join retry 200 same member', joinRetry.status === 200 && joinRetry.body.member.member_id === memberId);
  check('self-join nonempty body 400', (await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/self`, { display_name: 'spoof' })).status === 400);

  const directToken = await state.auth.addToken({ name: 'no-invite', role: 'contributor', scopes: [`/rooms/${ROOM}`] });
  check('no-invite provenance 403', (await api(directToken.token, 'POST', `/api/v1/rooms/${ROOM}/members/self`)).status === 403);
  // expanded scope: consumed generic invite, scope '/'
  const wideInvite = await api(admin1.token, 'POST', '/api/v1/invites', { name: 'wide', role: 'contributor', scopes: ['/'] });
  const wideRedeem = await api('', 'POST', '/api/v1/invites/redeem', { code: wideInvite.body.code });
  check('expanded-scope consumed token self-join 403', (await api(wideRedeem.body.token, 'POST', `/api/v1/rooms/${ROOM}/members/self`)).status === 403);
  check('elevated-role (admin) self-join 403', (await api(admin1.token, 'POST', `/api/v1/rooms/${ROOM}/members/self`)).status === 403);
  // cross-room: warroom-scoped consumed token against raceroom
  check('cross-room consumed token self-join 403', (await api(joinToken, 'POST', '/api/v1/rooms/raceroom/members/self')).status === 403);
  check('cross-room invite mint 403', (await api(joinToken, 'POST', '/api/v1/invites', { room_id: 'raceroom' })).status === 403);

  // concurrent same-principal self-join: exact statuses + same member
  const invite2 = await api(raceOwnerToken, 'POST', '/api/v1/invites', { room_id: 'raceroom' });
  const redeem2 = await api('', 'POST', '/api/v1/invites/redeem', { code: invite2.body.code });
  const [selfA, selfB] = await Promise.all([
    api(redeem2.body.token, 'POST', '/api/v1/rooms/raceroom/members/self'),
    api(redeem2.body.token, 'POST', '/api/v1/rooms/raceroom/members/self'),
  ]);
  check('concurrent same-principal self-join 201+200 same member',
    [selfA.status, selfB.status].sort().join(',') === '200,201'
      && selfA.body.member?.member_id === selfB.body.member?.member_id,
    `${selfA.status}/${selfB.status}`);

  // ── R-5 agent registration ───────────────────────────────────────────
  console.log('R-5 agents:');
  const agentsNew = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [
      { registration_key: 'flux-1', display_name: 'flux', model_alias: 'sonnet' },
      { registration_key: 'pixel-1', display_name: 'pixel' },
    ],
  });
  check('all-new batch 201, order preserved', agentsNew.status === 201
    && agentsNew.body.members.length === 2
    && agentsNew.body.members[0].display_name === 'flux'
    && agentsNew.body.members[1].display_name === 'pixel');
  const agentRetry = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [{ registration_key: 'flux-1', display_name: 'flux', model_alias: 'sonnet' }],
  });
  check('exact retry 200 same member', agentRetry.status === 200
    && agentRetry.body.members[0].member_id === agentsNew.body.members[0].member_id);
  const agentMixed = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [
      { registration_key: 'flux-1', display_name: 'flux', model_alias: 'sonnet' },
      { registration_key: 'brick-1', display_name: 'brick' },
    ],
  });
  check('mixed batch 200, order preserved (retried then new)', agentMixed.status === 200
    && agentMixed.body.members.length === 2
    && agentMixed.body.members[0].display_name === 'flux'
    && agentMixed.body.members[0].member_id === agentsNew.body.members[0].member_id
    && agentMixed.body.members[1].display_name === 'brick');
  check('changed content same key 409', (await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [{ registration_key: 'flux-1', display_name: 'flux', model_alias: 'opus' }],
  })).status === 409);
  check('duplicate keys in batch 400', (await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [{ registration_key: 'dup', display_name: 'a' }, { registration_key: 'dup', display_name: 'b' }],
  })).status === 400);
  check('descriptor path field 400', (await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [{ registration_key: 'evil-1', display_name: 'evil', path: '/Users/x' }],
  })).status === 400);
  check('no-membership (foreign) agent mint 403', (await api(directToken.token, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, {
    agents: [{ registration_key: 'foreign-1', display_name: 'foreign' }],
  })).status === 403);
  const [dupA, dupB] = await Promise.all([
    api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, { agents: [{ registration_key: 'race-key', display_name: 'racer' }] }),
    api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/agents`, { agents: [{ registration_key: 'race-key', display_name: 'racer' }] }),
  ]);
  check('concurrent same-key agents 201+200 same member',
    [dupA.status, dupB.status].sort().join(',') === '200,201'
      && dupA.body.members?.[0]?.member_id === dupB.body.members?.[0]?.member_id,
    `${dupA.status}/${dupB.status}`);

  // ── R-6 roster ───────────────────────────────────────────────────────
  console.log('R-6 roster:');
  const roster = await api(joinToken, 'GET', `/api/v1/rooms/${ROOM}/members`);
  const rosterJson = JSON.stringify(roster.body);
  check('roster shared to member', roster.status === 200 && roster.body.members.length >= 5);
  check('roster leaks no internals', !rosterJson.includes('tok_') && !rosterJson.includes('registration_key') && !rosterJson.includes('removed_by'));
  check('non-member roster 403', (await api(directToken.token, 'GET', `/api/v1/rooms/${ROOM}/members`)).status === 403);

  // ── R-8 ledger gating ────────────────────────────────────────────────
  console.log('R-8 ledger gating:');
  const writeBody = (overrides = {}) => ({
    event_type: 'message', correlation_id: ROOM, virtual_path: `/rooms/${ROOM}`,
    payload: { text: 'hello room' }, actor_member_id: memberId, ...overrides,
  });
  check('canonical write 201', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody())).status === 201);
  check('descendant path write 400', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody({ virtual_path: `/rooms/${ROOM}/notes.md` }))).status === 400);
  check('mismatched path write 400', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody({ virtual_path: '/docs' }))).status === 400);
  check('missing actor_member_id 403', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody({ actor_member_id: undefined }))).status === 403);
  check('unowned actor_member_id 403', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody({ actor_member_id: ownerMemberId }))).status === 403);
  check('malformed actor UUID 400', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody({ actor_member_id: 'not-a-uuid' }))).status === 400);
  check('member trace 200', (await api(joinToken, 'GET', `/api/v1/ledger/trace?correlation_id=${ROOM}`)).status === 200);
  check('non-member trace 403', (await api(directToken.token, 'GET', `/api/v1/ledger/trace?correlation_id=${ROOM}`)).status === 403);
  check('member descendant-path read 200', (await api(joinToken, 'GET', `/api/v1/ledger/events?path=/rooms/${ROOM}/notes.md`)).status === 200);
  check('read correlation+disagreeing path 400', (await api(joinToken, 'GET', `/api/v1/ledger/events?correlation_id=${ROOM}&path=/docs`)).status === 400);
  check('non-member path-only read 403', (await api(directToken.token, 'GET', `/api/v1/ledger/events?path=/rooms/${ROOM}/x`)).status === 403);
  const nonMemberGlobal = await api(directToken.token, 'GET', '/api/v1/ledger/events?limit=200');
  check('global read suppresses room rows for non-member', nonMemberGlobal.status === 200
    && !(nonMemberGlobal.body.events || []).some((event) => event.correlation_id === ROOM));
  const adminGlobal = await api(admin2.token, 'GET', '/api/v1/ledger/events?limit=200');
  check('admin WITHOUT membership also suppressed', adminGlobal.status === 200
    && !(adminGlobal.body.events || []).some((event) => event.correlation_id === ROOM));
  const memberGlobal = await api(joinToken, 'GET', '/api/v1/ledger/events?limit=200');
  check('global read shows room rows to member', (memberGlobal.body.events || []).some((event) => event.correlation_id === ROOM));
  // snapshots: exact valid/invalid gates
  const snapshotBody = (overrides = {}) => ({
    name: 'room-snap', correlation_id: ROOM, virtual_path: `/rooms/${ROOM}`,
    summary: 'canonical', clearance: 'UNCLASSIFIED', actor_member_id: memberId, ...overrides,
  });
  check('canonical member snapshot 201', (await api(joinToken, 'POST', '/api/v1/ledger/snapshots', snapshotBody())).status === 201);
  check('snapshot missing actor 403', (await api(joinToken, 'POST', '/api/v1/ledger/snapshots', snapshotBody({ actor_member_id: undefined }))).status === 403);
  check('snapshot foreign actor 403', (await api(joinToken, 'POST', '/api/v1/ledger/snapshots', snapshotBody({ actor_member_id: ownerMemberId }))).status === 403);
  check('snapshot non-member 403', (await api(directToken.token, 'POST', '/api/v1/ledger/snapshots', snapshotBody())).status === 403);
  check('snapshot correlation+disagreeing path 400 (member token)', (await api(joinToken, 'POST', '/api/v1/ledger/snapshots', snapshotBody({ virtual_path: '/docs' }))).status === 400);
  check('non-room ledger write unchanged', (await api(admin1.token, 'POST', '/api/v1/ledger/events', {
    event_type: 'note', correlation_id: 'ticket-77', payload: { text: 'regression' },
  })).status === 201);
  check('inactive-member ledger denial (no-membership write)', (await api(directToken.token, 'POST', '/api/v1/ledger/events', writeBody())).status === 403);

  // ── direct invalid SQL (SQLSTATE-verified) ───────────────────────────
  console.log('direct SQL constraint pins:');
  const principalTok = redeem.body.record.id;
  await expectSqlError('cross-room agent FK rejected', ['23503'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_member_id, owner_principal_token_id, registration_key)
     VALUES ('raceroom', 'agent', 'x', $1, $2, 'sqlpin-1')`, [memberId, principalTok]);
  await expectSqlError('cross-principal agent FK rejected', ['23503'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_member_id, owner_principal_token_id, registration_key)
     VALUES ($1, 'agent', 'x', $2, $3, 'sqlpin-2')`, [ROOM, memberId, admin2.record.id]);
  await expectSqlError('agent without owner rejected', ['23514'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_principal_token_id, registration_key)
     VALUES ($1, 'agent', 'x', $2, 'sqlpin-3')`, [ROOM, principalTok]);
  await expectSqlError('user with owner_member_id rejected', ['23514'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_member_id, owner_principal_token_id)
     VALUES ($1, 'user', 'x', $2, $3)`, [ROOM, memberId, principalTok]);
  await expectSqlError('descriptor on user row rejected', ['23514'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_principal_token_id, public_agent_descriptor)
     VALUES ($1, 'user', 'x', $2, '{}'::jsonb)`, [ROOM, principalTok]);
  await expectSqlError('second active owner rejected', ['23505'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, role_in_room, display_name, owner_principal_token_id)
     VALUES ($1, 'user', 'owner', 'x', $2)`, [ROOM, admin2.record.id]);
  await expectSqlError('second active human per principal rejected', ['23505'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_principal_token_id)
     VALUES ($1, 'user', 'twin', $2)`, [ROOM, principalTok]);
  const agentMemberId = agentsNew.body.members[0].member_id;
  await expectSqlError('agent-owned-by-agent rejected (trigger)', ['23514'],
    `INSERT INTO longhouse.room_members (room_id, actor_type, display_name, owner_member_id, owner_principal_token_id, registration_key)
     VALUES ($1, 'agent', 'x', $2, $3, 'sqlpin-4')`, [ROOM, agentMemberId, principalTok]);
  await expectSqlError('human removal orphaning agents rejected (trigger)', ['23514'],
    'UPDATE longhouse.room_members SET removed_at = now() WHERE room_id=$1 AND member_id=$2', [ROOM, memberId]);

  // ── registerAgents-vs-removal race (real Promise.all) ────────────────
  console.log('registerAgents-vs-removal serialization:');
  const agentraceReg = await api(admin1.token, 'POST', '/api/v1/rooms/agentrace/register', {});
  const arInvite = await api(admin1.token, 'POST', '/api/v1/invites', { room_id: 'agentrace' });
  const arRedeem = await api('', 'POST', '/api/v1/invites/redeem', { code: arInvite.body.code });
  const arJoin = await api(arRedeem.body.token, 'POST', '/api/v1/rooms/agentrace/members/self');
  const arMemberId = arJoin.body.member.member_id;
  check('agentrace setup', agentraceReg.status === 201 && arJoin.status === 201);
  const [arAgents, arRemove] = await Promise.all([
    api(arRedeem.body.token, 'POST', '/api/v1/rooms/agentrace/members/agents', {
      agents: [{ registration_key: 'ar-1', display_name: 'ar-agent' }],
    }),
    api(admin1.token, 'DELETE', `/api/v1/rooms/agentrace/members/${arMemberId}`),
  ]);
  // ONLY the two serialized outcomes are valid: agents registered before the
  // removal (201, then removed with the human) or removal won (403). A 409
  // here would mask a serialization bug.
  const serializedOutcome = (arAgents.status === 201 || arAgents.status === 403)
    && arRemove.status === 200;
  const arActive = await gatePool.query(
    "SELECT count(*)::int AS count FROM longhouse.room_members WHERE room_id='agentrace' AND removed_at IS NULL AND role_in_room='member'",
  );
  // Serialized outcomes only: either agents landed before removal (removal
  // took human+agents atomically) or removal won (register 403/409). Either
  // way nothing member-owned stays active.
  check('registerAgents-vs-removal serialized, nothing active left',
    serializedOutcome && arActive.rows[0].count === 0,
    `agents ${arAgents.status}, remove ${arRemove.status}, active ${arActive.rows[0].count}`);

  // ── R-7 removal + tombstone + races ──────────────────────────────────
  console.log('R-7 removal:');
  check('non-owner remove-other 403', (await api(joinToken, 'DELETE', `/api/v1/rooms/${ROOM}/members/${ownerMemberId}`)).status === 403);
  check('owner self-leave 409', (await api(admin1.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/${ownerMemberId}`)).status === 409);
  check('malformed member UUID 400', (await api(admin1.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/not-a-uuid`)).status === 400);

  // member SELF-removal, then repeat => membership_revoked (not 200)
  const selfInvite = await api(admin1.token, 'POST', '/api/v1/invites', { room_id: ROOM });
  const selfRedeem = await api('', 'POST', '/api/v1/invites/redeem', { code: selfInvite.body.code });
  const selfJoin2 = await api(selfRedeem.body.token, 'POST', `/api/v1/rooms/${ROOM}/members/self`);
  const selfMemberId = selfJoin2.body.member.member_id;
  check('member self-remove 200', (await api(selfRedeem.body.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/${selfMemberId}`)).status === 200);
  const selfRepeat = await api(selfRedeem.body.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/${selfMemberId}`);
  check('self-remove repeat 403 membership_revoked', selfRepeat.status === 403 && /membership_revoked/.test(JSON.stringify(selfRepeat.body)));

  const removal = await api(admin1.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/${memberId}`);
  check('owner removes human+agents atomically', removal.status === 200 && removal.body.removed.length >= 5,
    `removed ${JSON.stringify(removal.body.removed)}`);
  check('repeat delete by owner idempotent 200', (await api(admin1.token, 'DELETE', `/api/v1/rooms/${ROOM}/members/${memberId}`)).status === 200);
  check('removed member roster 403 membership_revoked', (await api(joinToken, 'GET', `/api/v1/rooms/${ROOM}/members`)).status === 403);
  const rejoin = await api(joinToken, 'POST', `/api/v1/rooms/${ROOM}/members/self`);
  check('removed principal same-token rejoin 403 membership_revoked', rejoin.status === 403 && /membership_revoked/.test(JSON.stringify(rejoin.body)));
  check('removed member ledger write denied', (await api(joinToken, 'POST', '/api/v1/ledger/events', writeBody())).status === 403);

  // remove-vs-selfJoin race on raceroom (no resurrection)
  const raceMemberId = selfA.body.member?.member_id || selfB.body.member?.member_id;
  const [raceRemove, raceJoin] = await Promise.all([
    api(raceOwnerToken, 'DELETE', `/api/v1/rooms/raceroom/members/${raceMemberId}`),
    api(redeem2.body.token, 'POST', '/api/v1/rooms/raceroom/members/self'),
  ]);
  check('remove-vs-selfJoin serialized responses', raceRemove.status === 200
    && (raceJoin.status === 200 || raceJoin.status === 403),
    `remove ${raceRemove.status}, join ${raceJoin.status}`);
  const resurrect = await gatePool.query(
    "SELECT count(*)::int AS count FROM longhouse.room_members WHERE room_id='raceroom' AND owner_principal_token_id=$1 AND actor_type='user' AND removed_at IS NULL",
    [redeem2.body.record.id],
  );
  const rejoinAfterRace = await api(redeem2.body.token, 'POST', '/api/v1/rooms/raceroom/members/self');
  check('remove-vs-selfJoin: no resurrection', resurrect.rows[0].count === 0 && rejoinAfterRace.status === 403,
    `active rows ${resurrect.rows[0].count}, rejoin ${rejoinAfterRace.status}`);

  const orphans = await gatePool.query(
    "SELECT count(*)::int AS count FROM longhouse.room_members agent WHERE agent.actor_type='agent' AND agent.removed_at IS NULL AND NOT EXISTS (SELECT 1 FROM longhouse.room_members h WHERE h.room_id=agent.room_id AND h.member_id=agent.owner_member_id AND h.removed_at IS NULL)",
  );
  check('no orphaned active agents anywhere', orphans.rows[0].count === 0);

  // ── split-Postgres F3: unequal auth URL string on the SAME test DB ────
  console.log('split-Postgres F3:');
  const splitRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'rooms-pg-smoke-split-'));
  tmpRoots.push(splitRoot);
  const splitAuthUrl = testDatabaseUrl + (testDatabaseUrl.includes('?') ? '&' : '?') + 'application_name=split_variant';
  const splitCreated = createOceanBedrockServer({
    root: splitRoot, instance: 'rooms-pg-smoke-split',
    databaseUrl: testDatabaseUrl, authDatabaseUrl: splitAuthUrl,
    authStore: 'postgres', ledgerStore: 'postgres',
  });
  splitServer = splitCreated.server; splitState = splitCreated.state;
  await splitCreated.ready;
  await new Promise((resolve) => splitServer.listen(0, '127.0.0.1', resolve));
  const splitBase = `http://127.0.0.1:${splitServer.address().port}`;
  check('split state disables federation', splitState.federationAvailable === false);
  const splitToken = await splitState.auth.addToken({ name: 'split-admin', role: 'admin', scopes: ['/'] });
  const splitAttempts = [
    ['POST', `/api/v1/rooms/${ROOM}/register`, '{}'],
    ['POST', `/api/v1/rooms/${ROOM}/members/self`, undefined],
    ['POST', `/api/v1/rooms/${ROOM}/members/agents`, JSON.stringify({ agents: [{ display_name: 'x', registration_key: 'k' }] })],
    ['GET', `/api/v1/rooms/${ROOM}/members`, undefined],
    ['DELETE', `/api/v1/rooms/${ROOM}/members/9c2f4a52-0000-4000-8000-000000000001`, undefined],
    ['POST', '/api/v1/invites', JSON.stringify({ room_id: ROOM })],
  ];
  for (const [method, apiPath, body] of splitAttempts) {
    const splitResponse = await fetch(`${splitBase}${apiPath}`, {
      method,
      headers: { authorization: `Bearer ${splitToken.token}`, 'content-type': 'application/json' },
      body,
    });
    const splitText = await splitResponse.text();
    check(`split-Postgres 503 no leak: ${method} ${apiPath.replace(base, '')}`,
      splitResponse.status === 503 && !splitText.includes('postgres://'),
      `status ${splitResponse.status}`);
  }
} finally {
  if (server) await new Promise((resolve) => server.close(resolve));
  if (splitServer) await new Promise((resolve) => splitServer.close(resolve));
  await state?.auth?.close?.().catch(() => {});
  await splitState?.auth?.close?.().catch(() => {});
  await gatePool.end().catch(() => {});
  for (const dir of tmpRoots) await fs.rm(dir, { recursive: true, force: true }).catch(() => {});
}

console.log(`\nrooms-postgres-smoke: ${passed} passed, ${failed} failed`);
console.log('cleanup is create-only by design — discard the disposable test database/container now.');
process.exit(failed ? 1 : 0);
