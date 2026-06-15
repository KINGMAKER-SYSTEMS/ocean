/**
 * Source adapter helper module — CRUD for the Ocean Bedrock source adapter tables.
 *
 * Pure data layer: connects to Postgres via getPool() (same pattern as src/metadata.mjs).
 * Every mutation emits a corresponding Ocean Ledger event for lineage tracing.
 *
 * Tables:
 *   longhouse.source_adapters     — adapter type definitions (local_folder, github, etc.)
 *   longhouse.source_instances    — configured source accounts/folders/repos/workspaces
 *   longhouse.source_streams      — selected streams/resources inside a source
 *   longhouse.source_sync_runs    — run history per source instance
 *   longhouse.source_records      — raw source object catalog / dedupe manifest
 */

import crypto from 'node:crypto';
import { getPool } from './metadata.mjs';
import { createLedgerStore } from './ledger.mjs';

// ---------------------------------------------------------------------------
// Ledger helpers
// ---------------------------------------------------------------------------

function ledgerStore() {
  return createLedgerStore({});
}

function nowIso() {
  return new Date().toISOString();
}

function correlationId(prefix = 'src') {
  return `${prefix}_${Date.now().toString(36)}_${crypto.randomBytes(4).toString('hex')}`;
}

/**
 * Emit a source adapter ledger event.
 *
 * @param {object} fields
 * @param {string} fields.eventType    — e.g. 'source.instance.registered'
 * @param {string} fields.sourceId     — e.g. 'local_folder:alice-macbook'
 * @param {string} [fields.virtualPath]
 * @param {string} [fields.sourceSequence]
 * @param {object} [fields.payload]
 * @param {string} [fields.clearance]
 * @param {string} [fields.correlationId]
 * @param {string} [fields.adapterId]
 * @returns {Promise<object>} the appended ledger event
 */
async function emitSourceEvent({
  eventType,
  sourceId,
  virtualPath = null,
  sourceSequence = null,
  payload = {},
  clearance = 'CONFIDENTIAL',
  correlationId: cId = null,
  adapterId = null,
}) {
  const store = ledgerStore();
  return store.append({
    event_type: eventType,
    correlation_id: cId || correlationId(),
    lab: 'ocean-context',
    actor_type: 'adapter',
    actor_id: adapterId || 'ocean-bedrock',
    actor_name: 'source-helper',
    source_id: sourceId,
    source_sequence: sourceSequence,
    virtual_path: virtualPath,
    payload,
    clearance,
    tags: ['source-ingest', 'ocean-bedrock', eventType],
  });
}

// ---------------------------------------------------------------------------
// ensureSourceAdapters
// ---------------------------------------------------------------------------

