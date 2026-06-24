// triageFindings: the daily-triage decision core of the information layer —
// maps a knowledge-layer snapshot (job/chunk/graph counts + ledger + semantic
// status) into prioritized findings. Pure; default thresholds keep these tests
// env-free. Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { triageFindings } from '../src/ocean-context.mjs';

const cleanSnapshot = {
  enabled: true,
  counts: {
    failedJobs: 0,
    staleFailedJobs: 0,
    queuedJobs: 0,
    pendingChunks: 0,
    failedSyncRuns: 0,
    graphNodes: 5,
    objects: 5,
  },
};
const semanticOk = { enabled: true };

test('triageFindings: a healthy box reports only the info "all clear"', () => {
  const out = triageFindings(cleanSnapshot, { ok: true }, semanticOk);
  assert.equal(out.length, 1);
  assert.equal(out[0].severity, 'info');
  assert.equal(out[0].area, 'ocean-context');
});

test('triageFindings: a broken ledger hash-chain is high severity', () => {
  const out = triageFindings(cleanSnapshot, { ok: false }, semanticOk);
  assert.ok(out.some((f) => f.severity === 'high' && f.area === 'ledger'));
});

test('triageFindings: ingest backlog/failures surface per area', () => {
  const snap = {
    enabled: true,
    counts: {
      failedJobs: 3, // > failedAlert default 0
      staleFailedJobs: 1,
      queuedJobs: 100, // > queueAlert default 50
      pendingChunks: 12,
      failedSyncRuns: 2,
      graphNodes: 0,
      objects: 4, // objects but no graph nodes -> graph finding
    },
  };
  const out = triageFindings(snap, { ok: true }, semanticOk);
  const areas = out.map((f) => f.area);
  assert.ok(areas.includes('ingest'), 'failed/queued jobs -> ingest');
  assert.ok(areas.includes('semantic'), 'pending chunks -> semantic');
  assert.ok(areas.includes('sources'), 'failed sync runs -> sources');
  assert.ok(areas.includes('graph'), 'objects-but-no-nodes -> graph');
  // The healthy "all clear" info finding must NOT appear when real issues exist.
  assert.ok(!out.some((f) => f.severity === 'info'));
});

test('triageFindings: unconfigured semantic env is flagged even on a clean box', () => {
  const out = triageFindings(cleanSnapshot, { ok: true }, { enabled: false });
  assert.ok(out.some((f) => f.area === 'semantic' && f.message.includes('not fully configured')));
});

test('triageFindings: a disabled snapshot skips count-based findings', () => {
  // snapshot.enabled = false -> none of the count checks run; only semantic/ledger.
  const out = triageFindings({ enabled: false, counts: {} }, { ok: true }, semanticOk);
  assert.equal(out.length, 1);
  assert.equal(out[0].severity, 'info', 'nothing to report when the snapshot is off');
});
