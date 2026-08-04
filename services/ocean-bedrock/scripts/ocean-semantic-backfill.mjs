#!/usr/bin/env node
import { getPool } from '../src/metadata.mjs';
import { embedObjectChunks, semanticStatus } from '../src/semantic.mjs';
import { extractGraphForObject } from '../src/graph.mjs';

function usage() {
  console.log(`Usage:
  npm run ocean:semantic:backfill
  npm run ocean:semantic:backfill -- --limit 50 --force

Options:
  --limit <n>      Max objects to backfill. Default: 50.
  --force          Re-embed chunks even when vectorize_id exists.
  --embed-only     Skip graph extraction.
  --graph-only     Skip embeddings/vector upsert.
`);
}

function parseArgs(argv) {
  const args = { limit: 50, force: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--limit') args.limit = Math.max(1, Number(argv[++i]));
    else if (arg === '--force') args.force = true;
    else if (arg === '--embed-only') args.embedOnly = true;
    else if (arg === '--graph-only') args.graphOnly = true;
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

const values = [args.limit];
const where = args.force
  ? ''
  : `WHERE EXISTS (
       SELECT 1 FROM longhouse.embedding_chunks ec
       WHERE ec.object_id = o.id AND (ec.vectorize_id IS NULL OR ec.embedding_provider = 'none')
     )`;
const objects = await db.query(
  `SELECT o.id, o.virtual_path
   FROM longhouse.objects o
   ${where}
   ORDER BY o.updated_at DESC
   LIMIT $1`,
  values,
);

const results = [];
for (const object of objects.rows) {
  const item = { objectId: object.id, virtualPath: object.virtual_path };
  try {
    if (!args.graphOnly) item.embedding = await embedObjectChunks(db, object.id, { force: args.force });
    if (!args.embedOnly) item.graph = await extractGraphForObject(db, object.id);
    item.ok = true;
  } catch (error) {
    item.ok = false;
    item.error = error.message;
  }
  results.push(item);
  console.log(JSON.stringify(item));
}

console.log(JSON.stringify({ ok: results.every((item) => item.ok), semantic: semanticStatus(), processed: results.length }, null, 2));
await db.end();
