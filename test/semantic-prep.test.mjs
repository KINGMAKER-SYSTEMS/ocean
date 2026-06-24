// Pure embed→vectorize prep — the "migrate data into the semantic index" half of
// Ocean Context. These shape what gets sent to Cloudflare Workers AI / Vectorize;
// no network here, just the deterministic transforms. Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  prepareText,
  compactMetadata,
  toNdjson,
  chunkVectorId,
} from '../src/semantic.mjs';

test('prepareText: collapses whitespace and truncates to the char budget', () => {
  assert.equal(prepareText('  a\n\t b   c ', 100), 'a b c');
  assert.equal(prepareText('abcdef', 3), 'abc', 'truncates to maxChars');
  assert.equal(prepareText('', 10), '');
  assert.equal(prepareText(null, 10), '', 'null-safe');
});

test('compactMetadata: keeps primitives, filters arrays, stringifies+caps objects', () => {
  const out = compactMetadata({
    s: 'x',
    n: 5,
    b: true,
    nothing: null,
    undef: undefined,
    arr: ['ok', 2, true, { drop: 1 }, ['nested']],
    obj: { a: 1 },
  });
  // null/undefined dropped entirely.
  assert.deepEqual(Object.keys(out).sort(), ['arr', 'b', 'n', 'obj', 's']);
  // arrays keep only primitive items.
  assert.deepEqual(out.arr, ['ok', 2, true]);
  // nested objects become a (capped) JSON string.
  assert.equal(out.obj, '{"a":1}');
  assert.equal(out.s, 'x');
  assert.equal(out.n, 5);
  assert.equal(out.b, true);
});

test('compactMetadata: caps a large stringified object at 2000 chars', () => {
  const big = { blob: 'y'.repeat(5000) };
  const out = compactMetadata({ big });
  assert.equal(out.big.length, 2000, 'object value truncated to the Vectorize cap');
});

test('toNdjson: newline-delimited JSON with a trailing newline', () => {
  assert.equal(toNdjson([{ a: 1 }, { b: 2 }]), '{"a":1}\n{"b":2}\n');
  assert.equal(toNdjson([]), '\n', 'empty list is just the trailing newline');
});

test('chunkVectorId: prefers explicit id, else a deterministic content hash', () => {
  assert.equal(chunkVectorId({ id: 'abc' }), 'chunk:abc');
  const a = chunkVectorId({ object_id: 'o1', chunk_index: 0, chunk_sha256: 'deadbeef' });
  assert.match(a, /^chunk:[0-9a-f]{64}$/);
  // Same content -> same id (idempotent upserts); different index -> different id.
  assert.equal(a, chunkVectorId({ object_id: 'o1', chunk_index: 0, chunk_sha256: 'deadbeef' }));
  assert.notEqual(a, chunkVectorId({ object_id: 'o1', chunk_index: 1, chunk_sha256: 'deadbeef' }));
});
