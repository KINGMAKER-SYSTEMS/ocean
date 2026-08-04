import crypto from 'node:crypto';

const DEFAULT_MODEL = '@cf/baai/bge-base-en-v1.5';
const DEFAULT_DIMENSIONS = 768;
const DEFAULT_INDEX = 'ocean-longhouse-context';

function envValue(primary, fallback = undefined) {
  return process.env[primary] || fallback;
}

export function semanticConfig() {
  const model = envValue('OCEAN_EMBEDDING_MODEL', DEFAULT_MODEL);
  return {
    accountId: envValue('CLOUDFLARE_ACCOUNT_ID'),
    apiToken: envValue('CLOUDFLARE_API_TOKEN'),
    model,
    dimensions: Number(envValue('OCEAN_EMBEDDING_DIMENSIONS', DEFAULT_DIMENSIONS)),
    vectorizeIndex: envValue('OCEAN_VECTORIZE_INDEX', DEFAULT_INDEX),
    pooling: envValue('OCEAN_EMBEDDING_POOLING', 'cls'),
    maxChars: Number(envValue('OCEAN_EMBEDDING_MAX_CHARS', 1800)),
    batchSize: Math.max(1, Math.min(Number(envValue('OCEAN_EMBEDDING_BATCH_SIZE', 16)), 50)),
    searchOverfetch: Math.max(1, Math.min(Number(envValue('OCEAN_SEMANTIC_SEARCH_OVERFETCH', 5)), 20)),
  };
}

export function semanticAvailable(config = semanticConfig()) {
  return Boolean(config.accountId && config.apiToken && config.vectorizeIndex && config.model);
}

export function semanticStatus(config = semanticConfig()) {
  return {
    enabled: semanticAvailable(config),
    provider: 'cloudflare',
    embeddingModel: config.model,
    dimensions: config.dimensions,
    vectorizeIndex: config.vectorizeIndex,
    pooling: config.pooling,
  };
}

function requireSemanticConfig(config = semanticConfig()) {
  if (!semanticAvailable(config)) {
    throw new Error('Semantic search requires CLOUDFLARE_ACCOUNT_ID, CLOUDFLARE_API_TOKEN, and OCEAN_VECTORIZE_INDEX.');
  }
  return config;
}

function cloudflareUrl(config, path) {
  return `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(config.accountId)}${path}`;
}

async function cloudflareFetch(config, path, options = {}) {
  const response = await fetch(cloudflareUrl(config, path), {
    ...options,
    headers: {
      Authorization: `Bearer ${config.apiToken}`,
      ...(options.headers || {}),
    },
  });
  const contentType = response.headers.get('content-type') || '';
  const body = contentType.includes('application/json') ? await response.json() : await response.text();
  if (!response.ok || body?.success === false) {
    const details = typeof body === 'string' ? body : JSON.stringify(body.errors || body.messages || body);
    throw new Error(`Cloudflare API ${options.method || 'GET'} ${path} failed: ${response.status} ${details}`);
  }
  return body;
}

export function prepareText(value, maxChars) {
  const text = String(value || '').replace(/\s+/g, ' ').trim();
  if (text.length <= maxChars) return text;
  return text.slice(0, maxChars);
}

