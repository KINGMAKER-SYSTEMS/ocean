// Unit tests for src/extract.mjs — the binary-document (.pdf/.docx) text
// extraction that closes the old "binary office/pdf docs upload but are not
// indexable" KNOWN GAP. Pure extraction + gate logic only: no DB, no server.
// The PDF fixture is generated in-test (a minimal but structurally valid
// %PDF-1.4 with one text object and a correct xref table); the DOCX fixture
// is a tiny checked-in zip at test/fixtures/minimal.docx.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { extractableBinary, extractText } from '../src/extract.mjs';
import { chunkText, looksTextual } from '../src/metadata.mjs';

const FIXTURES = path.join(path.dirname(fileURLToPath(import.meta.url)), 'fixtures');

// Build a minimal single-page PDF containing `text` as one Tj text object.
// Offsets in the xref table are computed, not hardcoded, so the file is a
// structurally valid PDF (not just something pdf.js happens to recover).
function buildMinimalPdf(text) {
  const escaped = text.replace(/\\/g, '\\\\').replace(/\(/g, '\\(').replace(/\)/g, '\\)');
  const stream = `BT /F1 12 Tf 72 720 Td (${escaped}) Tj ET`;
  const objects = [
    '1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n',
    '2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n',
    '3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n',
    `4 0 obj\n<< /Length ${stream.length} >>\nstream\n${stream}\nendstream\nendobj\n`,
    '5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n',
  ];
  const header = '%PDF-1.4\n';
  let body = '';
  const offsets = [];
  for (const obj of objects) {
    offsets.push(header.length + body.length);
    body += obj;
  }
  const xrefStart = header.length + body.length;
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) xref += `${String(off).padStart(10, '0')} 00000 n \n`;
  const trailer = `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefStart}\n%%EOF\n`;
  return Buffer.from(header + body + xref + trailer, 'latin1');
}

async function tempFile(name, bytes) {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bedrock-extract-'));
  const filePath = path.join(dir, name);
  await fs.writeFile(filePath, bytes);
  return filePath;
}

test('extractableBinary: pdf/docx are extractable, everything else is null', () => {
  assert.equal(extractableBinary('report.pdf'), 'pdf');
  assert.equal(extractableBinary('/deep/path/Report.PDF'), 'pdf'); // case-insensitive
  assert.equal(extractableBinary('notes.docx'), 'docx');
  // Explicitly NOT extractable (remaining known gap, pinned in
  // information-layer.test.mjs) plus ordinary text/binary formats.
  for (const name of ['deck.pptx', 'sheet.xlsx', 'memo.rtf', 'a.md', 'a.png', 'noext']) {
    assert.equal(extractableBinary(name), null, `${name} must not claim extractability`);
  }
});

test('extractText: pulls text out of a minimal valid PDF', async () => {
  const pdfPath = await tempFile('hello.pdf', buildMinimalPdf('Hello Ocean Bedrock PDF'));
  const text = await extractText(pdfPath, 'pdf');
  assert.ok(text.includes('Hello Ocean Bedrock PDF'), `extracted: ${JSON.stringify(text)}`);
});

test('extractText: pulls text out of the checked-in minimal DOCX fixture', async () => {
  const text = await extractText(path.join(FIXTURES, 'minimal.docx'), 'docx');
  assert.ok(text.includes('Hello Ocean Bedrock DOCX fixture.'), `extracted: ${JSON.stringify(text)}`);
  assert.ok(text.includes('Second paragraph for chunk coverage.'), 'multi-paragraph text survives');
});

test('extractText: corrupt PDF throws (job failure, not silent skip)', async () => {
  const junkPath = await tempFile('corrupt.pdf', Buffer.from('this is not a pdf at all, just junk bytes'));
  await assert.rejects(() => extractText(junkPath, 'pdf'), /pdf extraction failed/);
});

test('extractText: corrupt DOCX throws (job failure, not silent skip)', async () => {
  const junkPath = await tempFile('corrupt.docx', Buffer.from('junk that is definitely not a zip archive'));
  await assert.rejects(() => extractText(junkPath, 'docx'), /docx extraction failed/);
});

test('extractText: a PDF with zero extractable text throws instead of indexing emptiness', async () => {
  const emptyPath = await tempFile('empty.pdf', buildMinimalPdf(''));
  await assert.rejects(() => extractText(emptyPath, 'pdf'), /produced no text/);
});

test('extractText: unknown kind throws', async () => {
  await assert.rejects(() => extractText('/nowhere/file.pptx', 'pptx'), /unsupported binary extraction kind/);
});

// Decision-level integration: this is exactly the gate processIngestJob runs
// (textual -> utf8 read; else extractableBinary -> extractText; else skip),
// followed by the same chunkText call the shared pipeline uses. Proves an
// uploaded .pdf now produces real chunks where it used to be silently skipped.
test('gate + pipeline: extracted PDF text flows through chunkText to real chunks', async () => {
  const virtualPath = '/context/inbox/report.pdf';
  assert.equal(looksTextual(virtualPath), false, 'pdf still is not utf8-readable text');
  const kind = extractableBinary(virtualPath);
  assert.equal(kind, 'pdf', 'gate routes .pdf to extraction instead of skip');

  const pdfPath = await tempFile('report.pdf', buildMinimalPdf('Quarterly ocean report: sound velocity is up.'));
  const chunks = chunkText(await extractText(pdfPath, kind));
  assert.ok(chunks.length >= 1, 'extraction yields at least one chunk');
  assert.ok(chunks[0].includes('sound velocity is up'), 'chunk carries the document text');
});
