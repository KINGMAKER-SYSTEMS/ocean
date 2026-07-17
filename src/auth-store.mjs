import crypto from 'node:crypto';
import { Pool } from 'pg';

import {
  addToken,
  createInvite,
  findToken,
  hashToken,
  isTokenExpired,
  loadAuthFile,
  normalizeScopes,
  publicInviteRecord,
  publicTokenRecord,
  redeemInvite,
  revokeToken,
  VALID_ROLES,
  generateToken,
  nowIso,
} from './auth.mjs';

function timingSafeEqualString(a, b) {
  const left = Buffer.from(String(a));
  const right = Buffer.from(String(b));
  if (left.length !== right.length) return false;
  return crypto.timingSafeEqual(left, right);
}

function tokenRowToRecord(row) {
  if (!row) return null;
  return {
    id: row.id,
    name: row.name,
    hash: row.hash,
    role: row.role,
    scopes: row.scopes || ['/'],
    createdAt: row.created_at?.toISOString?.() || row.created_at,
    createdBy: row.created_by || null,
    expiresAt: row.expires_at?.toISOString?.() || row.expires_at || null,
    revokedAt: row.revoked_at?.toISOString?.() || row.revoked_at || null,
    revokedBy: row.revoked_by || null,
  };
}

function inviteRowToRecord(row) {
  if (!row) return null;
  return {
    id: row.id,
    name: row.name,
    hash: row.hash,
    role: row.role,
    scopes: row.scopes || ['/'],
    createdAt: row.created_at?.toISOString?.() || row.created_at,
    createdBy: row.created_by || null,
    expiresAt: row.expires_at?.toISOString?.() || row.expires_at,
    consumedAt: row.consumed_at?.toISOString?.() || row.consumed_at || null,
    consumedByTokenId: row.consumed_by_token_id || null,
    redemptionId: row.redemption_id || null,
  };
}

function makeId(prefix, hash) {
  return `${prefix}_${crypto.createHash('sha256').update(`${hash}:${Date.now()}:${Math.random()}`).digest('hex').slice(0, 12)}`;
}

export function createFileAuthStore(authFile) {
  return {
    kind: 'file',
    async load() {
      return loadAuthFile(authFile);
    },
    async findToken(token) {
      return findToken(await loadAuthFile(authFile), token);
    },
    async addToken(options) {
      return addToken(authFile, options);
    },
    async revokeToken(tokenId, revokedBy = null) {
      return revokeToken(authFile, tokenId, revokedBy);
    },
    async createInvite(options) {
      return createInvite(authFile, options);
    },
    async redeemInvite(code, options = {}) {
      return redeemInvite(authFile, code, options);
    },
  };
}

