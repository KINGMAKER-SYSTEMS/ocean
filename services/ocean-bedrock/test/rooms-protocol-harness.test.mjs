import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  PROTOCOL_PROOFS,
  evaluateProtocolProofs,
  formatProtocolReport,
  passedCheckNames,
} from '../scripts/rooms-protocol-harness.mjs';

function outputForAllProofs() {
  return Object.values(PROTOCOL_PROOFS)
    .flat()
    .map((name) => `  ✓ ${name}`)
    .join('\n');
}

test('protocol harness recognizes only passing smoke checks', () => {
  const names = passedCheckNames('  ✓ passed proof\n  ✗ failed proof\nnoise');
  assert.deepEqual([...names], ['passed proof']);
});

test('protocol harness accepts the complete Gate-2 proof matrix', () => {
  const report = evaluateProtocolProofs(outputForAllProofs());
  assert.equal(report.ok, true);
  assert.ok(report.sections.every(({ missing }) => missing.length === 0));
  assert.match(formatProtocolReport(report), /PASS G2-AC5 producer-scoped idempotency and conflict/);
});

test('protocol harness fails closed and names a missing proof', () => {
  const missing = 'double redeem 403';
  const output = outputForAllProofs().replace(`  ✓ ${missing}\n`, '');
  const report = evaluateProtocolProofs(output);
  const inviteSection = report.sections.find(({ acceptanceCriterion }) => acceptanceCriterion.startsWith('G2-AC2'));

  assert.equal(report.ok, false);
  assert.deepEqual(inviteSection.missing, [missing]);
  assert.match(formatProtocolReport(report), /FAIL G2-AC2/);
  assert.match(formatProtocolReport(report), /missing: double redeem 403/);
});
