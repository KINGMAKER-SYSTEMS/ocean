import crypto from 'node:crypto';
import path from 'node:path';
import { promises as fs } from 'node:fs';

import { recordObjectWrite } from '../metadata.mjs';
import { HttpError } from '../mounts/adapter.mjs';
import { resolveMount } from '../mounts/router.mjs';
import {
  completeSourceSyncRun,
  createSourceSyncRun,
  failSourceSyncRun,
  findObjectIdByVirtualPath,
  markSourceRecordDeleted,
  upsertSourceRecord,
} from '../sources.mjs';
import {
  getCursor,
  mirrorStreamCursor,
  setCursor,
  setCursorFailed,
  sourceCursorId,
} from './cursors.mjs';
import { redactSecrets } from './secrets.mjs';

export class SourceRunner {
  constructor({ adapterId = null, name = null } = {}) {
    this.adapterId = adapterId;
    this.name = name || adapterId || this.constructor.name;
  }

  async plan(_ctx) {
    return {};
  }

  async *iterate(_ctx) {
    throw new Error(`${this.name} does not implement iterate(ctx).`);
  }

  async commit(_ctx, _stats) {
    return {};
  }
}

export function normalizeApiPath(input = '/') {
  if (typeof input !== 'string') throw new Error('Path must be a string.');
  if (input.length > 4096) throw new Error('Path is too long.');
  if (input.includes('\0')) throw new Error('Path contains a NUL byte.');
  if (input.includes('\\')) throw new Error('Use POSIX paths with forward slashes, not backslashes.');
  let value = input.trim() || '/';
  if (!value.startsWith('/')) value = `/${value}`;
  const parts = value.split('/').filter((part) => part && part !== '.');
  if (parts.includes('..')) throw new Error('Path traversal is not allowed.');
  return parts.length ? `/${parts.join('/')}` : '/';
}

export function joinApiPath(prefix, ...parts) {
  const base = normalizeApiPath(prefix || '/');
  const cleanParts = parts
    .flat()
    .filter((part) => part !== undefined && part !== null && String(part) !== '')
    .flatMap((part) => String(part).split('/'))
    .filter((part) => part && part !== '.');
  if (cleanParts.some((part) => part === '..' || part.includes('\0') || part.includes('\\'))) {
    throw new Error('Path traversal is not allowed.');
  }
  return normalizeApiPath(`${base === '/' ? '' : base}/${cleanParts.join('/')}`);
}

export function pathInsidePrefix(apiPath, prefix) {
  const pathValue = normalizeApiPath(apiPath);
  const prefixValue = normalizeApiPath(prefix || '/');
  return prefixValue === '/' || pathValue === prefixValue || pathValue.startsWith(`${prefixValue}/`);
}

function sha256Buffer(buffer) {
  return crypto.createHash('sha256').update(buffer).digest('hex');
}

function packetBuffer(packet) {
  if (packet.bytes !== undefined && packet.bytes !== null) return Buffer.from(packet.bytes);
  if (packet.content !== undefined && packet.content !== null) {
    return Buffer.isBuffer(packet.content) ? packet.content : Buffer.from(String(packet.content), 'utf8');
  }
  return null;
}

async function existingSourceRecord(db, sourceInstanceId, sourceRecordId) {
  const result = await db.query(
    `SELECT id, object_id, content_sha256, deleted_at, virtual_path
     FROM longhouse.source_records
     WHERE source_instance_id = $1
       AND source_record_id = $2
     LIMIT 1`,
    [sourceInstanceId, sourceRecordId],
  );
  return result.rows[0] || null;
}

async function writePacketObject(state, packet, buffer, principal = {}) {
  const apiPath = normalizeApiPath(packet.virtual_path);
  const { mount, adapter, relPath } = await resolveMount(state, apiPath);
  if (mount.mode === 'readonly') throw new HttpError(403, `Mount "${mount.id}" is read-only.`);

  const tmpPath = path.join(state.tmpRoot, `${Date.now()}-${crypto.randomBytes(8).toString('hex')}.source-runner`);
  let wroteTmp = false;
  try {
    await fs.mkdir(path.dirname(tmpPath), { recursive: true });
    await fs.writeFile(tmpPath, buffer);
    wroteTmp = true;
    await adapter.write(relPath, tmpPath);
    wroteTmp = false;
    const info = await adapter.stat(relPath, apiPath);
    const objectId = await recordObjectWrite(state, apiPath, info, {
      id: principal.id || `source-runner:${packet.adapter_id || 'unknown'}`,
      name: principal.name || 'source-runner',
    }, mount.id, relPath);
    return { info, objectId };
  } finally {
    if (wroteTmp) await fs.rm(tmpPath, { force: true }).catch(() => {});
  }
}