export function createPostgresAuthStore(databaseUrl) {
  const pool = new Pool({ connectionString: databaseUrl });
  return {
    kind: 'postgres',
    pool,
    async load() {
      const [tokens, invites] = await Promise.all([
        pool.query('SELECT * FROM longhouse.auth_tokens ORDER BY created_at DESC'),
        pool.query('SELECT * FROM longhouse.auth_invites ORDER BY created_at DESC'),
      ]);
      return {
        version: 1,
        tokens: tokens.rows.map(tokenRowToRecord),
        invites: invites.rows.map(inviteRowToRecord),
      };
    },
    async findToken(token) {
      const wanted = hashToken(token);
      const result = await pool.query(
        `SELECT * FROM longhouse.auth_tokens
         WHERE hash = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
         LIMIT 1`,
        [wanted],
      );
      const record = tokenRowToRecord(result.rows[0]);
      return record && timingSafeEqualString(record.hash, wanted) ? record : null;
    },
    async addToken(options = {}) {
      const token = options.token || generateToken();
      const hash = hashToken(token);
      const role = options.role || 'contributor';
      if (!VALID_ROLES.has(role)) throw new Error(`Invalid role "${role}". Use one of: ${[...VALID_ROLES].join(', ')}`);
      const id = makeId('tok', hash);
      const scopes = normalizeScopes(options.scopes || ['/']);
      try {
        const result = await pool.query(
          `INSERT INTO longhouse.auth_tokens (id, name, hash, role, scopes, created_by, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *`,
          [id, options.name || id, hash, role, scopes, options.createdBy || null, options.expiresAt || null],
        );
        return { token, record: publicTokenRecord(tokenRowToRecord(result.rows[0])) };
      } catch (error) {
        if (error.code === '23505') throw new Error('A token with this secret already exists.');
        throw error;
      }
    },
    async revokeToken(tokenId, revokedBy = null) {
      const result = await pool.query(
        `UPDATE longhouse.auth_tokens
         SET revoked_at = COALESCE(revoked_at, now()), revoked_by = COALESCE(revoked_by, $2)
         WHERE id = $1
         RETURNING *`,
        [tokenId, revokedBy],
      );
      return result.rows[0] ? publicTokenRecord(tokenRowToRecord(result.rows[0])) : null;
    },
    async createInvite(options = {}) {
      const code = options.code || generateToken(24);
      const hash = hashToken(code);
      const role = options.role || 'contributor';
      if (!VALID_ROLES.has(role)) throw new Error(`Invalid role "${role}". Use one of: ${[...VALID_ROLES].join(', ')}`);
      if (role === 'admin' && !options.allowAdminInvite) throw new Error('Admin invites are not allowed by default.');
      const ttlMinutes = Math.max(1, Number(options.ttlMinutes || 60));
      const expiresAt = options.expiresAt || new Date(Date.now() + ttlMinutes * 60 * 1000).toISOString();
      const id = makeId('inv', hash);
      const scopes = normalizeScopes(options.scopes || ['/']);
      try {
        const result = await pool.query(
          `INSERT INTO longhouse.auth_invites (id, name, hash, role, scopes, created_by, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *`,
          [id, options.name || id, hash, role, scopes, options.createdBy || null, expiresAt],
        );
        return { code, invite: publicInviteRecord(inviteRowToRecord(result.rows[0])) };
      } catch (error) {
        if (error.code === '23505') throw new Error('An invite with this code already exists.');
        throw error;
      }
    },
    async redeemInvite(code, options = {}) {
      const wanted = hashToken(code || '');
      const redemptionId = options.redemptionId || null;
      const token = options.token || generateToken();
      const tokenHash = hashToken(token);
      const client = await pool.connect();
      try {
        await client.query('BEGIN');
        // S2 blockers B1 (idempotent form): the caller durably owns the
        // bearer + redemption_id BEFORE this call, so the row is selected
        // regardless of consumption to distinguish replay from reuse.
        // Legacy {code}-only keeps the original unconsumed/unexpired filter.
        const inviteResult = await client.query(
          redemptionId
            ? `SELECT * FROM longhouse.auth_invites
               WHERE hash = $1
               FOR UPDATE`
            : `SELECT * FROM longhouse.auth_invites
               WHERE hash = $1 AND consumed_at IS NULL AND expires_at > now()
               FOR UPDATE`,
          [wanted],
        );
        const invite = inviteRowToRecord(inviteResult.rows[0]);
        if (redemptionId && invite && invite.consumedAt) {
          // Exact replay: same code + same redemption_id + same token hash
          // returns the same safe record with NO state change. Anything else
          // is a second consumer and fails exactly like a consumed invite.
          // Replay does not re-check expiry: the redemption already happened;
          // only its response was lost.
          if (invite.redemptionId !== redemptionId || !invite.consumedByTokenId) {
            await client.query('ROLLBACK');
            return null;
          }
          const tokenResult = await client.query(
            'SELECT * FROM longhouse.auth_tokens WHERE id = $1',
            [invite.consumedByTokenId],
          );
          const existing = tokenRowToRecord(tokenResult.rows[0]);
          if (!existing || !timingSafeEqualString(existing.hash, tokenHash)) {
            await client.query('ROLLBACK');
            return null;
          }
          await client.query('COMMIT');
          return {
            replayed: true,
            invite: publicInviteRecord(invite),
            record: publicTokenRecord(existing),
          };
        }
        if (!invite || invite.consumedAt || isTokenExpired(invite)) {
          await client.query('ROLLBACK');
          return null;
        }
        const tokenRecord = {
          id: makeId('tok', tokenHash),
          name: invite.name,
          role: invite.role,
          scopes: normalizeScopes(invite.scopes || ['/']),
          createdBy: invite.id,
          expiresAt: options.expiresAt || null,
        };
        const inserted = await client.query(
          `INSERT INTO longhouse.auth_tokens (id, name, hash, role, scopes, created_by, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *`,
          [tokenRecord.id, tokenRecord.name, tokenHash, tokenRecord.role, tokenRecord.scopes, tokenRecord.createdBy, tokenRecord.expiresAt],
        );
        const updated = await client.query(
          `UPDATE longhouse.auth_invites
           SET consumed_at = now(), consumed_by_token_id = $2, redemption_id = $3
           WHERE id = $1
           RETURNING *`,
          [invite.id, tokenRecord.id, redemptionId],
        );
        await client.query('COMMIT');
        return {
          token,
          replayed: false,
          invite: publicInviteRecord(inviteRowToRecord(updated.rows[0])),
          record: publicTokenRecord(tokenRowToRecord(inserted.rows[0])),
        };
      } catch (error) {
        await client.query('ROLLBACK').catch(() => {});
        if (error.code === '23505') {
          // Extended form: EVERY uniqueness collision is a second-consumer
          // 403, not a 500 — a reused redemption_id hits the 007 partial
          // index, and a reused TOKEN against a different invite hits the
          // auth_tokens hash constraint first.
          if (redemptionId) return null;
          throw new Error('A token with this secret already exists.');
        }
        throw error;
      } finally {
        client.release();
      }
    },
    async close() {
      await pool.end();
    },
  };
}

export function createAuthStore({ authFile, databaseUrl, store = 'file' } = {}) {
  if (store === 'postgres') {
    if (!databaseUrl) throw new Error('OCEAN_AUTH_STORE=postgres requires DATABASE_URL or OCEAN_AUTH_DATABASE_URL.');
    return createPostgresAuthStore(databaseUrl);
  }
  return createFileAuthStore(authFile);
}
