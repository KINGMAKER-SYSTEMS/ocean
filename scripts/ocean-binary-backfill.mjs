#!/usr/bin/env node
// Backfill indexing for binary docs (.pdf/.docx) that uploaded before the
// extractor existed: they sit in longhouse.objects with NO embedding_chunks
// because the old looksTextual() gate silently skipped them. This finds those
// rows and re-enqueues index_object jobs; the ingest worker then runs the
// normal extract -> chunk -> embed -> graph pipeline (src/extract.mjs).
// Mirrors scripts/ocean-semantic-backfill.mjs's direct-DB style.
import { getPool, enqueueIngestJob, contentTypeFromPath } from '../src/metadata.mjs';

function usage() {
  console.log(`Usage:
  npm run ocean:backfill:binary
  npm run ocean:backfill:binary -- --dry-run --limit 100

Finds .pdf/.docx objects with no embedding chunks and enqueues index_object
jobs for the ingest worker (npm run ocean:worker) to extract and index.

Options:
  --limit <n>   Max objects to enqueue. Default: 50.
  --dry-run     List what would be enqueued without writing jobs.
`);
}

function parseArgs(argv) {
  const args = { limit: 50, dryRun: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--limit') args.limit = Math.max(1, Number(argv[++i]));
    else if (arg === '--dry-run') args.dryRun = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  usage();
  process.exit(0);
}

const db = getPool();
if (!db) throw new Error('DATABASE_URL is required.');

// pdf/docx files that were never chunked, without an index_object job already
// waiting — so repeated backfill runs do not double-enqueue.
const objects = await db.query(
  `SELECT o.id, o.virtual_path, o.size_bytes
   FROM longhouse.objects o
   WHERE o.deleted_at IS NULL
     AND o.kind = 'file'
     AND (lower(o.virtual_path) LIKE '%.pdf' OR lower(o.virtual_path) LIKE '%.docx')
     AND NOT EXISTS (
       SELECT 1 FROM longhouse.embedding_chunks ec WHERE ec.object_id = o.id
     )
     AND NOT EXISTS (
       SELECT 1 FROM longhouse.ingest_jobs j
       WHERE j.object_id = o.id AND j.job_type = 'index_object' AND j.status IN ('queued', 'running')
     )
   ORDER BY o.updated_at DESC
   LIMIT $1`,
  [args.limit],
);

const results = [];
for (const object of objects.rows) {
  const item = { objectId: object.id, virtualPath: object.virtual_path, sizeBytes: object.size_bytes };
  try {
    if (args.dryRun) {
      item.wouldEnqueue = true;
    } else {
      item.jobId = await enqueueIngestJob('index_object', object.virtual_path, object.id, {
        reason: 'binary_backfill',
        contentType: contentTypeFromPath(object.virtual_path),
      });
    }
    item.ok = true;
  } catch (error) {
    item.ok = false;
    item.error = error.message;
  }
  results.push(item);
  console.log(JSON.stringify(item));
}

console.log(JSON.stringify({ ok: results.every((item) => item.ok), dryRun: args.dryRun, matched: results.length }, null, 2));
await db.end();
