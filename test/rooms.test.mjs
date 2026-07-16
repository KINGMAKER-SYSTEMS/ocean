// Gate-2 S1A pure tests: validators and safe projections from src/rooms.mjs.
// DB-free by design — the real-Postgres protocol gate lives in
// scripts/rooms-postgres-smoke.mjs (quadruple-gated, disposable DB only).
// Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  normalizeRoomId,
  roomScope,
  validateTtlMinutes,
  validateAgentDescriptor,
  validateRegistrationKey,
  descriptorsEqual,
  publicMemberProjection,
  RoomsError,
  INVITE_TTL_DEFAULT_MINUTES,
  INVITE_TTL_MAX_MINUTES,
} from '../src/rooms.mjs';

// ── room ids ──────────────────────────────────────────────────────────

test('normalizeRoomId accepts and lowercases sane ids', () => {
  assert.equal(normalizeRoomId('Ocean-Team.1'), 'ocean-team.1');
  assert.equal(normalizeRoomId('  a  '), 'a');
});

test('normalizeRoomId rejects malformed ids', () => {
  for (const bad of ['', ' ', '-lead', '.dot', 'has space', 'a/b', '../up', 'a'.repeat(129), 'room:id', null, undefined]) {
    assert.throws(() => normalizeRoomId(bad), RoomsError, `expected reject: ${String(bad)}`);
  }
});

test('roomScope is the exact frozen shape', () => {
  assert.equal(roomScope('warroom'), '/rooms/warroom');
});

// ── ttl ───────────────────────────────────────────────────────────────

test('ttl defaults ONLY when omitted', () => {
  assert.equal(validateTtlMinutes(undefined), INVITE_TTL_DEFAULT_MINUTES);
});

test('ttl rejects null, empty, and numeric strings', () => {
  for (const bad of [null, '', '60', '1440', 0, -1, 1.5, NaN, Infinity, INVITE_TTL_MAX_MINUTES + 1, true, {}]) {
    assert.throws(() => validateTtlMinutes(bad), RoomsError, `expected reject: ${String(bad)}`);
  }
});

test('ttl accepts integer bounds 1..10080', () => {
  assert.equal(validateTtlMinutes(1), 1);
  assert.equal(validateTtlMinutes(INVITE_TTL_MAX_MINUTES), INVITE_TTL_MAX_MINUTES);
});

// ── descriptors ───────────────────────────────────────────────────────

test('descriptor accepts exactly the safe subset', () => {
  const descriptor = validateAgentDescriptor({
    display_name: 'flux',
    description: 'implementation agent',
    model_alias: 'sonnet',
    skills_count: 3,
    subagent_names: ['pixel', 'brick'],
  });
  assert.deepEqual(descriptor, {
    display_name: 'flux',
    description: 'implementation agent',
    model_alias: 'sonnet',
    skills_count: 3,
    subagent_names: ['pixel', 'brick'],
  });
});

test('descriptor rejects local path/tool/credential fields outright', () => {
  for (const forbidden of [
    { display_name: 'x', path: '/Users/someone/dev' },
    { display_name: 'x', workspace_root: '/repo' },
    { display_name: 'x', tools: ['bash'] },
    { display_name: 'x', credentials: 'hunter2' },
    { display_name: 'x', api_key: 'sk-...' },
    { display_name: 'x', env: { HOME: '/' } },
  ]) {
    assert.throws(() => validateAgentDescriptor(forbidden), RoomsError);
  }
});

test('descriptor rejects unknown fields and bad types', () => {
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', color: 'red' }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: '' }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: { obj: 1 } }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', description: { o: 1 } }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', model_alias: 7 }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', skills_count: '3' }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', skills_count: 2 ** 32 }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', subagent_names: 'pixel' }), RoomsError);
  assert.throws(() => validateAgentDescriptor({ display_name: 'x', subagent_names: [1] }), RoomsError);
  assert.throws(() => validateAgentDescriptor(null), RoomsError);
  assert.throws(() => validateAgentDescriptor(['display_name']), RoomsError);
});

test('registration_key requires a stable 1-128 char string', () => {
  assert.equal(validateRegistrationKey(' agent-flux-1 '), 'agent-flux-1');
  for (const bad of [undefined, null, '', '   ', 'k'.repeat(129), {}, 123, true, ['k']]) {
    assert.throws(() => validateRegistrationKey(bad), RoomsError, `expected reject: ${String(bad)}`);
  }
});