const DEFAULT_ADAPTERS = [
  {
    id: 'local_folder',
    display_name: 'Local Folder',
    description: 'Ingest files from a local directory tree.',
    config_schema: {
      type: 'object',
      properties: {
        device: { type: 'string', description: 'Device/hostname label' },
        local_path_label: { type: 'string', description: 'Human-friendly path label' },
        include_extensions: { type: 'array', items: { type: 'string' }, description: 'File extensions to include' },
        ignore: { type: 'array', items: { type: 'string' }, description: 'Patterns to ignore' },
        max_file_bytes: { type: 'integer', description: 'Maximum file size in bytes' },
      },
      required: ['device'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        local_path: { type: 'string', description: 'Absolute or relative path to folder' },
        remote_prefix: { type: 'string', description: 'Virtual path prefix for uploaded files' },
        recursive: { type: 'boolean', default: true },
      },
    },
    capabilities: {
      snapshot: true,
      incremental: true,
      webhook: false,
      delete_detection: true,
      binary_files: true,
      text_events: true,
    },
  },
  {
    id: 'github',
    display_name: 'GitHub Repository',
    description: 'Sync files from a GitHub repository.',
    config_schema: {
      type: 'object',
      properties: {
        owner: { type: 'string' },
        repo: { type: 'string' },
        branch: { type: 'string', default: 'main' },
        paths: { type: 'array', items: { type: 'string' } },
      },
      required: ['owner', 'repo'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        path: { type: 'string' },
        recursive: { type: 'boolean', default: true },
      },
    },
    capabilities: {
      snapshot: true,
      incremental: true,
      webhook: false,
      delete_detection: true,
      binary_files: true,
      text_events: true,
    },
  },
  {
    id: 'telegram',
    display_name: 'Telegram Chat',
    description: 'Ingest messages and media from Telegram chats.',
    config_schema: {
      type: 'object',
      properties: {
        chat_id: { type: 'string' },
        chat_type: { type: 'string', enum: ['group', 'channel', 'private'] },
      },
      required: ['chat_id'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        message_types: { type: 'array', items: { type: 'string' } },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: true,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: true,
    },
  },
  {
    id: 'slack',
    display_name: 'Slack Workspace',
    description: 'Ingest messages and files from Slack channels.',
    config_schema: {
      type: 'object',
      properties: {
        workspace: { type: 'string' },
        channel_id: { type: 'string' },
        capture_threads: { type: 'boolean', default: true },
        decision_keywords: { type: 'array', items: { type: 'string' } },
      },
      required: ['workspace', 'channel_id'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        channel: { type: 'string' },
        include_threads: { type: 'boolean', default: true },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: false,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: false,
    },
  },
  {
    id: 'notion',
    display_name: 'Notion Workspace',
    description: 'Ingest pages and databases from Notion.',
    config_schema: {
      type: 'object',
      properties: {
        workspace_id: { type: 'string' },
        page_ids: { type: 'array', items: { type: 'string' } },
        database_ids: { type: 'array', items: { type: 'string' } },
      },
      required: ['workspace_id'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        object_type: { type: 'string', enum: ['page', 'database'] },
        object_id: { type: 'string' },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: false,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: false,
    },
  },
  {
    id: 'linear',
    display_name: 'Linear Workspace',
    description: 'Ingest issues, projects, and cycles from Linear.',
    config_schema: {
      type: 'object',
      properties: {
        team_id: { type: 'string' },
        project_id: { type: 'string' },
        include_archived: { type: 'boolean', default: false },
      },
      required: ['team_id'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        resource_type: { type: 'string', enum: ['issues', 'projects', 'cycles'] },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: false,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: false,
    },
  },
  {
    id: 'google_drive',
    display_name: 'Google Drive',
    description: 'Ingest files and folders from Google Drive.',
    config_schema: {
      type: 'object',
      properties: {
        root_folder_id: { type: 'string' },
        include_shared_drives: { type: 'boolean', default: false },
        mime_types: { type: 'array', items: { type: 'string' } },
      },
      required: ['root_folder_id'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        folder_id: { type: 'string' },
        recursive: { type: 'boolean', default: true },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: false,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: false,
    },
  },
  {
    id: 'r2',
    display_name: 'Cloudflare R2 Bucket',
    description: 'Sync objects from Cloudflare R2 buckets.',
    config_schema: {
      type: 'object',
      properties: {
        bucket: { type: 'string' },
        prefix: { type: 'string', default: '' },
        endpoint: { type: 'string' },
      },
      required: ['bucket'],
    },
    stream_schema: {
      type: 'object',
      properties: {
        prefix: { type: 'string' },
        include_subdirectories: { type: 'boolean', default: true },
      },
    },
    capabilities: {
      snapshot: false,
      incremental: false,
      webhook: false,
      delete_detection: false,
      binary_files: false,
      text_events: false,
    },
  },
];

/**
 * Idempotently seed the source_adapters table with built-in adapter definitions.
 *
 * @param {object} [db] — optional pg Pool (auto-resolved via getPool() if omitted)
 * @returns {Promise<number>} number of adapters upserted
 */
export async function ensureSourceAdapters(db) {
  const pool = db || getPool();
  if (!pool) throw new Error('DATABASE_URL is required for source adapter operations.');

  let count = 0;
  for (const adapter of DEFAULT_ADAPTERS) {
    await pool.query(
      `INSERT INTO longhouse.source_adapters (id, display_name, description, config_schema, stream_schema, capabilities)
       VALUES ($1, $2, $3, $4, $5, $6)
       ON CONFLICT (id) DO UPDATE SET
         display_name = EXCLUDED.display_name,
         description = EXCLUDED.description,
         config_schema = EXCLUDED.config_schema,
         stream_schema = EXCLUDED.stream_schema,
         capabilities = EXCLUDED.capabilities,
         updated_at = now()`,
      [
        adapter.id,
        adapter.display_name,
        adapter.description,
        JSON.stringify(adapter.config_schema),
        JSON.stringify(adapter.stream_schema),
        JSON.stringify(adapter.capabilities),
      ],
    );
    count++;
  }

  await emitSourceEvent({
    eventType: 'source.adapters.seeded',
    sourceId: 'source_adapters',
    sourceSequence: `v1:${DEFAULT_ADAPTERS.length}`,
    payload: {
      adapter_ids: DEFAULT_ADAPTERS.map((adapter) => adapter.id),
      count,
    },
    adapterId: 'ocean-bedrock',
  });

  return count;
}

// ---------------------------------------------------------------------------
// upsertSourceInstance
// ---------------------------------------------------------------------------

/**
 * Create or update a source instance.
 *
 * @param {object} db — pg Pool
 * @param {object} instance
 * @param {string} instance.adapter_id — must reference an existing source_adapters.id
 * @param {string} instance.name
 * @param {string} [instance.owner_name]
 * @param {string} [instance.owner_token_id]
 * @param {string} instance.remote_prefix
 * @param {object} [instance.config={}]
 * @param {string} [instance.secret_ref]
 * @param {string} [instance.clearance='CONFIDENTIAL']
 * @param {string} [instance.correlationId] — ledger correlation ID
 * @returns {Promise<{id: string, adapter_id: string, name: string}>}
 */
export async function upsertSourceInstance(db, instance) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');

  const cId = instance.correlationId || correlationId();

  // Try to find existing instance by (adapter_id, name) as a natural key
  const existing = await db.query(
    'SELECT id FROM longhouse.source_instances WHERE adapter_id = $1 AND name = $2',
    [instance.adapter_id, instance.name],
  );

  let result;
  if (existing.rows.length) {
    // Update existing
    result = await db.query(
      `UPDATE longhouse.source_instances SET
         owner_name = COALESCE($2, longhouse.source_instances.owner_name),
         owner_token_id = COALESCE($3, longhouse.source_instances.owner_token_id),
         remote_prefix = $4,
         config = longhouse.source_instances.config || $5::jsonb,
         secret_ref = COALESCE($6, longhouse.source_instances.secret_ref),
         clearance = $7,
         updated_at = now()
       WHERE id = $1
       RETURNING id, adapter_id, name`,
      [
        existing.rows[0].id,
        instance.owner_name || null,
        instance.owner_token_id || null,
        instance.remote_prefix,
        JSON.stringify(instance.config || {}),
        instance.secret_ref || null,
        instance.clearance || 'CONFIDENTIAL',
      ],
    );
  } else {
    // Insert new
    result = await db.query(
      `INSERT INTO longhouse.source_instances
         (adapter_id, name, owner_name, owner_token_id, remote_prefix, config, secret_ref, clearance)
       VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8)
       RETURNING id, adapter_id, name`,
      [
        instance.adapter_id,
        instance.name,
        instance.owner_name || null,
        instance.owner_token_id || null,
        instance.remote_prefix,
        JSON.stringify(instance.config || {}),
        instance.secret_ref || null,
        instance.clearance || 'CONFIDENTIAL',
      ],
    );
  }

  const row = result.rows[0];

  await emitSourceEvent({
    eventType: 'source.instance.registered',
    sourceId: `${instance.adapter_id}:${instance.name}`,
    virtualPath: instance.remote_prefix,
    sourceSequence: null,
    payload: {
      adapter_id: instance.adapter_id,
      source_instance_id: row.id,
      name: instance.name,
      remote_prefix: instance.remote_prefix,
    },
    correlationId: cId,
    adapterId: instance.adapter_id,
  });

  return { id: row.id, adapter_id: row.adapter_id, name: row.name };
}

// ---------------------------------------------------------------------------
// upsertSourceStream
// ---------------------------------------------------------------------------

/**
 * Create or update a source stream.
 *
 * @param {object} db — pg Pool
 * @param {object} stream
 * @param {string} stream.source_instance_id
 * @param {string} stream.stream_key        — folder path, channel id, etc.
 * @param {string} stream.stream_type        — folder, channel, repo_contents, issues, pages
 * @param {string} stream.remote_prefix
 * @param {object} [stream.selection={}]
 * @param {object} [stream.cursor={}]
 * @param {string} [stream.correlationId]
 * @returns {Promise<{id: string, source_instance_id: string, stream_key: string}>}
 */
export async function upsertSourceStream(db, stream) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');

  const cId = stream.correlationId || correlationId();

  const result = await db.query(
    `INSERT INTO longhouse.source_streams
       (source_instance_id, stream_key, stream_type, remote_prefix, selection, cursor)
     VALUES ($1, $2, $3, $4, $5, $6)
     ON CONFLICT (source_instance_id, stream_key) DO UPDATE SET
       stream_type = EXCLUDED.stream_type,
       remote_prefix = EXCLUDED.remote_prefix,
       selection = longhouse.source_streams.selection || EXCLUDED.selection,
       cursor = longhouse.source_streams.cursor || EXCLUDED.cursor,
       updated_at = now()
     RETURNING id, source_instance_id, stream_key`,
    [
      stream.source_instance_id,
      stream.stream_key,
      stream.stream_type,
      stream.remote_prefix,
      JSON.stringify(stream.selection || {}),
      JSON.stringify(stream.cursor || {}),
    ],
  );

  const row = result.rows[0];

  // Resolve instance info for the ledger event
  const instResult = await db.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [stream.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.stream.selected',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || stream.source_instance_id}`,
    virtualPath: stream.remote_prefix,
    sourceSequence: null,
    payload: {
      source_instance_id: stream.source_instance_id,
      stream_id: row.id,
      stream_key: stream.stream_key,
      stream_type: stream.stream_type,
      remote_prefix: stream.remote_prefix,
    },
    correlationId: cId,
  });

  return { id: row.id, source_instance_id: row.source_instance_id, stream_key: row.stream_key };
}

// ---------------------------------------------------------------------------
// createSourceSyncRun
// ---------------------------------------------------------------------------

/**
 * Create a new sync run (status: running).
 *
 * @param {object} db — pg Pool
 * @param {object} run
 * @param {string} run.source_instance_id
 * @param {string} [run.stream_id] — optional, null for instance-level runs
 * @param {object} [run.metadata]
 * @param {string} [run.correlationId]
 * @returns {Promise<{id: string, source_instance_id: string, status: string, started_at: string}>}
 */
export async function createSourceSyncRun(db, run) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');

  const cId = run.correlationId || correlationId();
  const startedAt = nowIso();

  const result = await db.query(
    `INSERT INTO longhouse.source_sync_runs
       (source_instance_id, stream_id, status, started_at, metadata)
     VALUES ($1, $2::uuid, 'running', $3, $4::jsonb)
     RETURNING id, source_instance_id, stream_id, status, started_at, metadata`,
    [run.source_instance_id, run.stream_id || null, startedAt, JSON.stringify(run.metadata || {})],
  );

  const row = result.rows[0];

  // Resolve instance info
  const instResult = await db.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [run.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.sync.started',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || run.source_instance_id}`,
    sourceSequence: `${row.id}:started`,
    payload: {
      source_instance_id: run.source_instance_id,
      stream_id: run.stream_id || null,
      sync_run_id: row.id,
      started_at: startedAt,
    },
    correlationId: cId,
  });

  return {
    id: row.id,
    source_instance_id: row.source_instance_id,
    stream_id: row.stream_id,
    status: row.status,
    started_at: row.started_at,
    metadata: row.metadata || {},
  };
}