export async function embedTexts(texts, config = semanticConfig()) {
  config = requireSemanticConfig(config);
  const input = (Array.isArray(texts) ? texts : [texts]).map((text) => prepareText(text, config.maxChars));
  if (!input.length) return [];
  const body = await cloudflareFetch(config, `/ai/run/${config.model}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ text: input, pooling: config.pooling }),
  });
  const embeddings = body?.result?.data || body?.result?.embeddings || body?.data;
  if (!Array.isArray(embeddings)) throw new Error(`Unexpected Workers AI embedding response: ${JSON.stringify(body).slice(0, 500)}`);
  return embeddings;
}

export function toNdjson(items) {
  return `${items.map((item) => JSON.stringify(item)).join('\n')}\n`;
}

export function compactMetadata(metadata = {}) {
  const result = {};
  for (const [key, value] of Object.entries(metadata)) {
    if (value === undefined || value === null) continue;
    if (['string', 'number', 'boolean'].includes(typeof value)) result[key] = value;
    else if (Array.isArray(value)) result[key] = value.filter((item) => ['string', 'number', 'boolean'].includes(typeof item));
    else result[key] = JSON.stringify(value).slice(0, 2000);
  }
  return result;
}

export function chunkVectorId(chunk) {
  const id = chunk.id || crypto.createHash('sha256').update(`${chunk.object_id}:${chunk.chunk_index}:${chunk.chunk_sha256}`).digest('hex');
  return `chunk:${id}`;
}

export async function upsertVectors(vectors, config = semanticConfig()) {
  config = requireSemanticConfig(config);
  if (!vectors.length) return { mutationId: null, count: 0 };
  const ndjson = toNdjson(vectors.map((vector) => ({
    id: vector.id,
    values: vector.values,
    metadata: compactMetadata(vector.metadata || {}),
  })));
  const body = await cloudflareFetch(config, `/vectorize/v2/indexes/${encodeURIComponent(config.vectorizeIndex)}/upsert?unparsable-behavior=error`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-ndjson' },
    body: ndjson,
  });
  return { mutationId: body?.result?.mutationId || null, count: vectors.length, response: body };
}

export async function queryVectors(vector, options = {}, config = semanticConfig()) {
  config = requireSemanticConfig(config);
  const body = await cloudflareFetch(config, `/vectorize/v2/indexes/${encodeURIComponent(config.vectorizeIndex)}/query`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      vector,
      topK: Math.max(1, Math.min(Number(options.topK || 10), 100)),
      returnMetadata: options.returnMetadata || 'all',
      returnValues: Boolean(options.returnValues),
      ...(options.filter ? { filter: options.filter } : {}),
    }),
  });
  return body?.result || { count: 0, matches: [] };
}

function pathMatchesPrefix(virtualPath, prefix) {
  if (!prefix || prefix === '/') return true;
  return virtualPath === prefix || virtualPath.startsWith(`${prefix.replace(/\/$/, '')}/`);
}

export async function embedObjectChunks(db, objectId, options = {}) {
  const config = semanticConfig();
  if (!semanticAvailable(config)) return { skipped: true, reason: 'semantic environment is not configured', status: semanticStatus(config) };

  const limit = Math.max(1, Math.min(Number(options.limit || 200), 1000));
  const force = Boolean(options.force);
  const chunks = await db.query(
    `SELECT ec.id, ec.object_id, ec.virtual_path, ec.chunk_index, ec.chunk_text, ec.chunk_sha256, ec.token_count, o.content_type, o.size_bytes
     FROM longhouse.embedding_chunks ec
     LEFT JOIN longhouse.objects o ON o.id = ec.object_id
     WHERE ec.object_id = $1
       AND ($2::boolean OR ec.vectorize_id IS NULL OR ec.embedding_provider = 'none')
     ORDER BY ec.chunk_index ASC
     LIMIT $3`,
    [objectId, force, limit],
  );

  if (!chunks.rows.length) return { embedded: false, chunks: 0, reason: 'no pending chunks' };

  const batches = [];
  for (let i = 0; i < chunks.rows.length; i += config.batchSize) batches.push(chunks.rows.slice(i, i + config.batchSize));

  let embedded = 0;
  const mutationIds = [];
  for (const batch of batches) {
    const embeddings = await embedTexts(batch.map((row) => row.chunk_text), config);
    const vectors = batch.map((row, index) => ({
      id: chunkVectorId(row),
      values: embeddings[index],
      metadata: {
        kind: 'longhouse_chunk',
        object_id: row.object_id,
        chunk_id: row.id,
        virtual_path: row.virtual_path,
        chunk_index: row.chunk_index,
        chunk_sha256: row.chunk_sha256,
        token_count: row.token_count,
        content_type: row.content_type,
      },
    }));
    const upserted = await upsertVectors(vectors, config);
    if (upserted.mutationId) mutationIds.push(upserted.mutationId);

    for (let i = 0; i < batch.length; i += 1) {
      const row = batch[i];
      await db.query(
        `UPDATE longhouse.embedding_chunks
         SET embedding_provider = 'cloudflare-workers-ai',
             embedding_model = $2,
             dimensions = $3,
             vectorize_index = $4,
             vectorize_id = $5,
             metadata = metadata || $6::jsonb,
             updated_at = now()
         WHERE id = $1`,
        [
          row.id,
          config.model,
          config.dimensions,
          config.vectorizeIndex,
          vectors[i].id,
          JSON.stringify({ embeddedAt: new Date().toISOString(), pooling: config.pooling, mutationId: upserted.mutationId || null }),
        ],
      );
      embedded += 1;
    }
  }

  await db.query(
    `UPDATE longhouse.objects
     SET metadata = metadata || $2::jsonb, updated_at = now()
     WHERE id = $1`,
    [objectId, JSON.stringify({ embeddedAt: new Date().toISOString(), embeddingModel: config.model, vectorizeIndex: config.vectorizeIndex, embeddedChunks: embedded })],
  );

  return { embedded: true, chunks: embedded, embeddingModel: config.model, vectorizeIndex: config.vectorizeIndex, mutationIds };
}

async function lexicalChunkSearch(db, query, options = {}) {
  const terms = String(query || '').trim().split(/\s+/).filter(Boolean).slice(0, 8);
  if (!terms.length) return [];
  const values = [];
  const clauses = terms.map((term) => {
    values.push(`%${term}%`);
    return `ec.chunk_text ILIKE $${values.length}`;
  });
  if (options.path && options.path !== '/') {
    values.push(options.path.replace(/\/$/, ''));
    clauses.push(`(ec.virtual_path = $${values.length} OR ec.virtual_path LIKE $${values.length + 1})`);
    values.push(`${options.path.replace(/\/$/, '')}/%`);
  }
  values.push(Math.max(1, Math.min(Number(options.limit || 10), 100)));
  const limitParam = values.length;
  const result = await db.query(
    `SELECT ec.id AS chunk_id, ec.object_id, ec.virtual_path, ec.chunk_index, ec.chunk_text,
            ec.chunk_sha256, ec.embedding_provider, ec.embedding_model, ec.vectorize_id,
            o.content_type, o.size_bytes
     FROM longhouse.embedding_chunks ec
     LEFT JOIN longhouse.objects o ON o.id = ec.object_id
     WHERE ${clauses.join(' AND ')}
     ORDER BY ec.updated_at DESC
     LIMIT $${limitParam}`,
    values,
  );
  return result.rows.map((row, index) => ({ ...row, score: 1 / (index + 1), search_mode: 'lexical_chunk_fallback' }));
}

export async function semanticSearch(db, query, options = {}) {
  const config = semanticConfig();
  const limit = Math.max(1, Math.min(Number(options.limit || 10), 50));
  const path = options.path || '/';

  if (!semanticAvailable(config) || options.mode === 'lexical') {
    const matches = await lexicalChunkSearch(db, query, { path, limit });
    return { semantic: false, status: semanticStatus(config), matches };
  }

  const [queryVector] = await embedTexts([query], config);
  const vectorResult = await queryVectors(queryVector, { topK: Math.min(100, limit * config.searchOverfetch), returnMetadata: 'all' }, config);
  const rawMatches = vectorResult.matches || [];
  const vectorIds = rawMatches.map((match) => match.id).filter(Boolean);
  if (!vectorIds.length) return { semantic: true, status: semanticStatus(config), matches: [], vectorMatches: 0 };

  const chunks = await db.query(
    `SELECT ec.id AS chunk_id, ec.object_id, ec.virtual_path, ec.chunk_index, ec.chunk_text,
            ec.chunk_sha256, ec.embedding_provider, ec.embedding_model, ec.vectorize_id,
            o.content_type, o.size_bytes, o.metadata AS object_metadata
     FROM longhouse.embedding_chunks ec
     LEFT JOIN longhouse.objects o ON o.id = ec.object_id
     WHERE ec.vectorize_id = ANY($1::text[])`,
    [vectorIds],
  );
  const chunksByVectorId = new Map(chunks.rows.map((row) => [row.vectorize_id, row]));
  const matches = [];
  for (const match of rawMatches) {
    const row = chunksByVectorId.get(match.id);
    if (!row) continue;
    if (!pathMatchesPrefix(row.virtual_path, path)) continue;
    matches.push({
      ...row,
      score: match.score,
      vector_id: match.id,
      metadata: match.metadata || {},
      search_mode: 'vectorize',
      preview: String(row.chunk_text || '').slice(0, Number(options.previewChars || 600)),
    });
    if (matches.length >= limit) break;
  }

  return { semantic: true, status: semanticStatus(config), matches, vectorMatches: rawMatches.length };
}
