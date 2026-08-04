#!/usr/bin/env node
import assert from 'node:assert/strict';

import { getRunner, listRunnerAdapters } from '../src/sources/registry.mjs';
import { joinApiPath, pathInsidePrefix, SourceRunner } from '../src/sources/runner.mjs';
import { resolveSecret, validateSecretRef } from '../src/sources/secrets.mjs';

class FakeRunner extends SourceRunner {
  constructor() {
    super({ adapterId: 'fake', name: 'fake' });
  }

  async *iterate() {
    yield {
      source_record_id: 'fake:1',
      virtual_path: '/context/source-runner-check/one.md',
      content: '# one\n',
      content_type: 'text/markdown; charset=utf-8',
    };
    yield {
      source_record_id: 'fake:2',
      virtual_path: '/context/source-runner-check/two.md',
      content: '# two\n',
      content_type: 'text/markdown; charset=utf-8',
    };
  }
}

const adapters = listRunnerAdapters();
assert.deepEqual(adapters, ['github', 'telegram']);
assert.equal(getRunner('github').adapterId, 'github');
assert.equal(getRunner('telegram').adapterId, 'telegram');
assert.throws(() => getRunner('slack'), /no runner implemented/);

assert.equal(validateSecretRef('env:GITHUB_TOKEN'), 'env:GITHUB_TOKEN');
process.env.SOURCE_RUNNER_CHECK_SECRET = 'ok-value';
assert.equal(resolveSecret('env:SOURCE_RUNNER_CHECK_SECRET'), 'ok-value');
assert.throws(() => validateSecretRef('ghp_abcdefghijklmnopqrstuvwxyz1234567890'), /raw secret/);

assert.equal(joinApiPath('/context/github/repo', 'docs', 'README.md'), '/context/github/repo/docs/README.md');
assert.equal(pathInsidePrefix('/context/github/repo/docs/README.md', '/context/github/repo'), true);
assert.equal(pathInsidePrefix('/context/other/README.md', '/context/github/repo'), false);
assert.throws(() => joinApiPath('/context', '..', 'bad'), /Path traversal/);

const fake = new FakeRunner();
const packets = [];
for await (const packet of fake.iterate({})) packets.push(packet);
assert.equal(packets.length, 2);
assert.equal(packets[0].source_record_id, 'fake:1');

console.log('source-runner contract check passed');
