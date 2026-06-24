// Security regression: the local storage adapter must not let an INTERMEDIATE
// symlink escape the mount root (and thus a token's path scope). The leaf-only
// lstat check missed this; _diskPath now realpath-checks containment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import os from 'node:os';

import { LocalStorageAdapter } from '../src/mounts/adapters/local.mjs';

async function scaffold() {
  const base = await fs.mkdtemp(path.join(os.tmpdir(), 'ocbedrock-sec-'));
  const root = path.join(base, 'files');
  const outside = path.join(base, 'secret');
  await fs.mkdir(path.join(root, 'docs'), { recursive: true });
  await fs.mkdir(outside, { recursive: true });
  await fs.writeFile(path.join(outside, 'passwd.txt'), 'TOP SECRET');
  await fs.writeFile(path.join(root, 'docs', 'ok.txt'), 'fine');
  // The planted intermediate symlink: <root>/docs/escape -> <outside>.
  await fs.symlink(outside, path.join(root, 'docs', 'escape'));
  const adapter = new LocalStorageAdapter(
    { capabilities: { read: true, write: true, delete: true, move: true } },
    root,
  );
  return { base, adapter };
}

test('local adapter: legit in-root paths still resolve', async () => {
  const { base, adapter } = await scaffold();
  const r = await adapter.read('docs/ok.txt');
  assert.ok(r.diskPath.endsWith(path.join('docs', 'ok.txt')));
  await fs.rm(base, { recursive: true, force: true });
});

test('local adapter: read THROUGH an intermediate symlink is blocked', async () => {
  const { base, adapter } = await scaffold();
  await assert.rejects(
    () => adapter.read('docs/escape/passwd.txt'),
    /escapes the mount root/i,
    'an intermediate symlink must not expose files outside the root',
  );
  await fs.rm(base, { recursive: true, force: true });
});

test('local adapter: WRITE path under an intermediate symlink is blocked', async () => {
  const { base, adapter } = await scaffold();
  // The target doesn't exist yet (a write/mkdir) — the nearest existing ancestor
  // (the symlink) must still be resolved and rejected.
  await assert.rejects(
    () => adapter._diskPath('docs/escape/newfile.txt'),
    /escapes the mount root/i,
  );
  await fs.rm(base, { recursive: true, force: true });
});

test('local adapter: lexical ../ escape is still rejected', async () => {
  const { base, adapter } = await scaffold();
  await assert.rejects(() => adapter._diskPath('../../etc/passwd'), /escapes the mount root/i);
  await fs.rm(base, { recursive: true, force: true });
});