// ---------------------------------------------------------------------------
// completeSourceSyncRun
// ---------------------------------------------------------------------------

/**
 * Mark a sync run as completed with stats.
 *
 * @param {object} db — pg Pool
 * @param {string} runId
 * @param {object} stats
 * @param {number} [stats.scanned_count=0]
 * @param {number} [stats.changed_count=0]
 * @param {number} [stats.uploaded_count=0]
 * @param {number} [stats.skipped_count=0]
 * @param {number} [stats.error_count=0]
 * @param {string} [stats.error]
 * @param {string} [stats.manifest_path]
 * @param {string} [stats.correlationId]
 * @returns {Promise<object>} the updated run row
 */
export async function completeSourceSyncRun(db, runId, stats = {}) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');

  const status = stats.error ? 'failed' : 'completed';
  const finishedAt = nowIso();

  const result = await db.query(
    `UPDATE longhouse.source_sync_runs
     SET status = $2,
         finished_at = $3,
         scanned_count = $4,
         changed_count = $5,
         uploaded_count = $6,
         skipped_count = $7,
         error_count = $8,
         error = $9,
         manifest_path = $10,
         metadata = metadata || $11::jsonb
     WHERE id = $1
     RETURNING *`,
    [
      runId,
      status,
      finishedAt,
      stats.scanned_count || 0,
      stats.changed_count || 0,
      stats.uploaded_count || 0,
      stats.skipped_count || 0,
      stats.error_count || 0,
      stats.error || null,
      stats.manifest_path || null,
      JSON.stringify({ completedAt: finishedAt, ...stats.metadata }),
    ],
  );

  if (!result.rows.length) {
    throw new Error(`Sync run not found: ${runId}`);
  }

  const row = result.rows[0];
  const cId = stats.correlationId || correlationId();

  // Resolve instance info
  const instResult = await db.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [row.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.sync.completed',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || row.source_instance_id}`,
    sourceSequence: `${runId}:completed`,
    payload: {
      sync_run_id: runId,
      status,
      scanned_count: row.scanned_count,
      changed_count: row.changed_count,
      uploaded_count: row.uploaded_count,
      skipped_count: row.skipped_count,
      error_count: row.error_count,
      manifest_path: row.manifest_path,
      finished_at: finishedAt,
    },
    correlationId: cId,
  });

  return row;
}

