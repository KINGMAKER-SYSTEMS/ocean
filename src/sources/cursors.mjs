export function sourceCursorId(instance, stream = null) {
  if (!instance?.adapter_id || !instance?.id) throw new Error('sourceCursorId requires instance.adapter_id and instance.id.');
  const streamPart = stream?.id || stream?.stream_key || 'instance';
  return `${instance.adapter_id}:${instance.id}:${streamPart}`;
}

function requirePool(db) {
  if (!db) throw new Error('DATABASE_URL is required for source cursor operations.');
  return db;
}

export async function getCursor(db, sourceId) {
  const pool = requirePool(db);
  const result = await pool.query(
    `SELECT source_id, source_type, cursor_value, last_event_timestamp, last_ingested_at,
            status, error, metadata, updated_at
     FROM longhouse.ledger_source_cursors
     WHERE source_id = $1`,
    [sourceId],
  );
  return result.rows[0] || null;
}

export async function setCursor(db, sourceId, cursor = {}) {
  const pool = requirePool(db);
  const result = await pool.query(
    `INSERT INTO longhouse.ledger_source_cursors
       (source_id, source_type, cursor_value, last_event_timestamp, last_ingested_at, status, error, metadata, updated_at)
     VALUES ($1, $2, $3, $4::timestamptz, now(), $5, $6, $7::jsonb, now())
     ON CONFLICT (source_id) DO UPDATE SET
       source_type = EXCLUDED.source_type,
       cursor_value = COALESCE(EXCLUDED.cursor_value, longhouse.ledger_source_cursors.cursor_value),
       last_event_timestamp = COALESCE(EXCLUDED.last_event_timestamp, longhouse.ledger_source_cursors.last_event_timestamp),
       last_ingested_at = now(),
       status = EXCLUDED.status,
       error = EXCLUDED.error,
       metadata = longhouse.ledger_source_cursors.metadata || EXCLUDED.metadata,
       updated_at = now()
     RETURNING source_id, source_type, cursor_value, last_event_timestamp, last_ingested_at,
               status, error, metadata, updated_at`,
    [
      sourceId,
      cursor.source_type || cursor.sourceType || 'source',
      cursor.cursor_value ?? cursor.cursorValue ?? null,
      cursor.last_event_timestamp || cursor.lastEventTimestamp || null,
      cursor.status || 'active',
      cursor.error || null,
      JSON.stringify(cursor.metadata || {}),
    ],
  );
  return result.rows[0];
}

export async function setCursorFailed(db, sourceId, { source_type = 'source', error, metadata = {} } = {}) {
  return setCursor(db, sourceId, {
    source_type,
    cursor_value: null,
    status: 'failed',
    error: error || 'Source sync failed.',
    metadata,
  });
}

export async function mirrorStreamCursor(db, streamId, cursor = {}) {
  if (!streamId) return null;
  const pool = requirePool(db);
  const patch = {
    source_id: cursor.source_id || cursor.sourceId || null,
    cursor_value: cursor.cursor_value ?? cursor.cursorValue ?? null,
    status: cursor.status || 'active',
    updated_at: new Date().toISOString(),
    ...(cursor.metadata ? { metadata: cursor.metadata } : {}),
  };
  const result = await pool.query(
    `UPDATE longhouse.source_streams
     SET cursor = cursor || $2::jsonb,
         updated_at = now()
     WHERE id = $1
     RETURNING id, cursor`,
    [streamId, JSON.stringify(patch)],
  );
  return result.rows[0] || null;
}
