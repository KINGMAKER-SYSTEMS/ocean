import http from 'node:http';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promises as fs } from 'node:fs';
import { createReadStream, createWriteStream } from 'node:fs';
import { Readable } from 'node:stream';
import crypto from 'node:crypto';

import {
  publicInviteRecord,
  publicTokenRecord,
} from './auth.mjs';
import { createAuthStore } from './auth-store.mjs';
import {
  createRoomsStore,
  normalizeRoomId,
  roomScope,
  validateAgentDescriptor,
  validateRegistrationKey,
  validateTtlMinutes,
  RoomsError,
} from './rooms.mjs';
import {
  createLedgerStore,
} from './ledger.mjs';
import {
  claimNextIngestJob,
  completeIngestJob,
  getPool,
  processIngestJob,
  recordObjectDelete,
  recordObjectWrite,
} from './metadata.mjs';
import { HttpError } from './mounts/adapter.mjs';
import { invalidateMountRegistry, loadMounts } from './mounts/registry.mjs';
import { resolveMount } from './mounts/router.mjs';
import {
  graphNeighborhood,
  listGraphNodes,
} from './graph.mjs';
import {
  runOceanContextTriage,
  toolboxManifest,
} from './ocean-context.mjs';
import {
  semanticSearch,
  semanticStatus,
} from './semantic.mjs';
import {
  completeSourceSyncRun,
  createSourceSyncRun,
  failSourceSyncRun,
  findObjectIdByVirtualPath,
  getSourceInstance,
  getSourceStream,
  getSourceSyncRun,
  listSourceAdapters,
  listSourceInstances,
  listSourceStreams,
  listSourceSyncRuns,
  updateSourceInstance,
  updateSourceStream,
  upsertSourceInstance,
  upsertSourceRecord,
  upsertSourceStream,
} from './sources.mjs';
import { getRunner, listRunnerAdapters } from './sources/registry.mjs';
import { runSourceSync } from './sources/runner.mjs';
import { validateSecretRef } from './sources/secrets.mjs';

export const VERSION = '0.1.0';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const ROLE_PERMISSIONS = {
  readonly: new Set(['read']),
  contributor: new Set(['read', 'write', 'lock']),
  readwrite: new Set(['read', 'write', 'delete', 'lock']),
  agent: new Set(['read', 'write', 'delete', 'lock']),
  admin: new Set(['read', 'write', 'delete', 'lock', 'admin']),
};

const MIME_TYPES = new Map([
  ['.txt', 'text/plain; charset=utf-8'],
  ['.md', 'text/markdown; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'],
  ['.yaml', 'application/yaml; charset=utf-8'],
  ['.yml', 'application/yaml; charset=utf-8'],
  ['.csv', 'text/csv; charset=utf-8'],
  ['.html', 'text/html; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.mjs', 'text/javascript; charset=utf-8'],
  ['.ts', 'text/typescript; charset=utf-8'],
  ['.pdf', 'application/pdf'],
  ['.png', 'image/png'],
  ['.jpg', 'image/jpeg'],
  ['.jpeg', 'image/jpeg'],
  ['.gif', 'image/gif'],
  ['.webp', 'image/webp'],
  ['.svg', 'image/svg+xml'],
  ['.zip', 'application/zip'],
]);

function parseSize(value, fallback) {
  if (typeof value === 'number') return value;
  if (!value) return fallback;
  const match = String(value).trim().toLowerCase().match(/^(\d+(?:\.\d+)?)(b|kb|mb|gb)?$/);
  if (!match) return fallback;
  const amount = Number(match[1]);
  const unit = match[2] || 'b';
  const multipliers = { b: 1, kb: 1024, mb: 1024 ** 2, gb: 1024 ** 3 };
  return Math.floor(amount * multipliers[unit]);
}

function jsonResponse(res, status, body) {
  const payload = `${JSON.stringify(body, null, 2)}\n`;
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
  });
  res.end(payload);
}

function textResponse(res, status, body, contentType = 'text/plain; charset=utf-8') {
  res.writeHead(status, {
    'content-type': contentType,
    'content-length': Buffer.byteLength(body),
  });
  res.end(body);
}

function setCorsHeaders(req, res) {
  const origin = envValue('OCEAN_BEDROCK_CORS_ORIGIN', 'RT_NAS_CORS_ORIGIN', '*');
  res.setHeader('access-control-allow-origin', origin);
  res.setHeader('access-control-allow-methods', 'GET,POST,PUT,PATCH,DELETE,OPTIONS');
  res.setHeader('access-control-allow-headers', 'authorization,content-type,x-ocean-bedrock-token,x-ocean-bedrock-lock,x-rt-nas-token,x-rt-nas-lock,if-match,if-none-match');
  res.setHeader('access-control-max-age', '86400');
  const requestHeaders = req.headers['access-control-request-headers'];
  if (requestHeaders) res.setHeader('access-control-allow-headers', requestHeaders);
}

function normalizeApiPath(input = '/') {
  if (typeof input !== 'string') throw new HttpError(400, 'Path must be a string.');
  if (input.length > 4096) throw new HttpError(400, 'Path is too long.');
  if (input.includes('\0')) throw new HttpError(400, 'Path contains a NUL byte.');
  if (input.includes('\\')) throw new HttpError(400, 'Use POSIX paths with forward slashes, not backslashes.');

  let value = input.trim() || '/';
  if (!value.startsWith('/')) value = `/${value}`;
  const parts = value.split('/').filter((part) => part && part !== '.');
  if (parts.includes('..')) throw new HttpError(400, 'Path traversal is not allowed.');
  return parts.length ? `/${parts.join('/')}` : '/';
}

function pathInScopes(apiPath, scopes = ['/']) {
  return scopes.some((scope) => {
    const normalizedScope = normalizeApiPath(scope);
    return normalizedScope === '/' || apiPath === normalizedScope || apiPath.startsWith(`${normalizedScope}/`);
  });
}

function requirePermission(principal, permission) {
  const allowed = ROLE_PERMISSIONS[principal.role] || ROLE_PERMISSIONS.readonly;
  if (!allowed.has(permission)) {
    throw new HttpError(403, `Token role "${principal.role}" does not allow ${permission}.`);
  }
}

function requirePathScope(principal, apiPath) {
  if (!pathInScopes(apiPath, principal.scopes || ['/'])) {
    throw new HttpError(403, `Token is not scoped for ${apiPath}.`);
  }
}

const CLEARANCE_RANK = {
  PUBLIC: 0,
  UNCLASSIFIED: 1,
  CONFIDENTIAL: 2,
  SECRET: 3,
  TOP_SECRET: 4,
};

const ROLE_MAX_CLEARANCE = {
  readonly: 'UNCLASSIFIED',
  contributor: 'CONFIDENTIAL',
  readwrite: 'CONFIDENTIAL',
  agent: 'SECRET',
  admin: 'TOP_SECRET',
};

function principalMaxClearance(principal) {
  return ROLE_MAX_CLEARANCE[principal.role] || ROLE_MAX_CLEARANCE.readonly;
}

function clearanceAllowed(principal, clearance = 'UNCLASSIFIED') {
  return (CLEARANCE_RANK[clearance] ?? CLEARANCE_RANK.TOP_SECRET) <= CLEARANCE_RANK[principalMaxClearance(principal)];
}

function requireClearance(principal, clearance = 'UNCLASSIFIED') {
  if (!clearanceAllowed(principal, clearance)) {
    throw new HttpError(403, `Token role "${principal.role}" is not cleared for ${clearance}.`);
  }
}

function ledgerEventVisibleToPrincipal(event, principal) {
  const pathAllowed = !event.virtual_path || pathInScopes(normalizeApiPath(event.virtual_path), principal.scopes || ['/']);
  return pathAllowed && clearanceAllowed(principal, event.clearance || 'UNCLASSIFIED');
}

function extractToken(req, url) {
  const authorization = req.headers.authorization || '';
  const bearer = authorization.match(/^Bearer\s+(.+)$/i);
  if (bearer) return bearer[1].trim();
  if (req.headers['x-ocean-bedrock-token']) return String(req.headers['x-ocean-bedrock-token']).trim();
  if (req.headers['x-rt-nas-token']) return String(req.headers['x-rt-nas-token']).trim();
  if (url.searchParams.has('token')) return url.searchParams.get('token');
  return null;
}

async function authenticate(req, url, state) {
  const token = extractToken(req, url);
  if (!token) throw new HttpError(401, 'Missing token. Use Authorization: Bearer <token>.');
  const record = await state.auth.findToken(token);
  if (!record) throw new HttpError(403, 'Invalid, revoked, or expired token.');
  return {
    id: record.id,
    name: record.name,
    role: record.role,
    scopes: record.scopes || ['/'],
  };
}


async function readJsonBody(req, maxBytes) {
  const chunks = [];
  let total = 0;
  for await (const chunk of req) {
    total += chunk.length;
    if (total > maxBytes) throw new HttpError(413, 'JSON body is too large.');
    chunks.push(chunk);
  }
  const raw = Buffer.concat(chunks).toString('utf8').trim();
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch (error) {
    throw new HttpError(400, `Invalid JSON: ${error.message}`);
  }
}

async function receiveFile(req, tmpPath, maxBytes) {
  const contentLength = Number(req.headers['content-length'] || 0);
  if (contentLength && contentLength > maxBytes) throw new HttpError(413, `Upload exceeds limit of ${maxBytes} bytes.`);

  await fs.mkdir(path.dirname(tmpPath), { recursive: true });
  return new Promise((resolve, reject) => {
    const out = createWriteStream(tmpPath, { flags: 'wx' });
    let bytes = 0;
    let settled = false;

    function fail(error) {
      if (settled) return;
      settled = true;
      out.destroy();
      fs.rm(tmpPath, { force: true }).catch(() => {});
      reject(error);
    }

    req.on('data', (chunk) => {
      bytes += chunk.length;
      if (bytes > maxBytes) {
        fail(new HttpError(413, `Upload exceeds limit of ${maxBytes} bytes.`));
        req.destroy();
        return;
      }
      if (!out.write(chunk)) {
        req.pause();
        out.once('drain', () => req.resume());
      }
    });

    req.on('end', () => {
      out.end(() => {
        if (!settled) {
          settled = true;
          resolve(bytes);
        }
      });
    });

    req.on('error', fail);
    out.on('error', fail);
  });
}

function contentTypeFor(apiPath) {
  return MIME_TYPES.get(path.extname(apiPath).toLowerCase()) || 'application/octet-stream';
}

async function appendAudit(state, event) {
  const ts = new Date().toISOString();
  const auditEvent = { ts, ...event };
  const line = JSON.stringify(auditEvent);
  try {
    await fs.mkdir(path.dirname(state.auditFile), { recursive: true });
    await fs.appendFile(state.auditFile, `${line}\n`);
  } catch (error) {
    console.error('[ocean-bedrock] failed to write audit event', error);
  }

  try {
    await state.ledger.append({
      event_type: event.action || 'audit.event',
      correlation_id: event.correlation_id || event.correlationId || null,
      lab: event.lab || 'longhouse',
      actor_type: event.actorId ? 'agent' : 'system',
      actor_id: event.actorId || null,
      actor_name: event.actor || null,
      virtual_path: event.path || event.virtual_path || null,
      payload: auditEvent,
      clearance: event.clearance || 'UNCLASSIFIED',
      timestamp: ts,
    });
  } catch (error) {
    console.error('[ocean-bedrock] failed to append ledger event', error);
  }
}

