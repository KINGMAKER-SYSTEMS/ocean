// Gate-2 S1A focused HTTP tests — DB-free by construction: the server boots
// with file auth + JSONL ledger, so federationAvailable is false and every
// federated route must fail closed with an explicit 503 while ordinary local
// behavior stays green. Route-level positives that need a real database live
// in scripts/rooms-postgres-smoke.mjs (quadruple-gated).
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import os from 'node:os';
import { promises as fs } from 'node:fs';

import { createOceanBedrockServer } from '../src/server.mjs';

let server;
let state;
let baseUrl;
let adminToken;
let tmpRoot;

before(async () => {
  tmpRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'rooms-http-test-'));
  const created = createOceanBedrockServer({
    root: tmpRoot,
    authStore: 'file',
    ledgerStore: 'jsonl',
    instance: 'rooms-http-test',
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

function call(method, apiPath, body) {
  return fetch(`${baseUrl}${apiPath}`, {
    method,
    headers: {
      authorization: `Bearer ${adminToken}`,
      'content-type': 'application/json',
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

test('F3: every federated room route is an explicit 503 in file/JSONL mode', async () => {
  assert.equal(state.federationAvailable, false);
  const attempts = [
    ['POST', '/api/v1/rooms/warroom/register', {}],
    ['POST', '/api/v1/rooms/warroom/members/self', undefined],
    ['POST', '/api/v1/rooms/warroom/members/agents', { agents: [{ display_name: 'x', registration_key: 'k' }] }],
    ['GET', '/api/v1/rooms/warroom/members', undefined],
    ['DELETE', '/api/v1/rooms/warroom/members/9c2f4a52-0000-4000-8000-000000000001', undefined],
    ['POST', '/api/v1/invites', { room_id: 'warroom' }],
  ];
  for (const [method, apiPath, body] of attempts) {
    const response = await call(method, apiPath, body);
    assert.equal(response.status, 503, `${method} ${apiPath}`);
    const payload = await response.json();
    assert.equal(payload.ok, false);
    assert.match(payload.error, /Federation is unavailable/);
    assert.equal(JSON.stringify(payload).includes('postgres://'), false, 'must not leak URLs');
  }
});

test('ordinary local features still pass while federation is 503', async () => {
  const health = await fetch(`${baseUrl}/health`);
  assert.equal(health.status, 200);
  const info = await call('GET', '/api/v1/info');
  assert.equal(info.status, 200);
  const genericInvite = await call('POST', '/api/v1/invites', { name: 'legacy-flow', role: 'contributor' });
  assert.equal(genericInvite.status, 201, 'generic admin invite branch is byte-for-byte alive');
  const created = await genericInvite.json();
  assert.equal(typeof created.code, 'string');
  assert.equal(created.invite.role, 'contributor');
});

test('malformed percent-encoding in room segments is 400, not URIError 500', async () => {
  const response = await call('POST', '/api/v1/rooms/%E0%A4%A/register', {});
  assert.equal(response.status, 400);
  const payload = await response.json();
  assert.match(payload.error, /percent-encoding/);
});

test('unauthenticated room routes still require a token before anything else', async () => {
  const response = await fetch(`${baseUrl}/api/v1/rooms/warroom/members`);
  assert.equal(response.status, 401);
});

test('scalar/null/array JSON bodies are 400 on strict routes, never TypeError 500', async () => {
  for (const bad of [null, 42, ['agents']]) {
    const response = await call('POST', '/api/v1/invites', bad);
    assert.equal(response.status, 400, `invites body ${JSON.stringify(bad)}`);
    const payload = await response.json();
    assert.match(payload.error, /JSON object/);
  }
});
