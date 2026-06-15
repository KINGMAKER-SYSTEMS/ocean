import path from 'node:path';
import { getPool } from '../metadata.mjs';
import { LocalStorageAdapter } from './adapters/local.mjs';
import { createUnimplementedAdapter } from './adapter.mjs';

let _cache = null;
let _cacheExpiry = 0;
const TTL_MS = 30_000;

export function invalidateMountRegistry() {
  _cache = null;
  _cacheExpiry = 0;
}

function buildAdapter(mount, state) {
  if (mount.type === 'local') {
    let absRoot;
    if (mount.id === 'managed-local') {
      absRoot = state.filesRoot;
    } else if (typeof mount.config?.root === 'string' && path.isAbsolute(mount.config.root)) {
      absRoot = mount.config.root;
    } else {
      absRoot = path.resolve(state.root, mount.config?.root || '');
    }
    return new LocalStorageAdapter(mount, absRoot);
  }
  return createUnimplementedAdapter(mount);
}

const SYNTHETIC_LOCAL_MOUNT = {
  id: 'managed-local',
  type: 'local',
  prefix: '/',
  mode: 'readwrite',
  priority: 100,
  config: {},
  capabilities: { read: true, write: true, delete: true, move: true },
  enabled: true,
};

export async function loadMounts(state) {
  const now = Date.now();
  if (_cache && now < _cacheExpiry) return _cache;

  const db = getPool();
  if (!db) {
    _cache = [{ mount: SYNTHETIC_LOCAL_MOUNT, adapter: buildAdapter(SYNTHETIC_LOCAL_MOUNT, state) }];
    _cacheExpiry = now + TTL_MS;
    return _cache;
  }

  const result = await db.query(
    `SELECT id, type, prefix, mode, priority, config, capabilities, enabled
     FROM longhouse.mounts
     WHERE enabled = true
     ORDER BY length(prefix) DESC, priority DESC`,
  );

  _cache = result.rows.map((row) => {
    const mount = {
      id: row.id,
      type: row.type,
      prefix: row.prefix,
      mode: row.mode,
      priority: row.priority,
      config: row.config || {},
      capabilities: row.capabilities || {},
      enabled: row.enabled,
    };
    return { mount, adapter: buildAdapter(mount, state) };
  });
  _cacheExpiry = now + TTL_MS;
  return _cache;
}
