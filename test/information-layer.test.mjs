// First test suite for ocean-bedrock. Covers the PURE core of the Ocean Context
// ingest/merge pipeline — the functions that decide how a file becomes
// knowledge-layer chunks. No DB, no Cloudflare: these run anywhere `node --test`
// does. Run with `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  chunkText,
  contentTypeFromPath,
  looksTextual,
} from '../src/metadata.mjs';
import { extractableBinary } from '../src/extract.mjs';

test('chunkText: short text is a single chunk, CRLF normalized', () => {
  assert.deepEqual(chunkText('hello world', 4000, 400), ['hello world']);
  // \r\n collapses to \n even when it fits in one chunk.
  assert.deepEqual(chunkText('a\r\nb', 4000, 400), ['a\nb']);
  // Empty input is one empty chunk, never a crash.
  assert.deepEqual(chunkText('', 4000, 400), ['']);
});

test('chunkText: splits with overlap and covers the whole input', () => {
  const text = 'x'.repeat(1000);
  const chunks = chunkText(text, 400, 100);
  assert.ok(chunks.length > 1, 'long text splits into multiple chunks');
  // Every chunk respects the cap.
  for (const c of chunks) assert.ok(c.length <= 400, `chunk within cap: ${c.length}`);
  // Reassembling with the overlap removed reconstructs the original length:
  // each step advances by (end - overlap), so coverage is gapless.
  assert.equal(chunks.join('').length >= text.length, true, 'no gap in coverage');
  // Consecutive chunks share the overlap region (no prose newlines here, so the
  // boundary backtrack never fires and steps are exact).
  assert.equal(chunks[0].slice(-100), chunks[1].slice(0, 100), 'overlap carried forward');
});

test('chunkText: backtracks to a newline boundary past the halfway mark', () => {
  // maxChars 100: a newline at index 80 (> 50% of 100) should pull the cut back
  // to it so a chunk ends on a line boundary instead of mid-line.
  const text = 'a'.repeat(80) + '\n' + 'b'.repeat(200);
  const chunks = chunkText(text, 100, 10);
  assert.equal(chunks[0], 'a'.repeat(80), 'first chunk ends at the newline boundary');
});

test('chunkText: a newline before the halfway mark is NOT used (avoids tiny chunks)', () => {
  // Newline at index 20 (< 50% of 100) must be ignored, so the chunk fills to cap.
  const text = 'a'.repeat(20) + '\n' + 'b'.repeat(200);
  const chunks = chunkText(text, 100, 10);
  assert.equal(chunks[0].length, 100, 'early newline ignored, chunk fills to cap');
});

test('contentTypeFromPath: maps known text extensions, octet-stream otherwise', () => {
  assert.equal(contentTypeFromPath('a.md'), 'text/markdown; charset=utf-8');
  assert.equal(contentTypeFromPath('a.RS'), 'text/rust; charset=utf-8'); // case-insensitive
  assert.equal(contentTypeFromPath('a.json'), 'application/json; charset=utf-8');
  assert.equal(contentTypeFromPath('a.bin'), 'application/octet-stream');
  assert.equal(contentTypeFromPath('noext'), 'application/octet-stream');
});

test('looksTextual: text formats index, binaries do not', () => {
  assert.equal(looksTextual('a.md'), true);
  assert.equal(looksTextual('a.py'), true);
  assert.equal(looksTextual('a.bin', 'text/markdown; charset=utf-8'), true); // type wins
  assert.equal(looksTextual('a.png'), false);
});

// GAP CLOSED for .pdf/.docx (this is the flip of the old "KNOWN GAP: binary
// office/pdf docs upload but are not indexable" pin): processIngestJob's gate
// is now `looksTextual(path, contentType) || extractableBinary(path)`. PDF and
// DOCX stay non-textual (no utf8 read) but route through src/extract.mjs into
// the same chunk -> embed -> graph pipeline as text files. The full job needs
// a DB, so this asserts at the decision level — the exact gate expression the
// worker runs; extraction-to-chunks itself is covered end-to-end in
// test/binary-extraction.test.mjs.
test('binary doc gate: pdf/docx are extractable and pass processIngestJob gate', () => {
  for (const [ext, kind] of [['.pdf', 'pdf'], ['.docx', 'docx']]) {
    const virtualPath = `report${ext}`;
    assert.equal(looksTextual(virtualPath), false, `${ext} is still not utf8-readable text`);
    assert.equal(extractableBinary(virtualPath), kind, `${ext} routes to the ${kind} extractor`);
    const gateAdmits = looksTextual(virtualPath) || extractableBinary(virtualPath) !== null;
    assert.equal(gateAdmits, true, `${ext} is no longer silently dropped from the knowledge layer`);
  }
});

// KNOWN GAP (narrowed, not closed): bootstrap's DEFAULT_EXTENSIONS still
// advertises .pptx/.xlsx (and coworkers may upload .rtf), but there is no
// extractor for them — they land in longhouse.objects and are NEVER
// chunked/embedded/indexed into Ocean Context. This PINS the remaining gap so
// the decision stays explicit and a future extractor has a failing test to flip.
test('KNOWN GAP: pptx/xlsx/rtf upload but are not indexable', () => {
  for (const ext of ['.pptx', '.xlsx', '.rtf']) {
    assert.equal(looksTextual(`report${ext}`), false, `${ext} is not utf8-readable text`);
    assert.equal(
      extractableBinary(`report${ext}`),
      null,
      `${ext} has no extractor — still a silent drop from the knowledge layer`,
    );
  }
});