// ---------------------------------------------------------------------------
// upsertSourceRecord
// ---------------------------------------------------------------------------

/**
 * Create or update a source record (file, message, issue, etc.).
 *
 * @param {object} db — pg Pool
 * @param {object} record
 * @param {string} record.source_instance_id
 * @param {string} [record.stream_id] — optional
 * @param {string} record.source_record_id — unique identifier within the instance
 * @param {string} [record.virtual_path]
 * @param {string} [record.object_id] — references longhouse.objects(id)
 * @param {string} [record.source_updated_at]
 * @param {string} [record.content_sha256]
 * @param {object} [record.metadata={}]
 * @param {string} [record.correlationId]
 * @returns {Promise<{id: string, source_instance_id: string, source_record_id: string}>}
 */
export async function upsertSourceRecord(db, record) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');
  if (!record.source_record_id) throw new Error('source_record_id is required.');

  const cId = record.correlationId || correlationId();
  const seenAt = nowIso();

  const result = await db.query(
    `INSERT INTO longhouse.source_records
       (source_instance_id, stream_id, source_record_id, virtual_path, object_id, source_updated_at, content_sha256, metadata)
     VALUES ($1, $2::uuid, $3, $4, $5::uuid, $6::timestamptz, $7, $8)
     ON CONFLICT (source_instance_id, source_record_id) DO UPDATE SET
       stream_id = COALESCE(EXCLUDED.stream_id, longhouse.source_records.stream_id),
       virtual_path = COALESCE(EXCLUDED.virtual_path, longhouse.source_records.virtual_path),
       object_id = COALESCE(EXCLUDED.object_id, longhouse.source_records.object_id),
       source_updated_at = COALESCE(EXCLUDED.source_updated_at, longhouse.source_records.source_updated_at),
       content_sha256 = COALESCE(EXCLUDED.content_sha256, longhouse.source_records.content_sha256),
       metadata = longhouse.source_records.metadata || EXCLUDED.metadata,
       deleted_at = NULL,
       seen_at = $9
     RETURNING id, source_instance_id, source_record_id`,
    [
      record.source_instance_id,
      record.stream_id || null,
      record.source_record_id,
      record.virtual_path || null,
      record.object_id || null,
      record.source_updated_at || null,
      record.content_sha256 || null,
      JSON.stringify(record.metadata || {}),
      seenAt,
    ],
  );

  const row = result.rows[0];

  // Resolve instance info
  const instResult = await db.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [record.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.record.ingested',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || record.source_instance_id}`,
    virtualPath: record.virtual_path || null,
    sourceSequence: record.sourceSequence || record.source_sequence || `${record.source_record_id}:${record.content_sha256 || seenAt}`,
    payload: {
      source_instance_id: record.source_instance_id,
      stream_id: record.stream_id || null,
      source_record_id: record.source_record_id,
      virtual_path: record.virtual_path || null,
      content_sha256: record.content_sha256 || null,
    },
    correlationId: cId,
    clearance: 'CONFIDENTIAL',
  });

  return { id: row.id, source_instance_id: row.source_instance_id, source_record_id: row.source_record_id };
}

