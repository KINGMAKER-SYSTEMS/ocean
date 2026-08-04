// Text extraction for binary document formats, so .pdf/.docx uploads enter
// the SAME chunk -> embed -> graph pipeline as plain-text files instead of
// being silently dropped at the looksTextual() gate (the old KNOWN GAP in
// test/information-layer.test.mjs).
//
// Contract:
//   - extractableBinary(path) is the pure, extension-based gate (mirrors
//     looksTextual()'s style in metadata.mjs): 'pdf' | 'docx' | null.
//   - extractText(diskPath, kind) returns extracted plain text or THROWS.
//     A throw is the caller's job FAILING (visible in `npm run ocean:jobs`),
//     never a silent skip. That includes image-only/empty documents: zero
//     extractable text is an error, not an empty index entry.
//
// No DB imports here — this module stays pure enough to unit-test anywhere
// `node --test` runs. Parser deps (unpdf, mammoth) are lazy-imported so the
// server does not pay their load cost until a binary doc is actually indexed.
// .pptx/.xlsx/.rtf remain intentionally unextractable — see the pinned
// "KNOWN GAP" test in test/information-layer.test.mjs.
import path from 'node:path';
import { promises as fs } from 'node:fs';

export function extractableBinary(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  if (ext === '.pdf') return 'pdf';
  if (ext === '.docx') return 'docx';
  return null;
}

export async function extractText(diskPath, kind) {
  let text;
  if (kind === 'pdf') text = await extractPdfText(diskPath);
  else if (kind === 'docx') text = await extractDocxText(diskPath);
  else throw new Error(`unsupported binary extraction kind: ${kind}`);

  if (!text || !text.trim()) {
    throw new Error(`${kind} extraction produced no text for ${diskPath} (image-only or empty document?)`);
  }
  return text;
}

async function extractPdfText(diskPath) {
  const { getDocumentProxy, extractText: unpdfExtractText } = await import('unpdf');
  try {
    const bytes = new Uint8Array(await fs.readFile(diskPath));
    const pdf = await getDocumentProxy(bytes);
    const { text } = await unpdfExtractText(pdf, { mergePages: true });
    return text;
  } catch (error) {
    throw new Error(`pdf extraction failed for ${diskPath}: ${error.message}`, { cause: error });
  }
}

async function extractDocxText(diskPath) {
  const { default: mammoth } = await import('mammoth');
  try {
    const { value } = await mammoth.extractRawText({ path: diskPath });
    return value;
  } catch (error) {
    throw new Error(`docx extraction failed for ${diskPath}: ${error.message}`, { cause: error });
  }
}