async function readAudit(state, limit = 100) {
  try {
    const raw = await fs.readFile(state.auditFile, 'utf8');
    return raw.trim().split('\n').filter(Boolean).slice(-limit).map((line) => JSON.parse(line));
  } catch (error) {
    if (error.code === 'ENOENT') return [];
    throw error;
  }
}

function pathsIntersect(a, b) {
  return a === '/' || b === '/' || a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

async function loadLocks(state) {
  await fs.mkdir(path.dirname(state.locksFile), { recursive: true });
  let locks = { version: 1, locks: [] };
  try {
    const raw = await fs.readFile(state.locksFile, 'utf8');
    locks = raw.trim() ? JSON.parse(raw) : locks;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  if (!Array.isArray(locks.locks)) locks.locks = [];
  const now = Date.now();
  const active = locks.locks.filter((lock) => !lock.expiresAt || Date.parse(lock.expiresAt) > now);
  if (active.length !== locks.locks.length) {
    locks.locks = active;
    await saveLocks(state, locks);
  }
  return locks;
}

async function saveLocks(state, locks) {
  await fs.mkdir(path.dirname(state.locksFile), { recursive: true });
  const tmp = `${state.locksFile}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(tmp, `${JSON.stringify(locks, null, 2)}\n`);
  await fs.rename(tmp, state.locksFile);
}

async function lockConflicts(state, apiPath, principal) {
  const locks = await loadLocks(state);
  return locks.locks.filter((lock) => pathsIntersect(lock.path, apiPath) && lock.ownerId !== principal.id);
}

async function requireNoLockConflict(state, apiPath, principal) {
  const conflicts = await lockConflicts(state, apiPath, principal);
  if (conflicts.length) {
    throw new HttpError(423, `Path is locked by another agent: ${conflicts[0].path}`, { conflicts });
  }
}

async function listDirectory(state, apiPath, depth = 1) {
  const { adapter, relPath } = await resolveMount(state, apiPath);
  return adapter.list(relPath, apiPath, { depth });
}

async function statPath(state, apiPath) {
  const { adapter, relPath } = await resolveMount(state, apiPath);
  return adapter.stat(relPath, apiPath);
}

function ifMatchAllowsEtag(req, etag) {
  const ifMatch = req.headers['if-match'];
  if (!ifMatch) return true;
  if (ifMatch.trim() === '*') return true;
  return ifMatch.split(',').map((value) => value.trim()).includes(etag);
}

async function putFile(req, state, principal, apiPath) {
  requirePermission(principal, 'write');
  requirePathScope(principal, apiPath);
  await requireNoLockConflict(state, apiPath, principal);

  const { mount, adapter, relPath } = await resolveMount(state, apiPath);
  if (mount.mode === 'readonly') throw new HttpError(403, `Mount "${mount.id}" is read-only.`);

  const noneMatch = req.headers['if-none-match'];
  let existingInfo = null;
  try { existingInfo = await adapter.stat(relPath, apiPath); } catch (e) { if (!(e instanceof HttpError && e.status === 404)) throw e; }

  if (noneMatch === '*' && existingInfo) throw new HttpError(412, 'Target already exists.');
  if (existingInfo) {
    if (existingInfo.type === 'directory') throw new HttpError(409, 'Cannot overwrite a directory with a file.');
    if (!ifMatchAllowsEtag(req, existingInfo.etag)) throw new HttpError(412, 'If-Match does not match current file etag.');
  }

  const tmpPath = path.join(state.tmpRoot, `${Date.now()}-${crypto.randomBytes(8).toString('hex')}.upload`);
  const bytes = await receiveFile(req, tmpPath, state.maxUploadBytes);
  await adapter.write(relPath, tmpPath);
  const info = await adapter.stat(relPath, apiPath);
  try {
    await recordObjectWrite(state, apiPath, info, principal, mount.id, relPath);
  } catch (error) {
    console.error('[ocean-bedrock] failed to record object metadata', error);
  }
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: existingInfo ? 'file.update' : 'file.create',
    path: apiPath,
    bytes,
  });
  return info;
}

async function deletePath(state, principal, apiPath, recursive = false) {
  requirePermission(principal, 'delete');
  requirePathScope(principal, apiPath);
  if (apiPath === '/') throw new HttpError(400, 'Refusing to delete the filesystem root.');
  await requireNoLockConflict(state, apiPath, principal);

  const { mount, adapter, relPath } = await resolveMount(state, apiPath);
  if (mount.mode === 'readonly') throw new HttpError(403, `Mount "${mount.id}" is read-only.`);

  const info = await adapter.stat(relPath, apiPath);
  if (info.type === 'directory' && !recursive) throw new HttpError(400, 'Directory delete requires recursive=true.');
  await adapter.delete(relPath, { recursive });
  try {
    await recordObjectDelete(apiPath, principal);
  } catch (error) {
    console.error('[ocean-bedrock] failed to mark object deleted', error);
  }
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: info.type === 'directory' ? 'directory.delete' : 'file.delete',
    path: apiPath,
    recursive,
  });
}

async function createDirectory(req, state, principal) {
  const body = await readJsonBody(req, state.maxJsonBytes);
  const apiPath = normalizeApiPath(body.path);
  requirePermission(principal, 'write');
  requirePathScope(principal, apiPath);
  await requireNoLockConflict(state, apiPath, principal);
  const { mount, adapter, relPath } = await resolveMount(state, apiPath);
  if (mount.mode === 'readonly') throw new HttpError(403, `Mount "${mount.id}" is read-only.`);
  const info = await adapter.mkdir(relPath, apiPath);
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: 'directory.create',
    path: apiPath,
  });
  return info;
}

async function movePath(req, state, principal, copy = false) {
  const body = await readJsonBody(req, state.maxJsonBytes);
  const from = normalizeApiPath(body.from);
  const to = normalizeApiPath(body.to);
  if (from === '/') throw new HttpError(400, 'Refusing to move/copy the filesystem root.');
  requirePermission(principal, 'write');
  requirePermission(principal, 'delete');
  requirePathScope(principal, from);
  requirePathScope(principal, to);
  await requireNoLockConflict(state, from, principal);
  await requireNoLockConflict(state, to, principal);

  const { mount: fromMount, adapter: fromAdapter, relPath: fromRel } = await resolveMount(state, from);
  const { mount: toMount, relPath: toRel } = await resolveMount(state, to);
  if (fromMount.id !== toMount.id) throw new HttpError(400, 'Cross-mount move/copy is not yet supported.');
  if (fromMount.mode === 'readonly') throw new HttpError(403, `Mount "${fromMount.id}" is read-only.`);

  if (copy) {
    await fromAdapter.copy(fromRel, toRel, { overwrite: Boolean(body.overwrite) });
  } else {
    await fromAdapter.move(fromRel, toRel, { overwrite: Boolean(body.overwrite) });
  }
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: copy ? 'path.copy' : 'path.move',
    from,
    to,
    overwrite: Boolean(body.overwrite),
  });
  return fromAdapter.stat(toRel, to);
}

async function createLock(req, state, principal) {
  requirePermission(principal, 'lock');
  const body = await readJsonBody(req, state.maxJsonBytes);
  const apiPath = normalizeApiPath(body.path);
  requirePathScope(principal, apiPath);

  const ttlSeconds = Math.max(30, Math.min(Number(body.ttlSeconds || 900), 24 * 60 * 60));
  const locks = await loadLocks(state);
  const conflicts = locks.locks.filter((lock) => pathsIntersect(lock.path, apiPath) && lock.ownerId !== principal.id);
  if (conflicts.length) throw new HttpError(423, 'Path is already locked by another agent.', { conflicts });

  const lock = {
    id: `lock_${crypto.randomBytes(8).toString('hex')}`,
    path: apiPath,
    ownerId: principal.id,
    ownerName: principal.name,
    note: body.note || null,
    createdAt: new Date().toISOString(),
    expiresAt: new Date(Date.now() + ttlSeconds * 1000).toISOString(),
    ttlSeconds,
  };
  locks.locks.push(lock);
  await saveLocks(state, locks);
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: 'lock.create',
    path: apiPath,
    lockId: lock.id,
  });
  return lock;
}

async function releaseLock(state, principal, lockId) {
  requirePermission(principal, 'lock');
  const locks = await loadLocks(state);
  const index = locks.locks.findIndex((lock) => lock.id === lockId);
  if (index === -1) throw new HttpError(404, 'Lock not found.');
  const [lock] = locks.locks.splice(index, 1);
  if (lock.ownerId !== principal.id && principal.role !== 'admin') throw new HttpError(403, 'Only the lock owner or an admin can release this lock.');
  await saveLocks(state, locks);
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: 'lock.release',
    path: lock.path,
    lockId: lock.id,
  });
  return lock;
}

async function search(state, principal, url) {
  requirePermission(principal, 'read');
  const q = (url.searchParams.get('q') || '').trim();
  if (!q) throw new HttpError(400, 'Missing q search parameter.');
  const basePath = normalizeApiPath(url.searchParams.get('path') || '/');
  requirePathScope(principal, basePath);
  const limit = Math.max(1, Math.min(Number(url.searchParams.get('limit') || 50), 200));
  const maxFileBytes = Math.min(state.maxUploadBytes, parseSize(envValue('OCEAN_BEDROCK_SEARCH_MAX_FILE', 'RT_NAS_SEARCH_MAX_FILE', '2mb'), 2 * 1024 * 1024));
  const results = [];
  const lowerQ = q.toLowerCase();

  const { adapter, relPath: baseRel } = await resolveMount(state, basePath);
  if (!adapter.diskRoot) throw new HttpError(501, 'Text search is not yet supported for this mount type.');
  const baseDisk = path.join(adapter.diskRoot, baseRel);

  async function walk(apiPath, diskPath) {
    if (results.length >= limit) return;
    const stat = await fs.lstat(diskPath);
    if (stat.isSymbolicLink()) return;
    if (stat.isDirectory()) {
      const dirents = await fs.readdir(diskPath, { withFileTypes: true });
      for (const dirent of dirents) {
        await walk(
          apiPath === '/' ? `/${dirent.name}` : `${apiPath}/${dirent.name}`,
          path.join(diskPath, dirent.name),
        );
        if (results.length >= limit) return;
      }
      return;
    }
    if (!stat.isFile() || stat.size > maxFileBytes) return;
    try {
      const text = await fs.readFile(diskPath, 'utf8');
      const lines = text.split(/\r?\n/);
      for (let i = 0; i < lines.length; i += 1) {
        if (lines[i].toLowerCase().includes(lowerQ)) {
          results.push({ path: apiPath, line: i + 1, preview: lines[i].slice(0, 500) });
          if (results.length >= limit) return;
        }
      }
    } catch {
      // Ignore binary or unreadable files.
    }
  }

  await walk(basePath, baseDisk);
  return { q, path: basePath, limit, results };
}

function sourceDatabase() {
  const db = getPool();
  if (!db) throw new HttpError(503, 'Source registry requires DATABASE_URL on the Ocean Bedrock server.');
  return db;
}

function mountPublicView(row) {
  return {
    id: row.id,
    type: row.type,
    prefix: row.prefix,
    mode: row.mode,
    priority: row.priority,
    capabilities: row.capabilities,
    enabled: row.enabled,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

const SECRET_CONFIG_KEYS = new Set(['key', 'secret', 'password', 'token', 'credential', 'apikey', 'api_key', 'accesskey', 'access_key', 'privatekey', 'private_key']);

function mountPublicViewWithConfig(row) {
  const redacted = {};
  for (const [k, v] of Object.entries(row.config || {})) {
    redacted[k] = SECRET_CONFIG_KEYS.has(k.toLowerCase()) ? '[redacted]' : v;
  }
  return { ...mountPublicView(row), config: redacted };
}

function validateMountConfig(type, config, stateRoot) {
  for (const k of Object.keys(config)) {
    if (SECRET_CONFIG_KEYS.has(k.toLowerCase())) {
      throw new HttpError(400, `Config key "${k}" looks like a raw secret. Use a secretRef handle instead.`);
    }
  }
  if (type === 'local') {
    if (!config.root) throw new HttpError(400, 'Local mount requires config.root.');
    if (!path.isAbsolute(config.root)) throw new HttpError(400, 'Local mount config.root must be an absolute path.');
    if (stateRoot && config.root !== stateRoot && !config.root.startsWith(`${stateRoot}/`)) {
      throw new HttpError(400, `Local mount config.root must be within the managed files root (${stateRoot}).`);
    }
  }
}

function camelOrSnake(body, snake, camel, fallback = undefined) {
  if (body && body[snake] !== undefined) return body[snake];
  if (body && body[camel] !== undefined) return body[camel];
  return fallback;
}

function validatedSecretRef(value) {
  try {
    return validateSecretRef(value);
  } catch (error) {
    throw new HttpError(400, error.message);
  }
}

function filterVisibleSourceRows(rows, principal, prefixKey = 'remote_prefix', clearanceKey = 'clearance') {
  return rows.filter((row) => {
    const prefix = row[prefixKey] || row.source_instance_remote_prefix || row.remotePrefix || '/';
    const normalized = normalizeApiPath(prefix);
    return pathInScopes(normalized, principal.scopes || ['/']) && clearanceAllowed(principal, row[clearanceKey] || row.source_clearance || 'CONFIDENTIAL');
  });
}

function requireSourceRowVisible(row, principal, prefixKey = 'remote_prefix', clearanceKey = 'clearance') {
  if (!row) throw new HttpError(404, 'Source resource not found.');
  const prefix = normalizeApiPath(row[prefixKey] || row.source_instance_remote_prefix || row.remotePrefix || '/');
  requirePathScope(principal, prefix);
  requireClearance(principal, row[clearanceKey] || row.source_clearance || 'CONFIDENTIAL');
}

function normalizeOptionalPath(value) {
  if (value === undefined || value === null || value === '') return null;
  return normalizeApiPath(value);
}

function normalizeSourceStats(input = {}) {
  return {
    scanned_count: Number(input.scanned_count ?? input.scannedCount ?? input.scanned ?? 0),
    changed_count: Number(input.changed_count ?? input.changedCount ?? input.changed ?? 0),
    uploaded_count: Number(input.uploaded_count ?? input.uploadedCount ?? input.uploaded ?? 0),
    skipped_count: Number(input.skipped_count ?? input.skippedCount ?? input.skipped ?? 0),
    error_count: Number(input.error_count ?? input.errorCount ?? input.errors ?? 0),
    error: input.error || null,
    manifest_path: input.manifest_path || input.manifestPath || null,
    metadata: input.metadata || {},
    correlationId: input.correlationId || input.correlation_id || null,
  };
}

function sourceInstancePayload(body, principal) {
  const remotePrefix = normalizeApiPath(camelOrSnake(body, 'remote_prefix', 'remotePrefix'));
  return {
    adapter_id: camelOrSnake(body, 'adapter_id', 'adapterId', 'local_folder'),
    name: String(body.name || '').trim(),
    owner_name: camelOrSnake(body, 'owner_name', 'ownerName', principal.name),
    owner_token_id: principal.role === 'admin'
      ? camelOrSnake(body, 'owner_token_id', 'ownerTokenId', principal.id)
      : principal.id,
    remote_prefix: remotePrefix,
    config: body.config || {},
    secret_ref: validatedSecretRef(camelOrSnake(body, 'secret_ref', 'secretRef', null)),
    clearance: body.clearance || 'CONFIDENTIAL',
    correlationId: body.correlationId || body.correlation_id || null,
  };
}

function sourceStreamPayload(body) {
  return {
    source_instance_id: camelOrSnake(body, 'source_instance_id', 'sourceInstanceId'),
    stream_key: camelOrSnake(body, 'stream_key', 'streamKey'),
    stream_type: camelOrSnake(body, 'stream_type', 'streamType', 'folder'),
    remote_prefix: normalizeApiPath(camelOrSnake(body, 'remote_prefix', 'remotePrefix')),
    selection: body.selection || {},
    cursor: body.cursor || {},
    correlationId: body.correlationId || body.correlation_id || null,
  };
}

async function findSourceObjectId(db, virtualPath) {
  try {
    return await findObjectIdByVirtualPath(db, virtualPath);
  } catch {
    return null;
  }
}

async function handleSourceAdapters(req, res, state, principal, url) {
  requirePermission(principal, 'read');
  const db = sourceDatabase();
  const enabled = url.searchParams.has('enabled') ? url.searchParams.get('enabled') === 'true' : undefined;
  const runnerAdapters = new Set(listRunnerAdapters());
  const adapters = (await listSourceAdapters(db, { enabled })).map((adapter) => ({
    ...adapter,
    runner_implemented: runnerAdapters.has(adapter.id),
    sync_implemented: runnerAdapters.has(adapter.id) || adapter.id === 'local_folder',
    sync_mode: runnerAdapters.has(adapter.id) ? 'server_runner' : (adapter.id === 'local_folder' ? 'local_courier' : 'not_implemented'),
  }));
  jsonResponse(res, 200, { adapters });
}

async function handleSourceInstances(req, res, state, principal, url) {
  const db = sourceDatabase();

  if (req.method === 'GET') {
    requirePermission(principal, 'read');
    const rows = await listSourceInstances(db, {
      adapter_id: url.searchParams.get('adapter_id') || url.searchParams.get('adapterId'),
      owner_name: url.searchParams.get('owner_name') || url.searchParams.get('ownerName'),
      status: url.searchParams.get('status'),
      enabled: url.searchParams.has('enabled') ? url.searchParams.get('enabled') === 'true' : undefined,
      limit: url.searchParams.get('limit') || 200,
    });
    jsonResponse(res, 200, { sources: filterVisibleSourceRows(rows, principal) });
    return;
  }

  if (req.method === 'POST') {
    requirePermission(principal, 'write');
    const body = await readJsonBody(req, state.maxJsonBytes);
    const payload = sourceInstancePayload(body, principal);
    if (!payload.name) throw new HttpError(400, 'name is required.');
    requirePathScope(principal, payload.remote_prefix);
    requireClearance(principal, payload.clearance);
    const created = await upsertSourceInstance(db, payload);
    const row = await getSourceInstance(db, created.id);
    jsonResponse(res, 201, { source: row });
    return;
  }

  notFound();
}

async function handleSourceInstanceById(req, res, state, principal, id) {
  const db = sourceDatabase();
  const current = await getSourceInstance(db, id);
  requireSourceRowVisible(current, principal);

  if (req.method === 'GET') {
    requirePermission(principal, 'read');
    const streams = filterVisibleSourceRows(await listSourceStreams(db, { source_instance_id: id }), principal);
    const syncRuns = filterVisibleSourceRows(await listSourceSyncRuns(db, { source_instance_id: id, limit: 25 }), principal);
    jsonResponse(res, 200, { source: current, streams, syncRuns });
    return;
  }

  if (req.method === 'PATCH') {
    requirePermission(principal, 'write');
    const body = await readJsonBody(req, state.maxJsonBytes);
    if (principal.role !== 'admin') {
      delete body.owner_token_id;
      delete body.ownerTokenId;
    }
    const nextPrefix = normalizeOptionalPath(camelOrSnake(body, 'remote_prefix', 'remotePrefix'));
    if (nextPrefix) {
      body.remote_prefix = nextPrefix;
      delete body.remotePrefix;
      requirePathScope(principal, nextPrefix);
    }
    if (body.secret_ref !== undefined || body.secretRef !== undefined) {
      body.secret_ref = validatedSecretRef(camelOrSnake(body, 'secret_ref', 'secretRef', null));
      delete body.secretRef;
    }
    if (body.clearance) requireClearance(principal, body.clearance);
    const updated = await updateSourceInstance(db, id, body);
    jsonResponse(res, 200, { source: updated });
    return;
  }

  notFound();
}

async function handleSourceInstanceSync(req, res, state, principal, id, url) {
  requirePermission(principal, 'write');
  const db = sourceDatabase();
  const source = await getSourceInstance(db, id);
  requireSourceRowVisible(source, principal);
  if (!source.enabled || source.status !== 'active') throw new HttpError(409, 'Source instance is not active/enabled.');

  const body = await readJsonBody(req, state.maxJsonBytes);
  const streamId = camelOrSnake(body, 'stream_id', 'streamId', url.searchParams.get('stream_id') || url.searchParams.get('streamId'));
  const mode = body.mode || url.searchParams.get('mode') || 'incremental';
  const maxRecords = body.maxRecords || body.max_records || url.searchParams.get('maxRecords') || url.searchParams.get('max_records') || null;
  const correlationId = body.correlationId || body.correlation_id || null;

  let runner;
  try {
    runner = getRunner(source.adapter_id);
  } catch (error) {
    if (error.code === 'NO_SOURCE_RUNNER') throw new HttpError(501, error.message);
    throw error;
  }

  let streams;
  if (streamId) {
    const stream = await getSourceStream(db, streamId);
    if (!stream || stream.source_instance_id !== source.id) throw new HttpError(400, 'stream_id must belong to source_instance_id.');
    requireSourceRowVisible(stream, principal);
    if (!stream.enabled) throw new HttpError(409, 'Source stream is disabled.');
    streams = [stream];
  } else {
    streams = filterVisibleSourceRows(await listSourceStreams(db, { source_instance_id: source.id, enabled: true, limit: 100 }), principal);
    if (!streams.length) streams = [null];
  }

  const syncRuns = [];
  for (const stream of streams) {
    if (stream) requireSourceRowVisible(stream, principal);
    const result = await runSourceSync(db, state, {
      instance: source,
      stream,
      runner,
      principal,
      mode,
      maxRecords,
      correlationId,
    });
    syncRuns.push(result);
  }

  jsonResponse(res, 201, { source_instance_id: source.id, adapter_id: source.adapter_id, syncRuns });
}

async function handleSourceStreams(req, res, state, principal, url) {
  const db = sourceDatabase();

  if (req.method === 'GET') {
    requirePermission(principal, 'read');
    const rows = await listSourceStreams(db, {
      source_instance_id: url.searchParams.get('source_instance_id') || url.searchParams.get('sourceInstanceId'),
      stream_type: url.searchParams.get('stream_type') || url.searchParams.get('streamType'),
      enabled: url.searchParams.has('enabled') ? url.searchParams.get('enabled') === 'true' : undefined,
      limit: url.searchParams.get('limit') || 200,
    });
    jsonResponse(res, 200, { streams: filterVisibleSourceRows(rows, principal) });
    return;
  }

  if (req.method === 'POST') {
    requirePermission(principal, 'write');
    const body = await readJsonBody(req, state.maxJsonBytes);
    const payload = sourceStreamPayload(body);
    if (!payload.source_instance_id) throw new HttpError(400, 'source_instance_id is required.');
    if (!payload.stream_key) throw new HttpError(400, 'stream_key is required.');
    const source = await getSourceInstance(db, payload.source_instance_id);
    requireSourceRowVisible(source, principal);
    requirePathScope(principal, payload.remote_prefix);
    const created = await upsertSourceStream(db, payload);
    const stream = await getSourceStream(db, created.id);
    jsonResponse(res, 201, { stream });
    return;
  }

  notFound();
}

async function handleSourceStreamById(req, res, state, principal, id) {
  const db = sourceDatabase();
  const current = await getSourceStream(db, id);
  requireSourceRowVisible(current, principal);

  if (req.method === 'PATCH') {
    requirePermission(principal, 'write');
    const body = await readJsonBody(req, state.maxJsonBytes);
    const nextPrefix = normalizeOptionalPath(camelOrSnake(body, 'remote_prefix', 'remotePrefix'));
    if (nextPrefix) {
      body.remote_prefix = nextPrefix;
      delete body.remotePrefix;
      requirePathScope(principal, nextPrefix);
    }
    const updated = await updateSourceStream(db, id, body);
    jsonResponse(res, 200, { stream: updated });
    return;
  }

  notFound();
}

async function handleSyncRuns(req, res, state, principal, url) {
  const db = sourceDatabase();

  if (req.method === 'GET') {
    requirePermission(principal, 'read');
    const rows = await listSourceSyncRuns(db, {
      source_instance_id: url.searchParams.get('source_instance_id') || url.searchParams.get('sourceInstanceId'),
      stream_id: url.searchParams.get('stream_id') || url.searchParams.get('streamId'),
      status: url.searchParams.get('status'),
      limit: url.searchParams.get('limit') || 100,
    });
    jsonResponse(res, 200, { syncRuns: filterVisibleSourceRows(rows, principal, 'source_instance_remote_prefix', 'source_clearance') });
    return;
  }

  if (req.method === 'POST') {
    requirePermission(principal, 'write');
    const body = await readJsonBody(req, state.maxJsonBytes);
    const sourceInstanceId = camelOrSnake(body, 'source_instance_id', 'sourceInstanceId');
    if (!sourceInstanceId) throw new HttpError(400, 'source_instance_id is required.');
    const source = await getSourceInstance(db, sourceInstanceId);
    requireSourceRowVisible(source, principal);
    const streamId = camelOrSnake(body, 'stream_id', 'streamId', null);
    if (streamId) {
      const stream = await getSourceStream(db, streamId);
      if (!stream || stream.source_instance_id !== sourceInstanceId) throw new HttpError(400, 'stream_id must belong to source_instance_id.');
      requireSourceRowVisible(stream, principal);
    }
    const run = await createSourceSyncRun(db, {
      source_instance_id: sourceInstanceId,
      stream_id: streamId,
      metadata: body.metadata || {},
      correlationId: body.correlationId || body.correlation_id || null,
    });
    jsonResponse(res, 201, { syncRun: await getSourceSyncRun(db, run.id) });
    return;
  }

  notFound();
}

async function handleSyncRunById(req, res, state, principal, id) {
  const db = sourceDatabase();
  const run = await getSourceSyncRun(db, id);
  requireSourceRowVisible(run, principal, 'source_instance_remote_prefix', 'source_clearance');
  requirePermission(principal, 'read');
  jsonResponse(res, 200, { syncRun: run });
}

async function handleCompleteSyncRun(req, res, state, principal, id, failed = false) {
  requirePermission(principal, 'write');
  const db = sourceDatabase();
  const run = await getSourceSyncRun(db, id);
  requireSourceRowVisible(run, principal, 'source_instance_remote_prefix', 'source_clearance');
  const body = await readJsonBody(req, state.maxJsonBytes);
  const stats = normalizeSourceStats(body.stats || body);
  if (stats.manifest_path) requirePathScope(principal, normalizeApiPath(stats.manifest_path));
  const updated = failed
    ? await failSourceSyncRun(db, id, stats)
    : await completeSourceSyncRun(db, id, stats);
  jsonResponse(res, 200, { syncRun: await getSourceSyncRun(db, updated.id) });
}

async function handleLocalFolderPlan(req, res, state, principal) {
  requirePermission(principal, 'write');
  const db = sourceDatabase();
  const body = await readJsonBody(req, state.maxJsonBytes);
  const ownerName = String(body.ownerName || body.owner_name || principal.name || 'coworker').trim();
  const deviceName = String(body.deviceName || body.device_name || 'device').trim();
  const ownerSlug = String(body.ownerSlug || body.owner_slug || ownerName).trim().toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'coworker';
  const deviceSlug = String(body.deviceSlug || body.device_slug || deviceName).trim().toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'device';
  const remoteRoot = normalizeApiPath(body.remoteRoot || body.remote_root || `/coworkers/${ownerSlug}/${deviceSlug}`);
  requirePathScope(principal, remoteRoot);
  const clearance = body.clearance || 'CONFIDENTIAL';
  requireClearance(principal, clearance);
  const correlationId = body.correlationId || body.correlation_id || `cor-ingest-${ownerSlug}-${deviceSlug}-${Date.now()}`;

  const instanceResult = await upsertSourceInstance(db, {
    adapter_id: 'local_folder',
    name: body.name || `${ownerSlug}-${deviceSlug}`,
    owner_name: ownerName,
    owner_token_id: principal.id,
    remote_prefix: remoteRoot,
    config: {
      device: deviceName,
      device_slug: deviceSlug,
      feed_mode: body.feedMode || body.feed_mode || null,
      allowed_extensions: body.allowedExtensions ?? body.allowed_extensions ?? null,
      ignore: body.defaultIgnores || body.default_ignores || [],
      max_file_bytes: body.maxFileBytes || body.max_file_bytes || null,
      source_count: Array.isArray(body.sources) ? body.sources.length : 0,
      client: 'ocean-local-app',
    },
    clearance,
    correlationId,
  });
  const sourceInstance = await getSourceInstance(db, instanceResult.id);

  const streams = [];
  for (const source of Array.isArray(body.sources) ? body.sources : []) {
    if (source.enabled === false) continue;
    const remotePrefix = normalizeApiPath(source.remotePrefix || source.remote_prefix || `${remoteRoot}/${source.label || source.id || 'folder'}`);
    requirePathScope(principal, remotePrefix);
    const created = await upsertSourceStream(db, {
      source_instance_id: sourceInstance.id,
      stream_key: source.id || source.stream_key || source.label || remotePrefix,
      stream_type: 'folder',
      remote_prefix: remotePrefix,
      selection: {
        label: source.label || null,
        local_path_label: source.localPath ? path.basename(source.localPath) : null,
        local_path_hash: source.localPath ? crypto.createHash('sha256').update(String(source.localPath)).digest('hex') : null,
        enabled: source.enabled !== false,
        recursive: true,
      },
      cursor: {},
      correlationId,
    });
    streams.push(await getSourceStream(db, created.id));
  }

  const run = await createSourceSyncRun(db, {
    source_instance_id: sourceInstance.id,
    metadata: {
      plan: 'local_folder',
      ownerName,
      deviceName,
      source_count: streams.length,
    },
    correlationId,
  });

  jsonResponse(res, 201, { correlationId, sourceInstance, streams, syncRun: await getSourceSyncRun(db, run.id) });
}

async function handleLocalFolderRecordsBatch(req, res, state, principal) {
  requirePermission(principal, 'write');
  const db = sourceDatabase();
  const body = await readJsonBody(req, state.maxJsonBytes);
  const sourceInstanceId = camelOrSnake(body, 'source_instance_id', 'sourceInstanceId');
  if (!sourceInstanceId) throw new HttpError(400, 'source_instance_id is required.');
  const source = await getSourceInstance(db, sourceInstanceId);
  requireSourceRowVisible(source, principal);
  const records = Array.isArray(body.records) ? body.records : [];
  if (!records.length) throw new HttpError(400, 'records must be a non-empty array.');
  if (records.length > 500) throw new HttpError(400, 'records batch is limited to 500 records.');

  let upserted = 0;
  const errors = [];
  const streamCache = new Map();
  for (const record of records) {
    try {
      const virtualPath = normalizeOptionalPath(record.virtual_path || record.virtualPath);
      if (virtualPath) requirePathScope(principal, virtualPath);
      const streamId = record.stream_id || record.streamId || null;
      if (streamId) {
        if (!streamCache.has(streamId)) streamCache.set(streamId, await getSourceStream(db, streamId));
        const stream = streamCache.get(streamId);
        if (!stream || stream.source_instance_id !== sourceInstanceId) throw new HttpError(400, 'record stream_id must belong to source_instance_id.');
        requireSourceRowVisible(stream, principal);
      }
      const objectId = record.object_id || record.objectId || (virtualPath ? await findSourceObjectId(db, virtualPath) : null);
      await upsertSourceRecord(db, {
        source_instance_id: sourceInstanceId,
        stream_id: streamId,
        source_record_id: record.source_record_id || record.sourceRecordId,
        virtual_path: virtualPath,
        object_id: objectId,
        source_updated_at: record.source_updated_at || record.sourceUpdatedAt || null,
        content_sha256: record.content_sha256 || record.contentSha256 || null,
        metadata: record.metadata || {},
        correlationId: body.correlationId || body.correlation_id || record.correlationId || record.correlation_id || null,
      });
      upserted += 1;
    } catch (error) {
      errors.push({ source_record_id: record.source_record_id || record.sourceRecordId || null, error: error.message });
    }
  }

  jsonResponse(res, errors.length ? 207 : 200, { ok: errors.length === 0, upserted, errors });
}

async function handleLocalFolderCommit(req, res, state, principal) {
  requirePermission(principal, 'write');
  const db = sourceDatabase();
  const body = await readJsonBody(req, state.maxJsonBytes);
  const syncRunId = body.sync_run_id || body.syncRunId;
  if (!syncRunId) throw new HttpError(400, 'sync_run_id is required.');
  const run = await getSourceSyncRun(db, syncRunId);
  requireSourceRowVisible(run, principal, 'source_instance_remote_prefix', 'source_clearance');
  const stats = normalizeSourceStats(body.stats || body);
  if (stats.manifest_path) requirePathScope(principal, normalizeApiPath(stats.manifest_path));
  const updated = body.status === 'failed' || stats.error
    ? await failSourceSyncRun(db, syncRunId, stats)
    : await completeSourceSyncRun(db, syncRunId, stats);
  jsonResponse(res, 200, { syncRun: await getSourceSyncRun(db, updated.id) });
}

async function handleSemanticSearch(req, res, state, principal, url) {
  requirePermission(principal, 'read');
  const query = url.searchParams.get('q') || url.searchParams.get('query');
  if (!query) throw new HttpError(400, 'Missing q query parameter.');
  const apiPath = normalizeApiPath(url.searchParams.get('path') || '/');
  requirePathScope(principal, apiPath);
  const db = sourceDatabase();
  const limit = Math.max(1, Math.min(Number(url.searchParams.get('limit') || 10), 50));
  const result = await semanticSearch(db, query, { path: apiPath, limit, mode: url.searchParams.get('mode') || undefined });
  const matches = result.matches.filter((match) => {
    if (!match.virtual_path) return false;
    try {
      return pathInScopes(normalizeApiPath(match.virtual_path), principal.scopes || ['/']);
    } catch {
      return false;
    }
  });
  jsonResponse(res, 200, { ...result, path: apiPath, query, matches });
}

async function handleGraphNodes(req, res, state, principal, url) {
  requirePermission(principal, 'read');
  const db = sourceDatabase();
  const apiPath = normalizeApiPath(url.searchParams.get('path') || '/');
  requirePathScope(principal, apiPath);
  const rows = await listGraphNodes(db, {
    path: apiPath,
    type: url.searchParams.get('type') || undefined,
    q: url.searchParams.get('q') || undefined,
    limit: url.searchParams.get('limit') || 100,
  });
  const nodes = rows.filter((node) => node.virtual_path && pathInScopes(normalizeApiPath(node.virtual_path), principal.scopes || ['/']));
  jsonResponse(res, 200, { path: apiPath, nodes });
}

async function handleGraphNeighborhood(req, res, state, principal, url) {
  requirePermission(principal, 'read');
  const db = sourceDatabase();
  const apiPath = url.searchParams.has('path') ? normalizeApiPath(url.searchParams.get('path')) : null;
  if (apiPath) requirePathScope(principal, apiPath);
  const result = await graphNeighborhood(db, {
    path: apiPath,
    node_id: url.searchParams.get('node_id') || url.searchParams.get('nodeId') || undefined,
    depth: url.searchParams.get('depth') || 1,
    limit: url.searchParams.get('limit') || 100,
  });
  if (result.start?.virtual_path) requirePathScope(principal, normalizeApiPath(result.start.virtual_path));
  const visibleNodeIds = new Set(result.nodes.filter((node) => !node.virtual_path || pathInScopes(normalizeApiPath(node.virtual_path), principal.scopes || ['/'])).map((node) => node.id));
  jsonResponse(res, 200, {
    ...result,
    nodes: result.nodes.filter((node) => visibleNodeIds.has(node.id)),
    edges: result.edges.filter((edge) => visibleNodeIds.has(edge.from_node_id) && visibleNodeIds.has(edge.to_node_id)),
  });
}

async function handleOceanContextTriage(req, res, state, principal) {
  requirePermission(principal, 'write');
  requirePathScope(principal, '/context/ocean-bedrock');
  const body = req.method === 'POST' ? await readJsonBody(req, state.maxJsonBytes) : {};
  requireClearance(principal, body.clearance || 'CONFIDENTIAL');
  const result = await runOceanContextTriage(state, principal, {
    reportPath: body.reportPath || body.report_path || undefined,
    correlationId: body.correlationId || body.correlation_id || undefined,
    ledgerLimit: body.ledgerLimit || body.ledger_limit || 25,
    clearance: body.clearance || 'CONFIDENTIAL',
    force: body.force === true,
  });
  jsonResponse(res, 201, result);
}

async function seedDefaultFolders(state) {
  const folders = ['docs', 'context', 'sessions', 'handoffs', 'shared', 'vault'];
  for (const folder of folders) {
    await fs.mkdir(path.join(state.filesRoot, folder), { recursive: true });
  }

  async function writeIfAbsent(filePath, content) {
    try { await fs.access(filePath); } catch { await fs.writeFile(filePath, content); }
  }

  await writeIfAbsent(path.join(state.filesRoot, 'docs', 'README.md'), '# Company docs\n\nShared, durable documentation for agents and humans.\n');
  await writeIfAbsent(path.join(state.filesRoot, 'context', 'README.md'), '# Shared context\n\nUse this folder for reusable agent context, handoff notes, research summaries, and project memory.\n');
  await writeIfAbsent(path.join(state.filesRoot, 'sessions', 'README.md'), '# Sessions\n\nCreate one folder per durable agent or operator session.\n');
  await writeIfAbsent(path.join(state.filesRoot, 'vault', 'README.md'), '# Vault\n\nDo not store high-value raw secrets here unless this service is deployed behind appropriate controls. Prefer scoped tokens, encrypted blobs, or an external secrets manager.\n');
}

function workerEnabled() {
  return /^(1|true|yes)$/i.test(process.env.OCEAN_BEDROCK_WORKER_ENABLED || '');
}

function startIngestWorker(state) {
  if (!workerEnabled() || state.ingestWorkerStarted) return;
  state.ingestWorkerStarted = true;
  const workerId = `ocean-bedrock-server-${process.pid}`;
  const sleepMs = Math.max(1000, Number(process.env.OCEAN_BEDROCK_WORKER_SLEEP_MS || 5000));

  async function tick() {
    try {
      const job = await claimNextIngestJob(workerId);
      if (job) {
        try {
          const result = await processIngestJob(state, job);
          await completeIngestJob(job.id, 'done', null);
          console.log('[ocean-bedrock] indexed job', JSON.stringify({ jobId: job.id, path: job.virtual_path, result }));
        } catch (error) {
          const terminal = Number(job.attempts || 0) >= Number(job.max_attempts || 5);
          await completeIngestJob(job.id, terminal ? 'failed' : 'queued', error.message);
          console.error('[ocean-bedrock] ingest job failed', JSON.stringify({ jobId: job.id, path: job.virtual_path, terminal, error: error.message }));
        }
      }
    } catch (error) {
      console.error('[ocean-bedrock] ingest worker tick failed', error);
    } finally {
      setTimeout(tick, sleepMs).unref?.();
    }
  }

  setTimeout(tick, 1000).unref?.();
  console.log('[ocean-bedrock] ingest worker enabled');
}

async function bootstrapAuth(state) {
  const auth = await state.auth.load();
  const bootstrapToken = envValue('OCEAN_BEDROCK_BOOTSTRAP_TOKEN', 'RT_NAS_BOOTSTRAP_TOKEN');
  if (auth.tokens.length === 0 && bootstrapToken) {
    await state.auth.addToken({
      token: bootstrapToken,
      name: envValue('OCEAN_BEDROCK_BOOTSTRAP_NAME', 'RT_NAS_BOOTSTRAP_NAME', 'bootstrap-admin'),
      role: 'admin',
      scopes: ['/'],
      createdBy: process.env.OCEAN_BEDROCK_BOOTSTRAP_TOKEN ? 'OCEAN_BEDROCK_BOOTSTRAP_TOKEN' : 'RT_NAS_BOOTSTRAP_TOKEN',
    });
    console.warn(`[ocean-bedrock] bootstrapped admin token from bootstrap env (${state.auth.kind} auth store)`);
  }
}

async function handleTokenCreate(req, state, principal) {
  requirePermission(principal, 'admin');
  const body = await readJsonBody(req, state.maxJsonBytes);
  const ttlDays = body.ttlDays ? Number(body.ttlDays) : null;
  const expiresAt = ttlDays ? new Date(Date.now() + ttlDays * 24 * 60 * 60 * 1000).toISOString() : body.expiresAt || null;
  const created = await state.auth.addToken({
    name: body.name,
    role: body.role || 'contributor',
    scopes: body.scopes || ['/'],
    expiresAt,
    createdBy: principal.id,
  });
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: 'token.create',
    tokenId: created.record.id,
    tokenName: created.record.name,
    role: created.record.role,
    scopes: created.record.scopes,
  });
  return created;
}

async function handleInviteCreate(req, state, principal) {
  requirePermission(principal, 'admin');
  const body = await readJsonBody(req, state.maxJsonBytes);
  const created = await state.auth.createInvite({
    name: body.name,
    role: body.role || 'contributor',
    scopes: body.scopes || ['/'],
    ttlMinutes: body.ttlMinutes || 60,
    createdBy: principal.id,
  });
  await appendAudit(state, {
    actor: principal.name,
    actorId: principal.id,
    action: 'invite.create',
    inviteId: created.invite.id,
    inviteName: created.invite.name,
    role: created.invite.role,
    scopes: created.invite.scopes,
    expiresAt: created.invite.expiresAt,
  });
  return created;
}

async function handleInviteRedeem(req, state) {
  const body = await readJsonBody(req, state.maxJsonBytes);
  if (!body.code) throw new HttpError(400, 'Missing invite code.');
  const redeemed = await state.auth.redeemInvite(String(body.code).trim());
  if (!redeemed) throw new HttpError(403, 'Invalid, expired, or already redeemed invite.');
  await appendAudit(state, {
    actor: redeemed.record.name,
    actorId: redeemed.record.id,
    action: 'invite.redeem',
    inviteId: redeemed.invite.id,
    tokenId: redeemed.record.id,
    tokenName: redeemed.record.name,
    role: redeemed.record.role,
    scopes: redeemed.record.scopes,
  });
  return redeemed;
}

// ── Gate-2 S1A: federated rooms (frozen by gate2-s1a-freeze.md v4) ─────

function requireFederation(state) {
  if (!state.federationAvailable) {
    throw new HttpError(503, 'Federation is unavailable: auth, ledger, and rooms must share one Postgres database.');
  }
}

function roomsErrorToHttp(error) {
  if (error instanceof RoomsError) {
    return new HttpError(error.status, error.message, error.code ? { code: error.code } : undefined);
  }
  return error;
}

async function requireActiveRoomMembership(state, principal, roomId) {
  const memberships = await state.rooms.activeMembership(roomId, principal.id);
  if (!memberships.length) {
    if (await state.rooms.wasRemoved(roomId, principal.id)) {
      throw new HttpError(403, 'membership_revoked');
    }
    throw new HttpError(403, 'No active membership in this room.');
  }
  return memberships;
}

// R-4 provenance: the calling token must be the one minted by a CONSUMED
// invite (token.created_by → invite.consumed_by_token_id === principal.id).
async function requireConsumedInviteProvenance(state, principal) {
  const result = await state.auth.pool.query(
    `SELECT 1 FROM longhouse.auth_tokens t
     JOIN longhouse.auth_invites i ON i.id = t.created_by
     WHERE t.id = $1 AND i.consumed_by_token_id = t.id`,
    [principal.id],
  );
  if (!result.rows.length) {
    throw new HttpError(403, 'Self-join requires the token minted by a consumed room invite.');
  }
}

async function handleRoomRegister(req, state, principal, roomIdRaw) {
  requireFederation(state);
  const roomId = normalizeRoomId(roomIdRaw);
  requirePermission(principal, 'admin');
  requirePathScope(principal, roomScope(roomId));
  const body = await readJsonBody(req, state.maxJsonBytes);
  const allowedKeys = Object.keys(body).filter((key) => key !== 'title');
  if (allowedKeys.length) throw new HttpError(400, 'Registration accepts an optional title only.');
  const title = body.title !== undefined ? String(body.title).slice(0, 256) : null;
  const result = await state.rooms.register({ roomId, principal, title }).catch((error) => { throw roomsErrorToHttp(error); });
  await appendAudit(state, {
    actor: principal.name, actorId: principal.id, action: 'room.register', roomId,
  });
  return result;
}

async function handleRoomSelfJoin(state, principal, roomIdRaw) {
  requireFederation(state);
  const roomId = normalizeRoomId(roomIdRaw);
  if (principal.role !== 'contributor') {
    throw new HttpError(403, 'Self-join requires the room-scoped contributor token from a consumed invite.');
  }
  const scopes = principal.scopes || [];
  if (scopes.length !== 1 || scopes[0] !== roomScope(roomId)) {
    throw new HttpError(403, `Self-join token must be scoped exactly to ${roomScope(roomId)}.`);
  }
  await requireConsumedInviteProvenance(state, principal);
  const result = await state.rooms.selfJoin({ roomId, principal }).catch((error) => { throw roomsErrorToHttp(error); });
  await appendAudit(state, {
    actor: principal.name, actorId: principal.id, action: 'room.self_join', roomId,
  });
  return result;
}

async function handleRoomAgentsRegister(req, state, principal, roomIdRaw) {
  requireFederation(state);
  const roomId = normalizeRoomId(roomIdRaw);
  requirePermission(principal, 'write');
  requirePathScope(principal, roomScope(roomId));
  const body = await readJsonBody(req, state.maxJsonBytes);
  if (!Array.isArray(body.agents) || body.agents.length === 0) {
    throw new HttpError(400, 'agents must be a non-empty array.');
  }
  const seenKeys = new Set();
  const agents = body.agents.map((entry) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) {
      throw new HttpError(400, 'Each agents entry must be an object.');
    }
    const { registration_key: rawKey, ...descriptorFields } = entry;
    let registrationKey;
    let descriptor;
    try {
      registrationKey = validateRegistrationKey(rawKey);
      descriptor = validateAgentDescriptor(descriptorFields);
    } catch (error) {
      throw roomsErrorToHttp(error);
    }
    if (seenKeys.has(registrationKey)) {
      throw new HttpError(400, 'Duplicate registration_key within one batch.');
    }
    seenKeys.add(registrationKey);
    return { registration_key: registrationKey, descriptor };
  });
  const result = await state.rooms.registerAgents({ roomId, principal, agents }).catch((error) => { throw roomsErrorToHttp(error); });
  await appendAudit(state, {
    actor: principal.name, actorId: principal.id, action: 'room.agents_register', roomId,
    count: agents.length,
  });
  return result;
}

async function handleRoomRoster(state, principal, roomIdRaw) {
  requireFederation(state);
  const roomId = normalizeRoomId(roomIdRaw);
  requirePermission(principal, 'read');
  requirePathScope(principal, roomScope(roomId));
  await requireActiveRoomMembership(state, principal, roomId);
  return { members: await state.rooms.roster(roomId) };
}

async function handleRoomMemberRemove(state, principal, roomIdRaw, memberIdRaw) {
  requireFederation(state);
  const roomId = normalizeRoomId(roomIdRaw);
  const memberId = String(memberIdRaw || '').trim();
  requirePathScope(principal, roomScope(roomId));
  const memberships = await requireActiveRoomMembership(state, principal, roomId);
  const humanRow = memberships.find((row) => row.actor_type === 'user');
  if (!humanRow) throw new HttpError(403, 'No active human membership in this room.');
  const isOwner = humanRow.role_in_room === 'owner';
  if (isOwner && memberId === humanRow.member_id) {
    throw new HttpError(409, 'Owner self-leave is not allowed in Gate 2; rooms cannot be orphaned.');
  }
  if (!isOwner && memberId !== humanRow.member_id) {
    throw new HttpError(403, 'Members may remove only themselves.');
  }
  const result = await state.rooms.removeMember({
    roomId, memberId, removedByPrincipalId: principal.id,
  }).catch((error) => { throw roomsErrorToHttp(error); });
  await appendAudit(state, {
    actor: principal.name, actorId: principal.id, action: 'room.member_remove', roomId,
    memberId, removedCount: result.removed.length,
  });
  return result;
}

// R-8: room-aware ledger gating. When correlation_id or virtual_path
// identifies a REGISTERED room: valid token + room scope + active membership;
// writes require the canonical pair AND an actor_member_id owned by the
// caller. Reads may identify by correlation alone. Non-room ledger behavior
// is untouched (including deployments without federation, where no rooms
// exist by construction).
const ROOM_PATH_RE = /^\/rooms\/([^/]+)$/;

async function gateRoomLedgerAccess(state, principal, { correlationId, virtualPath, isWrite, actorMemberId }) {
  if (!state.federationAvailable) return;
  const pathMatch = virtualPath ? ROOM_PATH_RE.exec(virtualPath) : null;
  const candidates = new Set();
  if (correlationId) candidates.add(correlationId);
  if (pathMatch) candidates.add(pathMatch[1]);
  if (!candidates.size) return;

  let roomId = null;
  for (const candidate of candidates) {
    if (await state.rooms.findRoom(candidate)) { roomId = candidate; break; }
  }
  if (!roomId) return;

  if (isWrite) {
    if (correlationId !== roomId || virtualPath !== roomScope(roomId)) {
      throw new HttpError(400, `Registered-room writes require correlation_id = <room_id> and virtual_path = ${roomScope(roomId)}.`);
    }
  } else if (correlationId && pathMatch && correlationId !== pathMatch[1]) {
    throw new HttpError(400, 'correlation_id and virtual_path identify different rooms.');
  } else if (!correlationId && pathMatch) {
    // Path-only read of a registered room is fine; gate on that room.
  }

  requirePathScope(principal, roomScope(roomId));
  await requireActiveRoomMembership(state, principal, roomId);

  if (isWrite) {
    if (!actorMemberId) throw new HttpError(403, 'Registered-room writes require actor_member_id.');
    const owned = await state.rooms.memberOwnedByPrincipal(roomId, actorMemberId, principal.id);
    if (!owned) throw new HttpError(403, 'actor_member_id is not an active member owned by the calling principal.');
  }
}

function notFound() {
  throw new HttpError(404, 'Route not found.');
}

function envValue(primary, legacy, fallback = undefined) {
  return process.env[primary] || (legacy ? process.env[legacy] : undefined) || fallback;
}

export function createOceanBedrockServer(options = {}) {
  const root = path.resolve(options.root || envValue('OCEAN_BEDROCK_ROOT', 'RT_NAS_ROOT', path.join(process.cwd(), 'data')));
  const metaRoot = path.resolve(options.metaRoot || envValue('OCEAN_BEDROCK_META_ROOT', 'RT_NAS_META_ROOT', path.join(root, '.ocean-bedrock')));
  // Gate-2 F3: resolve the shared database URL ONCE, before auth resolution,
  // so a programmatic {databaseUrl} reaches auth exactly like env would.
  const databaseUrl = options.databaseUrl || process.env.DATABASE_URL || null;
  const state = {
    instance: options.instance || envValue('OCEAN_BEDROCK_INSTANCE', 'RT_NAS_INSTANCE', 'ocean-bedrock'),
    root,
    filesRoot: path.join(root, 'files'),
    metaRoot,
    authFile: path.resolve(options.authFile || envValue('OCEAN_BEDROCK_AUTH_FILE', 'RT_NAS_AUTH_FILE', path.join(metaRoot, 'tokens.json'))),
    authStore: options.authStore || process.env.OCEAN_AUTH_STORE || process.env.OCEAN_BEDROCK_AUTH_STORE || 'file',
    authDatabaseUrl: options.authDatabaseUrl || process.env.OCEAN_AUTH_DATABASE_URL || databaseUrl,
    locksFile: path.resolve(options.locksFile || envValue('OCEAN_BEDROCK_LOCKS_FILE', 'RT_NAS_LOCKS_FILE', path.join(metaRoot, 'locks.json'))),
    auditFile: path.resolve(options.auditFile || envValue('OCEAN_BEDROCK_AUDIT_FILE', 'RT_NAS_AUDIT_FILE', path.join(metaRoot, 'audit.jsonl'))),
    ledgerFile: path.resolve(options.ledgerFile || process.env.OCEAN_LEDGER_FILE || process.env.LONGHOUSE_LEDGER_FILE || process.env.RT_NAS_LEDGER_FILE || path.join(metaRoot, 'ocean-ledger.jsonl')),
    ledgerStore: options.ledgerStore || process.env.OCEAN_LEDGER_STORE || (process.env.DATABASE_URL ? 'postgres' : 'jsonl'),
    databaseUrl,
    tmpRoot: path.resolve(options.tmpRoot || envValue('OCEAN_BEDROCK_TMP_ROOT', 'RT_NAS_TMP_ROOT', path.join(metaRoot, 'tmp'))),
    maxUploadBytes: parseSize(options.maxUploadBytes || envValue('OCEAN_BEDROCK_MAX_UPLOAD', 'RT_NAS_MAX_UPLOAD'), 250 * 1024 * 1024),
    maxJsonBytes: parseSize(options.maxJsonBytes || envValue('OCEAN_BEDROCK_MAX_JSON', 'RT_NAS_MAX_JSON'), 1024 * 1024),
    startedAt: new Date().toISOString(),
  };

  state.auth = createAuthStore({
    authFile: state.authFile,
    databaseUrl: state.authDatabaseUrl,
    store: state.authStore,
  });

  state.ledger = createLedgerStore({
    store: state.ledgerStore,
    ledgerFile: state.ledgerFile,
    databaseUrl: state.databaseUrl,
  });

  // Gate-2 F3: federation is available iff auth, ledger, and rooms resolve to
  // ONE Postgres database. Exact resolved values are compared, never logged.
  // Rooms reuse state.auth.pool — equality of the resolved URLs is proven
  // above, so that pool IS the state.databaseUrl database.
  state.federationAvailable = Boolean(
    state.databaseUrl
      && state.auth.kind === 'postgres'
      && state.ledger.kind === 'postgres'
      && state.authDatabaseUrl === state.databaseUrl,
  );
  state.rooms = state.federationAvailable ? createRoomsStore(state.auth.pool) : null;

  const initPromise = (async () => {
    await fs.mkdir(state.filesRoot, { recursive: true });
    await fs.mkdir(state.metaRoot, { recursive: true });
    await fs.mkdir(state.tmpRoot, { recursive: true });
    await seedDefaultFolders(state);
    await bootstrapAuth(state);
    startIngestWorker(state);
  })();

  async function router(req, res) {
    await initPromise;
    setCorsHeaders(req, res);
    if (req.method === 'OPTIONS') {
      res.writeHead(204);
      res.end();
      return;
    }

    const url = new URL(req.url, `http://${req.headers.host || 'localhost'}`);

    if (req.method === 'GET' && (url.pathname === '/' || url.pathname === '/api' || url.pathname === '/api/v1')) {
      jsonResponse(res, 200, {
        ok: true,
        service: 'ocean-bedrock',
        instance: state.instance,
        version: VERSION,
        docs: {
          health: '/health',
          openapi: '/api/v1/openapi.yaml',
          info: '/api/v1/info',
          list: '/api/v1/list?path=/&depth=1',
          file: '/api/v1/file?path=/docs/README.md&inline=1',
          ledger: '/api/v1/ledger/events',
          sources: '/api/v1/sources/adapters',
          semanticSearch: '/api/v1/semantic/search?q=term&path=/context',
          graph: '/api/v1/graph/neighborhood?path=/docs/README.md',
          oceanContextTriage: '/api/v1/ocean-context/triage/daily',
          toolbox: '/api/v1/toolbox/manifest',
          localFolderSync: '/api/v1/sync/local-folder/plan',
          sourceRunnerSync: '/api/v1/sources/instances/{id}/sync',
          mounts: '/api/v1/mounts',
        },
        note: 'Most /api/v1 routes require Authorization: Bearer <token>.',
      });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/health') {
      jsonResponse(res, 200, { ok: true, instance: state.instance, version: VERSION, uptimeSeconds: Math.floor(process.uptime()) });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/openapi.yaml') {
      const openApiPath = path.resolve(__dirname, '..', 'docs', 'openapi.yaml');
      const payload = await fs.readFile(openApiPath, 'utf8');
      textResponse(res, 200, payload, 'application/yaml; charset=utf-8');
      return;
    }

    if (!url.pathname.startsWith('/api/v1/')) notFound();

    if (req.method === 'POST' && url.pathname === '/api/v1/invites/redeem') {
      jsonResponse(res, 201, await handleInviteRedeem(req, state));
      return;
    }

    const principal = await authenticate(req, url, state);

    if (req.method === 'GET' && url.pathname === '/api/v1/info') {
      requirePermission(principal, 'read');
      jsonResponse(res, 200, {
        instance: state.instance,
        version: VERSION,
        apiVersion: 'v1',
        startedAt: state.startedAt,
        maxUploadBytes: state.maxUploadBytes,
        defaultFolders: ['/docs', '/context', '/sessions', '/handoffs', '/shared', '/vault'],
        oceanLedger: {
          store: state.ledger.kind,
          localFile: state.ledger.kind === 'jsonl' ? state.ledgerFile : undefined,
        },
        semantic: semanticStatus(),
        principal,
      });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/list') {
      requirePermission(principal, 'read');
      const apiPath = normalizeApiPath(url.searchParams.get('path') || '/');
      requirePathScope(principal, apiPath);
      const depth = Math.max(1, Math.min(Number(url.searchParams.get('depth') || 1), 20));
      jsonResponse(res, 200, await listDirectory(state, apiPath, depth));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/tree') {
      requirePermission(principal, 'read');
      const apiPath = normalizeApiPath(url.searchParams.get('path') || '/');
      requirePathScope(principal, apiPath);
      const depth = Math.max(1, Math.min(Number(url.searchParams.get('depth') || 5), 20));
      jsonResponse(res, 200, await listDirectory(state, apiPath, depth));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/stat') {
      requirePermission(principal, 'read');
      const apiPath = normalizeApiPath(url.searchParams.get('path') || '/');
      requirePathScope(principal, apiPath);
      jsonResponse(res, 200, await statPath(state, apiPath));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/file') {
      requirePermission(principal, 'read');
      const apiPath = normalizeApiPath(url.searchParams.get('path'));
      requirePathScope(principal, apiPath);
      const { adapter, relPath } = await resolveMount(state, apiPath);
      const info = await adapter.stat(relPath, apiPath);
      if (info.type !== 'file') throw new HttpError(400, `${apiPath} is not a file.`);
      if (req.headers['if-none-match'] === info.etag) {
        res.writeHead(304, { etag: info.etag });
        res.end();
        return;
      }
      const readResult = await adapter.read(relPath);
      res.writeHead(200, {
        'content-type': contentTypeFor(apiPath),
        'content-length': info.size,
        etag: info.etag,
        ...(info.mtime ? { 'last-modified': new Date(info.mtime).toUTCString() } : {}),
        'content-disposition': url.searchParams.get('inline') === '1' ? 'inline' : `attachment; filename="${path.basename(apiPath).replace(/"/g, '')}"`,
      });
      if (readResult.diskPath) {
        createReadStream(readResult.diskPath).pipe(res);
      } else {
        Readable.fromWeb(readResult.stream).pipe(res);
      }
      return;
    }

    if (req.method === 'PUT' && url.pathname === '/api/v1/file') {
      const apiPath = normalizeApiPath(url.searchParams.get('path'));
      jsonResponse(res, 200, await putFile(req, state, principal, apiPath));
      return;
    }

    if (req.method === 'DELETE' && url.pathname === '/api/v1/file') {
      const apiPath = normalizeApiPath(url.searchParams.get('path'));
      await deletePath(state, principal, apiPath, url.searchParams.get('recursive') === 'true');
      jsonResponse(res, 200, { ok: true, path: apiPath });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/mkdir') {
      jsonResponse(res, 201, await createDirectory(req, state, principal));
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/move') {
      jsonResponse(res, 200, await movePath(req, state, principal, false));
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/copy') {
      jsonResponse(res, 201, await movePath(req, state, principal, true));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/search') {
      jsonResponse(res, 200, await search(state, principal, url));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/ledger/events') {
      requirePermission(principal, 'read');
      const apiPath = url.searchParams.has('path') ? normalizeApiPath(url.searchParams.get('path')) : null;
      if (apiPath) requirePathScope(principal, apiPath);
      const events = await state.ledger.read({
        correlation_id: url.searchParams.get('correlation_id') || undefined,
        actor_id: url.searchParams.get('actor_id') || undefined,
        actor_name: url.searchParams.get('actor_name') || undefined,
        actor_type: url.searchParams.get('actor_type') || undefined,
        event_type: url.searchParams.get('event_type') || undefined,
        source_id: url.searchParams.get('source_id') || undefined,
        path: apiPath || undefined,
        limit: url.searchParams.get('limit') || 100,
      });
      const visible = events.filter((event) => ledgerEventVisibleToPrincipal(event, principal));
      jsonResponse(res, 200, { events: visible });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/ledger/events') {
      requirePermission(principal, 'write');
      const body = await readJsonBody(req, state.maxJsonBytes);
      const apiPath = body.virtual_path || body.virtualPath || body.path ? normalizeApiPath(body.virtual_path || body.virtualPath || body.path) : null;
      if (apiPath) requirePathScope(principal, apiPath);
      requireClearance(principal, body.clearance || 'UNCLASSIFIED');
      const adminOverride = principal.role === 'admin';
      const event = await state.ledger.append({
        ...body,
        virtual_path: apiPath,
        actor_type: adminOverride ? body.actor_type || body.actorType || 'agent' : 'agent',
        actor_id: adminOverride ? body.actor_id || body.actorId || principal.id : principal.id,
        actor_name: adminOverride ? body.actor_name || body.actorName || principal.name : principal.name,
        source_id: body.source_id || body.sourceId || 'longhouse-api',
      });
      jsonResponse(res, 201, { event });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/ledger/trace') {
      requirePermission(principal, 'read');
      const correlationId = url.searchParams.get('correlation_id');
      if (!correlationId) throw new HttpError(400, 'Missing correlation_id.');
      const events = await state.ledger.trace(correlationId, url.searchParams.get('limit') || 1000);
      const visible = events.filter((event) => ledgerEventVisibleToPrincipal(event, principal));
      jsonResponse(res, 200, { correlation_id: correlationId, events: visible });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/ledger/snapshots') {
      requirePermission(principal, 'write');
      const body = await readJsonBody(req, state.maxJsonBytes);
      const clearance = body.clearance || 'CONFIDENTIAL';
      requireClearance(principal, clearance);
      const apiPath = body.virtual_path || body.virtualPath || body.path ? normalizeApiPath(body.virtual_path || body.virtualPath || body.path) : null;
      if (apiPath) requirePathScope(principal, apiPath);
      const visibleFiles = body.files || body.visible_files || body.visibleFiles || [];
      if (!Array.isArray(visibleFiles)) throw new HttpError(400, 'files/visible_files must be an array when provided.');
      const normalizedFiles = visibleFiles.map((filePath) => normalizeApiPath(filePath));
      for (const filePath of normalizedFiles) requirePathScope(principal, filePath);
      const visibleEvents = body.events || body.visible_events || body.visibleEvents || [];
      if (!Array.isArray(visibleEvents)) throw new HttpError(400, 'events/visible_events must be an array when provided.');
      const snapshot = {
        name: body.name || null,
        snapshot_type: body.snapshot_type || body.snapshotType || 'context',
        summary: body.summary || null,
        files: normalizedFiles,
        events: visibleEvents,
        metadata: body.metadata || {},
        token_scope: principal.scopes || ['/'],
        clearance_level: clearance,
      };
      const event = await state.ledger.append({
        event_type: 'context.snapshot.created',
        correlation_id: body.correlation_id || body.correlationId || null,
        lab: body.lab || body.domain || 'context',
        actor_type: 'agent',
        actor_id: principal.id,
        actor_name: principal.name,
        source_id: body.source_id || body.sourceId || 'longhouse-api',
        virtual_path: apiPath,
        payload: snapshot,
        context_snapshot: snapshot,
        clearance,
        tags: Array.isArray(body.tags) ? body.tags : [],
      });
      jsonResponse(res, 201, { snapshot: event });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/ledger/verify') {
      requirePermission(principal, 'admin');
      jsonResponse(res, 200, await state.ledger.verify());
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/semantic/search') {
      await handleSemanticSearch(req, res, state, principal, url);
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/graph/nodes') {
      await handleGraphNodes(req, res, state, principal, url);
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/graph/neighborhood') {
      await handleGraphNeighborhood(req, res, state, principal, url);
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/ocean-context/triage/daily') {
      await handleOceanContextTriage(req, res, state, principal);
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/toolbox/manifest') {
      requirePermission(principal, 'read');
      const proto = String(req.headers['x-forwarded-proto'] || url.protocol.replace(/:$/, '') || 'https').split(',')[0].trim();
      jsonResponse(res, 200, toolboxManifest({ baseUrl: `${proto}://${req.headers.host}`, principal }));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/sources/adapters') {
      await handleSourceAdapters(req, res, state, principal, url);
      return;
    }

    if ((req.method === 'GET' || req.method === 'POST') && url.pathname === '/api/v1/sources/instances') {
      await handleSourceInstances(req, res, state, principal, url);
      return;
    }

    const sourceInstanceSyncMatch = url.pathname.match(/^\/api\/v1\/sources\/instances\/([^/]+)\/sync$/);
    if (req.method === 'POST' && sourceInstanceSyncMatch) {
      await handleSourceInstanceSync(req, res, state, principal, decodeURIComponent(sourceInstanceSyncMatch[1]), url);
      return;
    }

    const sourceInstanceMatch = url.pathname.match(/^\/api\/v1\/sources\/instances\/([^/]+)$/);
    if ((req.method === 'GET' || req.method === 'PATCH') && sourceInstanceMatch) {
      await handleSourceInstanceById(req, res, state, principal, decodeURIComponent(sourceInstanceMatch[1]));
      return;
    }

    if ((req.method === 'GET' || req.method === 'POST') && url.pathname === '/api/v1/sources/streams') {
      await handleSourceStreams(req, res, state, principal, url);
      return;
    }

    const sourceStreamMatch = url.pathname.match(/^\/api\/v1\/sources\/streams\/([^/]+)$/);
    if (req.method === 'PATCH' && sourceStreamMatch) {
      await handleSourceStreamById(req, res, state, principal, decodeURIComponent(sourceStreamMatch[1]));
      return;
    }

    if ((req.method === 'GET' || req.method === 'POST') && url.pathname === '/api/v1/sync-runs') {
      await handleSyncRuns(req, res, state, principal, url);
      return;
    }

    const syncRunActionMatch = url.pathname.match(/^\/api\/v1\/sync-runs\/([^/]+)\/(complete|fail)$/);
    if (req.method === 'POST' && syncRunActionMatch) {
      await handleCompleteSyncRun(req, res, state, principal, decodeURIComponent(syncRunActionMatch[1]), syncRunActionMatch[2] === 'fail');
      return;
    }

    const syncRunMatch = url.pathname.match(/^\/api\/v1\/sync-runs\/([^/]+)$/);
    if (req.method === 'GET' && syncRunMatch) {
      await handleSyncRunById(req, res, state, principal, decodeURIComponent(syncRunMatch[1]));
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/sync/local-folder/plan') {
      await handleLocalFolderPlan(req, res, state, principal);
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/sync/local-folder/records:batch') {
      await handleLocalFolderRecordsBatch(req, res, state, principal);
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/sync/local-folder/commit') {
      await handleLocalFolderCommit(req, res, state, principal);
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/locks') {
      requirePermission(principal, 'read');
      const locks = await loadLocks(state);
      const apiPath = url.searchParams.has('path') ? normalizeApiPath(url.searchParams.get('path')) : null;
      if (apiPath) requirePathScope(principal, apiPath);
      jsonResponse(res, 200, {
        locks: locks.locks.filter((lock) => !apiPath || pathsIntersect(lock.path, apiPath)),
      });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/locks') {
      jsonResponse(res, 201, await createLock(req, state, principal));
      return;
    }

    const lockDelete = url.pathname.match(/^\/api\/v1\/locks\/([^/]+)$/);
    if (req.method === 'DELETE' && lockDelete) {
      jsonResponse(res, 200, { released: await releaseLock(state, principal, decodeURIComponent(lockDelete[1])) });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/tokens') {
      requirePermission(principal, 'admin');
      const auth = await state.auth.load();
      jsonResponse(res, 200, { tokens: auth.tokens.map(publicTokenRecord), store: state.auth.kind });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/tokens') {
      jsonResponse(res, 201, await handleTokenCreate(req, state, principal));
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/invites') {
      requirePermission(principal, 'admin');
      const auth = await state.auth.load();
      jsonResponse(res, 200, { invites: (auth.invites || []).map(publicInviteRecord), store: state.auth.kind });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/invites') {
      jsonResponse(res, 201, await handleInviteCreate(req, state, principal));
      return;
    }

    const tokenDelete = url.pathname.match(/^\/api\/v1\/tokens\/([^/]+)$/);
    if (req.method === 'DELETE' && tokenDelete) {
      requirePermission(principal, 'admin');
      const revoked = await state.auth.revokeToken(decodeURIComponent(tokenDelete[1]), principal.id);
      if (!revoked) throw new HttpError(404, 'Token not found.');
      await appendAudit(state, {
        actor: principal.name,
        actorId: principal.id,
        action: 'token.revoke',
        tokenId: revoked.id,
      });
      jsonResponse(res, 200, { revoked });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/audit') {
      requirePermission(principal, 'admin');
      const limit = Math.max(1, Math.min(Number(url.searchParams.get('limit') || 100), 1000));
      jsonResponse(res, 200, { events: await readAudit(state, limit) });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/api/v1/mounts') {
      requirePermission(principal, 'admin');
      const db = getPool();
      if (!db) {
        const mounts = await loadMounts(state);
        jsonResponse(res, 200, { mounts: mounts.map(({ mount }) => mountPublicView(mount)) });
        return;
      }
      const result = await db.query(
        `SELECT id, type, prefix, mode, priority, capabilities, enabled, created_at, updated_at
         FROM longhouse.mounts ORDER BY length(prefix) DESC, priority DESC`,
      );
      jsonResponse(res, 200, { mounts: result.rows.map(mountPublicView) });
      return;
    }

    const mountById = url.pathname.match(/^\/api\/v1\/mounts\/([^/]+)$/);

    if (req.method === 'GET' && mountById) {
      requirePermission(principal, 'admin');
      const db = getPool();
      if (!db) throw new HttpError(503, 'Mount registry requires DATABASE_URL.');
      const result = await db.query(
        `SELECT id, type, prefix, mode, priority, config, capabilities, enabled, created_at, updated_at
         FROM longhouse.mounts WHERE id = $1`,
        [decodeURIComponent(mountById[1])],
      );
      if (!result.rows.length) throw new HttpError(404, 'Mount not found.');
      jsonResponse(res, 200, { mount: mountPublicViewWithConfig(result.rows[0]) });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/api/v1/mounts') {
      requirePermission(principal, 'admin');
      const db = getPool();
      if (!db) throw new HttpError(503, 'Mount registry requires DATABASE_URL.');
      const body = await readJsonBody(req, state.maxJsonBytes);
      const VALID_TYPES = new Set(['local', 'r2', 's3', 'github', 'webdav', 'sftp', 'remote-longhouse']);
      const VALID_MODES = new Set(['readwrite', 'readonly', 'writeonly']);
      if (!body.id || typeof body.id !== 'string') throw new HttpError(400, 'Mount id is required.');
      if (!VALID_TYPES.has(body.type)) throw new HttpError(400, `Invalid mount type. Must be one of: ${[...VALID_TYPES].join(', ')}.`);
      if (body.mode && !VALID_MODES.has(body.mode)) throw new HttpError(400, `Invalid mode. Must be one of: ${[...VALID_MODES].join(', ')}.`);
      if (!body.prefix || typeof body.prefix !== 'string') throw new HttpError(400, 'Mount prefix is required.');
      const prefix = normalizeApiPath(body.prefix);
      const config = body.config || {};
      validateMountConfig(body.type, config, state.filesRoot);
      const capabilities = body.capabilities || {};
      const priority = Number(body.priority ?? 100);
      const result = await db.query(
        `INSERT INTO longhouse.mounts (id, type, prefix, mode, priority, config, capabilities)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, type, prefix, mode, priority, capabilities, enabled, created_at, updated_at`,
        [body.id, body.type, prefix, body.mode || 'readwrite', priority, JSON.stringify(config), JSON.stringify(capabilities)],
      );
      invalidateMountRegistry();
      await appendAudit(state, { actor: principal.name, actorId: principal.id, action: 'mount.create', mountId: body.id });
      jsonResponse(res, 201, { mount: mountPublicView(result.rows[0]) });
      return;
    }

    if (req.method === 'DELETE' && mountById) {
      requirePermission(principal, 'admin');
      const db = getPool();
      if (!db) throw new HttpError(503, 'Mount registry requires DATABASE_URL.');
      const mountId = decodeURIComponent(mountById[1]);
      if (mountId === 'managed-local') throw new HttpError(400, 'The managed-local mount cannot be disabled.');
      const result = await db.query(
        `UPDATE longhouse.mounts SET enabled = false, updated_at = now() WHERE id = $1 RETURNING id`,
        [mountId],
      );
      if (!result.rows.length) throw new HttpError(404, 'Mount not found.');
      invalidateMountRegistry();
      await appendAudit(state, { actor: principal.name, actorId: principal.id, action: 'mount.disable', mountId });
      jsonResponse(res, 200, { disabled: mountId });
      return;
    }

    notFound();
  }

  const server = http.createServer((req, res) => {
    router(req, res).catch((error) => {
      const status = error instanceof HttpError ? error.status : 500;
      if (status >= 500 && !(error instanceof HttpError)) console.error('[ocean-bedrock] request failed', error);
      if (!res.headersSent) {
        jsonResponse(res, status, {
          ok: false,
          error: error.message || 'Internal server error',
          details: error.details,
        });
      } else {
        res.destroy(error);
      }
    });
  });

  return { server, state, ready: initPromise };
}

export const createRtNasServer = createOceanBedrockServer;

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const port = Number(process.env.PORT || envValue('OCEAN_BEDROCK_PORT', 'RT_NAS_PORT', 8080));
  const host = envValue('OCEAN_BEDROCK_HOST', 'RT_NAS_HOST', process.env.HOST || '0.0.0.0');
  const { server, state, ready } = createOceanBedrockServer();
  await ready;
  server.listen(port, host, () => {
    console.log(`[ocean-bedrock] ${state.instance} listening on http://${host}:${port}`);
    console.log(`[ocean-bedrock] files root: ${state.filesRoot}`);
    console.log(`[ocean-bedrock] auth file: ${state.authFile}`);
    console.log(`[ocean-bedrock] ocean ledger store: ${state.ledger.kind}`);
    if (state.ledger.kind === 'jsonl') console.log(`[ocean-bedrock] ocean ledger: ${state.ledgerFile}`);
    if (!envValue('OCEAN_BEDROCK_BOOTSTRAP_TOKEN', 'RT_NAS_BOOTSTRAP_TOKEN')) {
      console.log('[ocean-bedrock] create tokens with: npm run token:create -- --name agent-name --role agent');
    }
  });
}
