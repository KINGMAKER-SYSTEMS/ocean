// Pure graph-extraction core of the Ocean Context pipeline: the functions that
// turn a document's text into knowledge-graph nodes/edges (headings, tags,
// links, path lineage). Deterministic, no DB — the "merging data into the graph"
// half of the information layer. Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  slugify,
  stableRef,
  extractHeadings,
  extractTags,
  extractLinks,
  pathPrefixes,
} from '../src/graph.mjs';

test('slugify: lowercases, collapses non-alphanumerics, trims dashes', () => {
  assert.equal(slugify('Hello, World!'), 'hello-world');
  assert.equal(slugify('  --Ocean  Context--  '), 'ocean-context');
  // Empty / junk input falls back to the stable sentinel, never ''.
  assert.equal(slugify(''), 'node');
  assert.equal(slugify('!!!'), 'node');
  assert.equal(slugify(null), 'node');
});

test('stableRef: deterministic sha256-keyed ref, prefix preserved', () => {
  const a = stableRef('topic', 'ocean');
  assert.match(a, /^topic:[0-9a-f]{64}$/);
  assert.equal(a, stableRef('topic', 'ocean'), 'same input -> same ref');
  assert.notEqual(a, stableRef('topic', 'sea'), 'different value -> different ref');
  assert.notEqual(a, stableRef('file', 'ocean'), 'different prefix -> different ref');
});

test('extractHeadings: ATX levels, trailing # stripped, limit honored', () => {
  const md = '# Title\n## Sub ##\nbody\n###   Spaced   \n#nothdr';
  const got = extractHeadings(md);
  assert.deepEqual(got, [
    { level: 1, title: 'Title' },
    { level: 2, title: 'Sub' }, // trailing closing #'s removed
    { level: 3, title: 'Spaced' },
  ]);
  // `#nothdr` (no space after #) is not a heading.
  assert.equal(extractHeadings('#nothdr').length, 0);
  // Limit caps the count.
  const many = Array.from({ length: 10 }, (_, i) => `# h${i}`).join('\n');
  assert.equal(extractHeadings(many, 3).length, 3);
});

test('extractTags: hashtags on word boundaries, deduped + lowercased', () => {
  const got = extractTags('intro #Ocean and #ocean again, #ml-ops end');
  assert.deepEqual(got, ['ocean', 'ml-ops'], 'deduped, lowercased');
  // A leading-letter requirement + min length: `#1` and `#ab` are too short/invalid.
  assert.deepEqual(extractTags('#1 #ab #abc'), ['abc']);
  // Mid-word hash (not preceded by start/space) is ignored.
  assert.deepEqual(extractTags('foo#bar'), []);
});

test('extractLinks: markdown link targets, optional title ignored', () => {
  const md = 'see [docs](./a.md) and [site](https://x.io "Title") and [bad]()';
  assert.deepEqual(extractLinks(md), ['./a.md', 'https://x.io']);
});

test('pathPrefixes: ancestor directories of a virtual path, not the leaf', () => {
  assert.deepEqual(pathPrefixes('/docs/context/readme.md'), ['/docs', '/docs/context']);
  // A top-level file has no ancestor prefixes.
  assert.deepEqual(pathPrefixes('/readme.md'), []);
  assert.deepEqual(pathPrefixes(''), []);
});