export async function markSourceRecordDeleted(db, record) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');
  if (!record.source_instance_id) throw new Error('source_instance_id is required.');
  if (!record.source_record_id) throw new Error('source_record_id is required.');

  const deletedAt = record.deleted_at || record.deletedAt || nowIso();
  const metadata = record.metadata || {};
  const result = await db.query(
    `UPDATE longhouse.source_records
     SET deleted_at = $3::timestamptz,
         metadata = metadata || $4::jsonb,
         seen_at = now()
     WHERE source_instance_id = $1
       AND source_record_id = $2
     RETURNING id, source_instance_id, source_record_id, virtual_path, content_sha256`,
    [record.source_instance_id, record.source_record_id, deletedAt, JSON.stringify(metadata)],
  );

  if (!result.rows.length) return null;
  const row = result.rows[0];

  const instResult = await db.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [record.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.record.deleted',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || record.source_instance_id}`,
    virtualPath: record.virtual_path || row.virtual_path || null,
    sourceSequence: `${record.source_record_id}:deleted:${deletedAt}`,
    payload: {
      source_instance_id: record.source_instance_id,
      stream_id: record.stream_id || null,
      source_record_id: record.source_record_id,
      virtual_path: record.virtual_path || row.virtual_path || null,
      deleted_at: deletedAt,
    },
    correlationId: record.correlationId || record.correlation_id || null,
    clearance: record.clearance || 'CONFIDENTIAL',
    adapterId: instance.adapter_id || null,
  });

  return row;
}