test('descriptorsEqual: exact retry matches, changed content does not', () => {
  const a = validateAgentDescriptor({ display_name: 'flux', skills_count: 3 });
  const same = validateAgentDescriptor({ display_name: 'flux', skills_count: 3 });
  const changed = validateAgentDescriptor({ display_name: 'flux', skills_count: 4 });
  assert.equal(descriptorsEqual(a, same), true);
  assert.equal(descriptorsEqual(a, changed), false);
  assert.equal(descriptorsEqual(a, { ...same, description: 'now with text' }), false);
});

// ── safe projection ───────────────────────────────────────────────────

test('publicMemberProjection omits every internal field', () => {
  const row = {
    room_id: 'warroom',
    member_id: '9c2f4a52-0000-4000-8000-000000000001',
    actor_type: 'agent',
    role_in_room: 'member',
    display_name: 'flux',
    owner_member_id: '9c2f4a52-0000-4000-8000-000000000000',
    owner_principal_token_id: 'tok_secretref123',
    registration_key: 'agent-flux-1',
    public_agent_descriptor: { display_name: 'flux' },
    joined_at: '2026-07-16T18:00:00.000Z',
    removed_at: null,
    removed_by: 'tok_removerref',
  };
  const projection = publicMemberProjection(row);
  assert.deepEqual(projection, {
    member_id: row.member_id,
    actor_type: 'agent',
    role_in_room: 'member',
    display_name: 'flux',
    joined_at: '2026-07-16T18:00:00.000Z',
    owner_member_id: row.owner_member_id,
    public_agent_descriptor: { display_name: 'flux' },
  });
  const serialized = JSON.stringify(projection);
  for (const leak of ['owner_principal_token_id', 'registration_key', 'removed_by', 'tok_']) {
    assert.equal(serialized.includes(leak), false, `leaked: ${leak}`);
  }
});

test('publicMemberProjection rebuilds descriptors — forbidden JSONB keys never leak', () => {
  // Simulates a malformed/legacy/direct-SQL row whose stored descriptor
  // carries fields the validator would never accept.
  const projection = publicMemberProjection({
    member_id: 'id', actor_type: 'agent', role_in_room: 'member',
    display_name: 'rogue', owner_member_id: 'owner-id',
    owner_principal_token_id: 'tok_x', registration_key: 'k',
    joined_at: '2026-07-16T18:00:00.000Z',
    public_agent_descriptor: {
      display_name: 'rogue',
      description: 'fine',
      path: '/Users/someone/dev/secret-project',
      tools: ['bash'],
      token: 'tok_leak',
      api_key: 'sk-leak',
    },
  });
  assert.deepEqual(projection.public_agent_descriptor, {
    display_name: 'rogue',
    description: 'fine',
  });
  const serialized = JSON.stringify(projection);
  for (const leak of ['path', 'tools', 'sk-leak', 'tok_leak']) {
    assert.equal(serialized.includes(leak), false, `leaked: ${leak}`);
  }
});

test('publicMemberProjection fails closed on nested-value types', () => {
  // Allowed top-level keys hiding nested objects (direct-SQL/legacy rows)
  // must not serialize — the whole descriptor is omitted.
  const projection = publicMemberProjection({
    member_id: 'id', actor_type: 'agent', role_in_room: 'member',
    display_name: 'sneaky', owner_member_id: 'owner-id',
    owner_principal_token_id: 'tok_x', registration_key: 'k',
    joined_at: '2026-07-16T18:00:00.000Z',
    public_agent_descriptor: {
      display_name: 'sneaky',
      description: { token: 'tok_nested_secret' },
      subagent_names: [{ path: '/Users/someone/dev' }],
    },
  });
  assert.equal('public_agent_descriptor' in projection, false);
  const serialized = JSON.stringify(projection);
  for (const leak of ['tok_nested_secret', '/Users/someone/dev', 'token', 'path']) {
    assert.equal(serialized.includes(leak), false, `leaked: ${leak}`);
  }
});

test('publicMemberProjection: human row has no owner/descriptor keys', () => {
  const projection = publicMemberProjection({
    member_id: 'id', actor_type: 'user', role_in_room: 'owner',
    display_name: 'smaths', owner_member_id: null,
    owner_principal_token_id: 'tok_x', registration_key: null,
    public_agent_descriptor: null, joined_at: '2026-07-16T18:00:00.000Z',
  });
  assert.equal('owner_member_id' in projection, false);
  assert.equal('public_agent_descriptor' in projection, false);
});
