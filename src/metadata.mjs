import path from 'node:path';
import crypto from 'node:crypto';
import { promises as fs } from 'node:fs';
import { Pool } from 'pg';
import { embedObjectChunks } from './semantic.mjs';
import { extractGraphForObject } from './graph.mjs';

let pool = null;

export function hasDatabase() {
  return Boolean(process.env.DATABASE_URL);
}

export function getPool() {
  if (!process.env.DATABASE_URL) return null;
  if (!pool) pool = new Pool({ connectionString: process.env.DATABASE_URL });
  return pool;
}

export async function recordObjectWrite(state, apiPath, info, principal = {}) {
  const db = getPool();
  if (!db) return null;

  const backendKey = apiPath.replace(/^\//, '');
  const contentType = contentTypeFromPath(apiPath);
  const result = await db.query(
    `INSERT INTO longhouse.objects (
       virtual_path, mount_id, backend_key, kind, content_type, size_bytes, etag, metadata, updated_at
     ) VALUES ($1, 'managed-local', $2, $3, $4, $5, $6, $7, now())
     ON CONFLICT (virtual_path) DO UPDATE SET
       mount_id = EXCLUDED.mount_id,
       backend_key = EXCLUDED.backend_key,
       kind = EXCLUDED.kind,
       content_type = EXCLUDED.content_type,
       size_bytes = EXCLUDED.size_bytes,
       etag = EXCLUDED.etag,
       metadata = longhouse.objects.metadata || EXCLUDED.metadata,
       deleted_at = null,
       updated_at = now()
     RETURNING id`,
    [
      apiPath,
      backendKey,
      info.type === 'directory' ? 'directory' : 'file',
      contentType,
      info.size ?? null,
      info.etag ?? null,
      {
        source: 'ocean-bedrock-http',
        actorId: principal.id || null,
        actorName: principal.name || null,
        recordedAt: new Date().toISOString(),
      },
    ],
  );

  const objectId = result.rows[0]?.id;
  if (objectId && info.type !== 'directory') {
    await enqueueIngestJob('index_object', apiPath, objectId, {
      reason: 'file_write',
      contentType,
      etag: info.etag || null,
    });
  }
  return objectId;
}

export async function recordObjectDelete(apiPath, principal = {}) {
  const db = getPool();
  if (!db) return;
  await db.query(
    `UPDATE longhouse.objects
     SET deleted_at = now(), metadata = metadata || $2::jsonb, updated_at = now()
     WHERE virtual_path = $1`,
    [apiPath, JSON.stringify({ deletedBy: principal.name || principal.id || null, deletedAt: new Date().toISOString() })],
  );
}

export async function enqueueIngestJob(jobType, virtualPath, objectId = null, payload = {}) {
  const db = getPool();
  if (!db) return null;
  const result = await db.query(
    `INSERT INTO longhouse.ingest_jobs (job_type, virtual_path, object_id, payload)
     VALUES ($1, $2, $3, $4)
     RETURNING id`,
    [jobType, virtualPath, objectId, payload],
  );
  return result.rows[0]?.id || null;
}

function contentTypeFromPath(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  if (['.md', '.mdx'].includes(ext)) return 'text/markdown; charset=utf-8';
  if (['.txt', '.log', '.rst'].includes(ext)) return 'text/plain; charset=utf-8';
  if (['.json', '.jsonl'].includes(ext)) return 'application/json; charset=utf-8';
  if (['.yaml', '.yml'].includes(ext)) return 'application/yaml; charset=utf-8';
  if (ext === '.csv') return 'text/csv; charset=utf-8';
  if (['.js', '.mjs', '.cjs'].includes(ext)) return 'text/javascript; charset=utf-8';
  if (['.ts', '.tsx'].includes(ext)) return 'text/typescript; charset=utf-8';
  if (ext === '.html' || ext === '.htm') return 'text/html; charset=utf-8';
  if (ext === '.css') return 'text/css; charset=utf-8';
  if (ext === '.py') return 'text/x-python; charset=utf-8';
  if (ext === '.rs') return 'text/rust; charset=utf-8';
  if (ext === '.go') return 'text/go; charset=utf-8';
  if (ext === '.sql') return 'application/sql; charset=utf-8';
  return 'application/octet-stream';
}

function looksTextual(filePath, contentType) {
  if (contentType && (contentType.startsWith('text/') || contentType.includes('json') || contentType.includes('yaml') || contentType.includes('sql'))) return true;
  return [
    '.md', '.mdx', '.txt', '.rst', '.json', '.jsonl', '.yaml', '.yml', '.csv', '.tsv',
    '.html', '.htm', '.css', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx', '.py', '.rs',
    '.go', '.java', '.rb', '.php', '.sh', '.sql', '.toml', '.ini', '.log',
  ].includes(path.extname(filePath).toLowerCase());
}

export async function claimNextIngestJob(workerId = `worker-${process.pid}`) {
  const db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for ingest worker.');
  const result = await db.query(
    `WITH next AS (
       SELECT id
       FROM longhouse.ingest_jobs
       WHERE status = 'queued' AND run_after <= now()
       ORDER BY created_at ASC
       LIMIT 1
       FOR UPDATE SKIP LOCKED
     )
     UPDATE longhouse.ingest_jobs j
     SET status = 'running', locked_by = $1, locked_at = now(), attempts = attempts + 1, updated_at = now()
     FROM next
     WHERE j.id = next.id
     RETURNING j.*`,
    [workerId],
  );
  return result.rows[0] || null;
}

export async function completeIngestJob(jobId, status, error = null) {
  const db = getPool();
  await db.query(
    `UPDATE longhouse.ingest_jobs
     SET status = $2, error = $3, locked_by = null, locked_at = null, updated_at = now()
     WHERE id = $1`,
    [jobId, status, error],
  );
}

export async function listFailedIngestJobs({ limit = 50 } = {}) {
  const db = getPool();
  if (!db) return [];
  const result = await db.query(
    `SELECT id, job_type, status, virtual_path, attempts, max_attempts, error, updated_at, payload
     FROM longhouse.ingest_jobs
     WHERE status = 'failed'
     ORDER BY updated_at DESC
     LIMIT $1`,
    [limit],
  );
  return result.rows;
}

export async function requeueIngestJob(id, { resetAttempts = false } = {}) {
  const db = getPool();
  if (!db) throw new Error('DATABASE_URL required.');
  const result = await db.query(
    `UPDATE longhouse.ingest_jobs
     SET status = 'queued',
         locked_by = null,
         locked_at = null,
         run_after = now(),
         attempts = CASE WHEN $2 THEN 0 ELSE attempts END,
         error = null,
         updated_at = now()
     WHERE id = $1 AND status = 'failed'
     RETURNING *`,
    [id, resetAttempts],
  );
  return result.rows[0] || null;
}

export async function abandonIngestJob(id, reason = 'abandoned by operator') {
  const db = getPool();
  if (!db) throw new Error('DATABASE_URL required.');
  const result = await db.query(
    `UPDATE longhouse.ingest_jobs
     SET status = 'cancelled',
         error = $2,
         locked_by = null,
         locked_at = null,
         updated_at = now()
     WHERE id = $1 AND status = 'failed'
     RETURNING *`,
    [id, reason],
  );
  return result.rows[0] || null;
}

export async function processIngestJob(state, job) {
  if (job.job_type === 'embed_object') {
    if (!job.object_id) return { skipped: true, reason: 'missing object_id' };
    return embedObjectChunks(getPool(), job.object_id, job.payload || {});
  }
  if (job.job_type === 'extract_graph') {
    if (!job.object_id) return { skipped: true, reason: 'missing object_id' };
    return extractGraphForObject(getPool(), job.object_id);
  }
  if (job.job_type !== 'index_object') return { skipped: true, reason: `unsupported job_type ${job.job_type}` };
  if (!job.virtual_path || !job.object_id) return { skipped: true, reason: 'missing virtual_path/object_id' };

  const db = getPool();
  const diskPath = path.join(state.filesRoot, job.virtual_path.replace(/^\//, ''));
  const stat = await fs.stat(diskPath);
  const contentType = job.payload?.contentType || contentTypeFromPath(job.virtual_path);
  const maxBytes = Number(process.env.OCEAN_BEDROCK_INDEX_MAX_BYTES || 2 * 1024 * 1024);

  if (!stat.isFile()) return { skipped: true, reason: 'not a file' };
  if (stat.size > maxBytes) return { skipped: true, reason: `too large (${stat.size} > ${maxBytes})` };
  if (!looksTextual(job.virtual_path, contentType)) return { skipped: true, reason: `non-text content type ${contentType}` };

  const text = await fs.readFile(diskPath, 'utf8');
  const chunks = chunkText(text);
  await db.query('DELETE FROM longhouse.embedding_chunks WHERE object_id = $1', [job.object_id]);

  for (let i = 0; i < chunks.length; i += 1) {
    const chunk = chunks[i];
    await db.query(
      `INSERT INTO longhouse.embedding_chunks (
         object_id, virtual_path, chunk_index, chunk_text, chunk_sha256, token_count,
         embedding_provider, embedding_model, dimensions, metadata
       ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)`,
      [
        job.object_id,
        job.virtual_path,
        i,
        chunk,
        crypto.createHash('sha256').update(chunk).digest('hex'),
        Math.ceil(chunk.length / 4),
        'none',
        'text-chunk-v1',
        0,
        { indexedAt: new Date().toISOString(), contentType },
      ],
    );
  }

  await db.query(
    `INSERT INTO longhouse.graph_nodes (node_type, name, virtual_path, object_id, external_ref, properties)
     VALUES ('file', $1, $2, $3, $2, $4)
     ON CONFLICT (node_type, external_ref) DO UPDATE SET
       name = EXCLUDED.name,
       virtual_path = EXCLUDED.virtual_path,
       object_id = EXCLUDED.object_id,
       properties = longhouse.graph_nodes.properties || EXCLUDED.properties,
       updated_at = now()`,
    [path.basename(job.virtual_path), job.virtual_path, job.object_id, { contentType, chunks: chunks.length }],
  );

  await db.query(
    `UPDATE longhouse.objects
     SET metadata = metadata || $2::jsonb, updated_at = now()
     WHERE id = $1`,
    [job.object_id, JSON.stringify({ indexedAt: new Date().toISOString(), chunks: chunks.length, indexModel: 'text-chunk-v1' })],
  );

  await enqueueIngestJob('embed_object', job.virtual_path, job.object_id, { reason: 'index_object', force: true });
  await enqueueIngestJob('extract_graph', job.virtual_path, job.object_id, { reason: 'index_object' });

  return { indexed: true, chunks: chunks.length, bytes: stat.size, queued: ['embed_object', 'extract_graph'] };
}

function chunkText(text, maxChars = Number(process.env.OCEAN_BEDROCK_CHUNK_CHARS || 4000), overlap = Number(process.env.OCEAN_BEDROCK_CHUNK_OVERLAP || 400)) {
  const normalized = text.replace(/\r\n/g, '\n');
  if (normalized.length <= maxChars) return [normalized];
  const chunks = [];
  let start = 0;
  while (start < normalized.length) {
    let end = Math.min(normalized.length, start + maxChars);
    if (end < normalized.length) {
      const boundary = normalized.lastIndexOf('\n', end);
      if (boundary > start + Math.floor(maxChars * 0.5)) end = boundary;
    }
    chunks.push(normalized.slice(start, end));
    if (end >= normalized.length) break;
    start = Math.max(0, end - overlap);
  }
  return chunks;
}