// ---------------------------------------------------------------------------
// getSourceInstance
// ---------------------------------------------------------------------------

/**
 * Retrieve a source instance by id.
 *
 * @param {object} db — pg Pool
 * @param {string} id
 * @returns {Promise<object|null>} the instance row or null
 */
export async function getSourceInstance(db, id) {
  if (!db) db = getPool();
  if (!db) throw new Error('DATABASE_URL is required for source adapter operations.');

  const result = await db.query(
    `SELECT si.*, sa.display_name AS adapter_display_name, sa.capabilities, sa.config_schema, sa.stream_schema
     FROM longhouse.source_instances si
     LEFT JOIN longhouse.source_adapters sa ON sa.id = si.adapter_id
     WHERE si.id = $1`,
    [id],
  );

  return result.rows[0] || null;
}

// ---------------------------------------------------------------------------
// API-facing query/update helpers
// ---------------------------------------------------------------------------

function requirePool(db) {
  const pool = db || getPool();
  if (!pool) throw new Error('DATABASE_URL is required for source adapter operations.');
  return pool;
}

function positiveLimit(value, fallback = 200, max = 1000) {
  const number = Number(value || fallback);
  if (!Number.isFinite(number)) return fallback;
  return Math.max(1, Math.min(Math.floor(number), max));
}

function addOptionalFilter(where, values, column, value, cast = '') {
  if (value === undefined || value === null || value === '') return;
  values.push(value);
  where.push(`${column} = $${values.length}${cast}`);
}

export async function listSourceAdapters(db, filters = {}) {
  const pool = requirePool(db);
  const where = [];
  const values = [];
  addOptionalFilter(where, values, 'enabled', filters.enabled, '::boolean');
  const result = await pool.query(
    `SELECT *
     FROM longhouse.source_adapters
     ${where.length ? `WHERE ${where.join(' AND ')}` : ''}
     ORDER BY id`,
    values,
  );
  return result.rows;
}

export async function listSourceInstances(db, filters = {}) {
  const pool = requirePool(db);
  const where = [];
  const values = [];
  addOptionalFilter(where, values, 'si.adapter_id', filters.adapter_id || filters.adapterId);
  addOptionalFilter(where, values, 'si.owner_name', filters.owner_name || filters.ownerName);
  addOptionalFilter(where, values, 'si.status', filters.status);
  addOptionalFilter(where, values, 'si.enabled', filters.enabled, '::boolean');
  values.push(positiveLimit(filters.limit));
  const limitParam = values.length;
  const result = await pool.query(
    `SELECT si.*, sa.display_name AS adapter_display_name, sa.capabilities, sa.config_schema, sa.stream_schema
     FROM longhouse.source_instances si
     LEFT JOIN longhouse.source_adapters sa ON sa.id = si.adapter_id
     ${where.length ? `WHERE ${where.join(' AND ')}` : ''}
     ORDER BY si.updated_at DESC, si.created_at DESC
     LIMIT $${limitParam}`,
    values,
  );
  return result.rows;
}