function normalizeStats(stats) {
  return {
    scanned_count: stats.scanned_count || 0,
    changed_count: stats.changed_count || 0,
    uploaded_count: stats.uploaded_count || 0,
    skipped_count: stats.skipped_count || 0,
    error_count: stats.error_count || 0,
    metadata: redactSecrets(stats.metadata || {}),
  };
}

export async function runSourceSync(db, state, options = {}) {
  const {
    instance,
    stream = null,
    runner,
    principal = {},
    mode = 'incremental',
    correlationId = null,
    maxRecords = null,
  } = options;

  if (!db) throw new Error('DATABASE_URL is required for source runner sync.');
  if (!state) throw new Error('Ocean Bedrock state is required for source runner sync.');
  if (!instance?.id) throw new Error('Source instance is required for source runner sync.');
  if (!runner?.iterate) throw new Error(`No SourceRunner provided for adapter ${instance.adapter_id}.`);

  const sourceId = sourceCursorId(instance, stream);
  const oldCursor = await getCursor(db, sourceId);
  const run = await createSourceSyncRun(db, {
    source_instance_id: instance.id,
    stream_id: stream?.id || null,
    metadata: redactSecrets({
      runner: runner.name || runner.constructor?.name || instance.adapter_id,
      adapter_id: instance.adapter_id,
      mode,
      source_cursor_id: sourceId,
      previous_cursor: oldCursor?.cursor_value || null,
    }),
    correlationId,
  });

  const stats = normalizeStats({});
  let nextCursorValue = oldCursor?.cursor_value ?? null;
  let lastEventTimestamp = null;

  const ctx = {
    db,
    state,
    instance,
    stream,
    mode,
    run,
    sourceId,
    cursor: oldCursor,
    cursorValue: oldCursor?.cursor_value || null,
    principal,
    maxRecords,
    stats,
    setNextCursor(value, metadata = {}) {
      if (value !== undefined && value !== null) nextCursorValue = String(value);
      if (metadata?.last_event_timestamp || metadata?.lastEventTimestamp) {
        lastEventTimestamp = metadata.last_event_timestamp || metadata.lastEventTimestamp;
      }
    },
  };

  try {
    const plan = await runner.plan(ctx);
    if (plan?.cursor_value !== undefined || plan?.cursorValue !== undefined) nextCursorValue = String(plan.cursor_value ?? plan.cursorValue);

    const prefix = stream?.remote_prefix || instance.remote_prefix || '/';
    let processed = 0;

    for await (const rawPacket of runner.iterate(ctx)) {
      if (!rawPacket) continue;
      if (maxRecords && processed >= Number(maxRecords)) break;
      processed += 1;
      stats.scanned_count += 1;

      const packet = { ...rawPacket };
      if (!packet.source_record_id) throw new Error('Source runner packet missing source_record_id.');
      if (packet.cursor_value !== undefined || packet.cursor !== undefined) {
        nextCursorValue = String(packet.cursor_value ?? packet.cursor);
      }
      if (packet.source_updated_at) lastEventTimestamp = packet.source_updated_at;

      if (packet.virtual_path) packet.virtual_path = normalizeApiPath(packet.virtual_path);
      if (packet.virtual_path && !pathInsidePrefix(packet.virtual_path, prefix)) {
        throw new Error(`Runner packet path ${packet.virtual_path} escapes stream prefix ${prefix}.`);
      }

      if (packet.deleted) {
        const deleted = await markSourceRecordDeleted(db, {
          source_instance_id: instance.id,
          stream_id: stream?.id || null,
          source_record_id: packet.source_record_id,
          virtual_path: packet.virtual_path || null,
          deleted_at: packet.deleted_at || null,
          metadata: redactSecrets(packet.metadata || {}),
          correlationId,
          clearance: instance.clearance || 'CONFIDENTIAL',
        });
        if (deleted) stats.changed_count += 1;
        else stats.skipped_count += 1;
        continue;
      }

      if (!packet.virtual_path) throw new Error(`Source runner packet ${packet.source_record_id} missing virtual_path.`);
      const buffer = packetBuffer(packet);
      if (!buffer) {
        await upsertSourceRecord(db, {
          source_instance_id: instance.id,
          stream_id: stream?.id || null,
          source_record_id: packet.source_record_id,
          virtual_path: packet.virtual_path,
          object_id: packet.object_id || null,
          source_updated_at: packet.source_updated_at || null,
          content_sha256: packet.content_sha256 || null,
          metadata: redactSecrets(packet.metadata || {}),
          correlationId,
        });
        stats.changed_count += 1;
        continue;
      }

      const contentSha256 = packet.content_sha256 || sha256Buffer(buffer);
      const existing = await existingSourceRecord(db, instance.id, packet.source_record_id);
      if (existing?.content_sha256 === contentSha256 && !existing.deleted_at) {
        stats.skipped_count += 1;
        continue;
      }

      const { objectId } = await writePacketObject(state, packet, buffer, principal);
      await upsertSourceRecord(db, {
        source_instance_id: instance.id,
        stream_id: stream?.id || null,
        source_record_id: packet.source_record_id,
        virtual_path: packet.virtual_path,
        object_id: objectId || packet.object_id || await findObjectIdByVirtualPath(db, packet.virtual_path),
        source_updated_at: packet.source_updated_at || null,
        content_sha256: contentSha256,
        sourceSequence: existing?.deleted_at && existing.content_sha256 === contentSha256
          ? `${packet.source_record_id}:${contentSha256}:restored:${Date.now()}`
          : undefined,
        metadata: redactSecrets({
          ...(packet.metadata || {}),
          content_type: packet.content_type || null,
          byte_length: buffer.length,
        }),
        correlationId,
      });
      stats.changed_count += 1;
      stats.uploaded_count += 1;
    }

    const commitResult = await runner.commit(ctx, stats);
    if (commitResult?.cursor_value !== undefined || commitResult?.cursorValue !== undefined || commitResult?.cursor !== undefined) {
      nextCursorValue = String(commitResult.cursor_value ?? commitResult.cursorValue ?? commitResult.cursor);
    }
    const commitMetadata = redactSecrets(commitResult?.metadata || {});

    await setCursor(db, sourceId, {
      source_type: instance.adapter_id,
      cursor_value: nextCursorValue,
      last_event_timestamp: lastEventTimestamp,
      status: 'active',
      metadata: redactSecrets({ sync_run_id: run.id, stream_id: stream?.id || null, mode }),
    });
    await mirrorStreamCursor(db, stream?.id || null, {
      source_id: sourceId,
      cursor_value: nextCursorValue,
      status: 'active',
      metadata: { sync_run_id: run.id, mode },
    });

    const completed = await completeSourceSyncRun(db, run.id, {
      ...stats,
      metadata: redactSecrets({
        source_cursor_id: sourceId,
        cursor_value: nextCursorValue,
        mode,
        ...commitMetadata,
      }),
      correlationId,
    });

    return { syncRun: completed, stats, sourceCursorId: sourceId, cursorValue: nextCursorValue };
  } catch (error) {
    stats.error_count += 1;
    await setCursorFailed(db, sourceId, {
      source_type: instance.adapter_id,
      error: error.message,
      metadata: { sync_run_id: run.id, mode },
    }).catch(() => {});
    await mirrorStreamCursor(db, stream?.id || null, {
      source_id: sourceId,
      cursor_value: oldCursor?.cursor_value || null,
      status: 'failed',
      metadata: { sync_run_id: run.id, mode, error: error.message },
    }).catch(() => {});
    await failSourceSyncRun(db, run.id, {
      ...stats,
      error: error.message,
      metadata: redactSecrets({ source_cursor_id: sourceId, mode }),
      correlationId,
    }).catch(() => {});
    throw error;
  }
}
