#!/usr/bin/env node
import assert from 'node:assert/strict';
import { classifySensitivePath, DEFAULT_IGNORES, classifySkip } from '../src/ingest-filters.mjs';

const sensitive = [
  '.env',
  '.env.local',
  'id_rsa',
  'secret.pem',
  'app.key',
  'backup.p12',
  'service-account-prod.json',
  'terraform.tfstate',
  '.npmrc',
  '.aws/credentials',
  '.ssh/id_ed25519',
  'passwords.kdbx',
  'client-credentials.json',
];

for (const rel of sensitive) {
  const result = classifySensitivePath(rel);
  assert.equal(result.sensitive, true, `${rel} should be sensitive`);
}

const noise = ['node_modules', '.git', 'dist', 'build'];
for (const name of noise) {
  const result = classifySkip(name, name, DEFAULT_IGNORES);
  assert.equal(result.skipped, true, `${name} should be skipped`);
  assert.equal(result.reason, 'ignore', `${name} should be noise ignore`);
}

const normal = ['notes/project.md', 'src/index.mjs', 'docs/readme.txt'];
for (const rel of normal) {
  const result = classifySensitivePath(rel);
  assert.equal(result.sensitive, false, `${rel} should not be sensitive`);
}

console.log(JSON.stringify({ ok: true, sensitiveChecked: sensitive.length, noiseChecked: noise.length, normalChecked: normal.length }, null, 2));