export async function updateSourceInstance(db, id, patch = {}) {
  const pool = requirePool(db);
  const sets = [];
  const values = [id];
  const assigned = new Set();

  function setColumn(inputKey, column, transform = (value) => value) {
    if (!(inputKey in patch) || assigned.has(column)) return;
    assigned.add(column);
    values.push(transform(patch[inputKey]));
    sets.push(`${column} = $${values.length}`);
  }

  setColumn('name', 'name', (value) => String(value).trim());
  setColumn('owner_name', 'owner_name');
  setColumn('ownerName', 'owner_name');
  setColumn('owner_token_id', 'owner_token_id');
  setColumn('ownerTokenId', 'owner_token_id');
  setColumn('remote_prefix', 'remote_prefix');
  setColumn('remotePrefix', 'remote_prefix');
  if ('config' in patch) {
    values.push(JSON.stringify(patch.config || {}));
    sets.push(`config = config || $${values.length}::jsonb`);
  }
  setColumn('secret_ref', 'secret_ref');
  setColumn('secretRef', 'secret_ref');
  setColumn('clearance', 'clearance');
  setColumn('status', 'status');
  setColumn('enabled', 'enabled', Boolean);

  if (!sets.length) return getSourceInstance(pool, id);
  sets.push('updated_at = now()');

  const result = await pool.query(
    `UPDATE longhouse.source_instances
     SET ${sets.join(', ')}
     WHERE id = $1
     RETURNING *`,
    values,
  );
  if (!result.rows.length) throw new Error(`Source instance not found: ${id}`);
  const row = result.rows[0];

  await emitSourceEvent({
    eventType: 'source.instance.updated',
    sourceId: `${row.adapter_id}:${row.name}`,
    virtualPath: row.remote_prefix,
    payload: { source_instance_id: row.id, patch_keys: Object.keys(patch) },
    correlationId: patch.correlationId || patch.correlation_id || null,
    adapterId: row.adapter_id,
  });

  return getSourceInstance(pool, id);
}

export async function listSourceStreams(db, filters = {}) {
  const pool = requirePool(db);
  const where = [];
  const values = [];
  addOptionalFilter(where, values, 'ss.source_instance_id', filters.source_instance_id || filters.sourceInstanceId, '::uuid');
  addOptionalFilter(where, values, 'ss.stream_type', filters.stream_type || filters.streamType);
  addOptionalFilter(where, values, 'ss.enabled', filters.enabled, '::boolean');
  values.push(positiveLimit(filters.limit));
  const limitParam = values.length;
  const result = await pool.query(
    `SELECT ss.*, si.adapter_id, si.name AS source_instance_name, si.owner_name, si.clearance AS source_clearance
     FROM longhouse.source_streams ss
     JOIN longhouse.source_instances si ON si.id = ss.source_instance_id
     ${where.length ? `WHERE ${where.join(' AND ')}` : ''}
     ORDER BY ss.updated_at DESC, ss.created_at DESC
     LIMIT $${limitParam}`,
    values,
  );
  return result.rows;
}

export async function getSourceStream(db, id) {
  const pool = requirePool(db);
  const result = await pool.query(
    `SELECT ss.*, si.adapter_id, si.name AS source_instance_name, si.owner_name, si.clearance AS source_clearance
     FROM longhouse.source_streams ss
     JOIN longhouse.source_instances si ON si.id = ss.source_instance_id
     WHERE ss.id = $1`,
    [id],
  );
  return result.rows[0] || null;
}

export async function updateSourceStream(db, id, patch = {}) {
  const pool = requirePool(db);
  const sets = [];
  const values = [id];
  const assigned = new Set();

  function setColumn(inputKey, column, transform = (value) => value) {
    if (!(inputKey in patch) || assigned.has(column)) return;
    assigned.add(column);
    values.push(transform(patch[inputKey]));
    sets.push(`${column} = $${values.length}`);
  }

  setColumn('stream_key', 'stream_key');
  setColumn('streamKey', 'stream_key');
  setColumn('stream_type', 'stream_type');
  setColumn('streamType', 'stream_type');
  setColumn('remote_prefix', 'remote_prefix');
  setColumn('remotePrefix', 'remote_prefix');
  if ('selection' in patch) {
    values.push(JSON.stringify(patch.selection || {}));
    sets.push(`selection = selection || $${values.length}::jsonb`);
  }
  if ('cursor' in patch) {
    values.push(JSON.stringify(patch.cursor || {}));
    sets.push(`cursor = cursor || $${values.length}::jsonb`);
  }
  setColumn('enabled', 'enabled', Boolean);

  if (!sets.length) return getSourceStream(pool, id);
  sets.push('updated_at = now()');

  const result = await pool.query(
    `UPDATE longhouse.source_streams
     SET ${sets.join(', ')}
     WHERE id = $1
     RETURNING *`,
    values,
  );
  if (!result.rows.length) throw new Error(`Source stream not found: ${id}`);
  const row = result.rows[0];

  await emitSourceEvent({
    eventType: 'source.stream.updated',
    sourceId: row.source_instance_id,
    virtualPath: row.remote_prefix,
    payload: { source_stream_id: row.id, source_instance_id: row.source_instance_id, patch_keys: Object.keys(patch) },
    correlationId: patch.correlationId || patch.correlation_id || null,
  });

  return getSourceStream(pool, id);
}

export async function getSourceSyncRun(db, id) {
  const pool = requirePool(db);
  const result = await pool.query(
    `SELECT sr.*, si.adapter_id, si.name AS source_instance_name, si.remote_prefix AS source_instance_remote_prefix, si.owner_name, si.clearance AS source_clearance
     FROM longhouse.source_sync_runs sr
     JOIN longhouse.source_instances si ON si.id = sr.source_instance_id
     WHERE sr.id = $1`,
    [id],
  );
  return result.rows[0] || null;
}

export async function listSourceSyncRuns(db, filters = {}) {
  const pool = requirePool(db);
  const where = [];
  const values = [];
  addOptionalFilter(where, values, 'sr.source_instance_id', filters.source_instance_id || filters.sourceInstanceId, '::uuid');
  addOptionalFilter(where, values, 'sr.stream_id', filters.stream_id || filters.streamId, '::uuid');
  addOptionalFilter(where, values, 'sr.status', filters.status);
  values.push(positiveLimit(filters.limit));
  const limitParam = values.length;
  const result = await pool.query(
    `SELECT sr.*, si.adapter_id, si.name AS source_instance_name, si.remote_prefix AS source_instance_remote_prefix, si.owner_name, si.clearance AS source_clearance
     FROM longhouse.source_sync_runs sr
     JOIN longhouse.source_instances si ON si.id = sr.source_instance_id
     ${where.length ? `WHERE ${where.join(' AND ')}` : ''}
     ORDER BY sr.started_at DESC
     LIMIT $${limitParam}`,
    values,
  );
  return result.rows;
}

export async function failSourceSyncRun(db, runId, stats = {}) {
  const pool = requirePool(db);
  const finishedAt = nowIso();
  const result = await pool.query(
    `UPDATE longhouse.source_sync_runs
     SET status = 'failed',
         finished_at = $2,
         scanned_count = $3,
         changed_count = $4,
         uploaded_count = $5,
         skipped_count = $6,
         error_count = $7,
         error = $8,
         manifest_path = $9,
         metadata = metadata || $10::jsonb
     WHERE id = $1
     RETURNING *`,
    [
      runId,
      finishedAt,
      stats.scanned_count || stats.scannedCount || 0,
      stats.changed_count || stats.changedCount || 0,
      stats.uploaded_count || stats.uploadedCount || 0,
      stats.skipped_count || stats.skippedCount || 0,
      stats.error_count || stats.errorCount || 1,
      stats.error || 'Sync run failed.',
      stats.manifest_path || stats.manifestPath || null,
      JSON.stringify({ failedAt: finishedAt, ...(stats.metadata || {}) }),
    ],
  );
  if (!result.rows.length) throw new Error(`Sync run not found: ${runId}`);
  const row = result.rows[0];

  const instResult = await pool.query(
    'SELECT adapter_id, name FROM longhouse.source_instances WHERE id = $1',
    [row.source_instance_id],
  );
  const instance = instResult.rows[0] || {};

  await emitSourceEvent({
    eventType: 'source.sync.failed',
    sourceId: `${instance.adapter_id || '?'}:${instance.name || row.source_instance_id}`,
    sourceSequence: `${runId}:failed`,
    payload: {
      sync_run_id: runId,
      error: row.error,
      error_count: row.error_count,
      finished_at: finishedAt,
    },
    correlationId: stats.correlationId || stats.correlation_id || null,
  });

  return row;
}

export async function findObjectIdByVirtualPath(db, virtualPath) {
  const pool = requirePool(db);
  const result = await pool.query('SELECT id FROM longhouse.objects WHERE virtual_path = $1 LIMIT 1', [virtualPath]);
  return result.rows[0]?.id || null;
}
