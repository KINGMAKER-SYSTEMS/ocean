// rooms.mjs — Gate-2 S1A federated room membership store.
// Frozen by gate2-s1a-freeze.md v4 (ocean-surface .stitchpad/artifacts).
// All room store logic lives here; server.mjs only wires routes and the
// room-aware ledger guards. Postgres-only: the caller (server.mjs) gates
// every entry point behind the single-shared-authority check (F3), so this
// module assumes a pg Pool and never sees file/JSONL state.

// ── validation ────────────────────────────────────────────────────────

const ROOM_ID_RE = /^[a-z0-9][a-z0-9._-]{0,127}$/;

export const INVITE_TTL_DEFAULT_MINUTES = 1440;   // 24h
export const INVITE_TTL_MAX_MINUTES = 10080;      // 7d

// Safe public agent descriptor: exactly these keys, nothing local.
const DESCRIPTOR_ALLOWED_KEYS = new Set([
  'display_name', 'description', 'model_alias', 'skills_count', 'subagent_names',
]);
// Any of these appearing anywhere in a descriptor is an immediate 400 —
// Bedrock never accepts local paths/tools/credentials.
const DESCRIPTOR_FORBIDDEN_KEYS = new Set([
  'path', 'paths', 'cwd', 'root', 'folder', 'workspace_root', 'tools', 'tool',
  'credentials', 'credential', 'token', 'bearer', 'secret', 'secrets',
  'api_key', 'apikey', 'env', 'permissions', 'home',
]);

export class RoomsError extends Error {
  constructor(status, message, code = null) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

export function normalizeRoomId(input) {
  const value = String(input ?? '').trim().toLowerCase();
  if (!ROOM_ID_RE.test(value)) {
    throw new RoomsError(400, 'Invalid room id: use 1-128 chars of a-z 0-9 . _ - starting alphanumeric.');
  }
  return value;
}

export function roomScope(roomId) {
  return `/rooms/${roomId}`;
}

export function roomVirtualPath(roomId) {
  return `/rooms/${roomId}`;
}

export function validateTtlMinutes(input) {
  // Default ONLY when the key is omitted; null/empty/numeric strings are
  // rejected — the wire type is a finite JSON integer.
  if (input === undefined) return INVITE_TTL_DEFAULT_MINUTES;
  if (typeof input !== 'number' || !Number.isFinite(input) || !Number.isInteger(input)
      || input < 1 || input > INVITE_TTL_MAX_MINUTES) {
    throw new RoomsError(400, `ttl_minutes must be a JSON integer in 1..${INVITE_TTL_MAX_MINUTES}.`);
  }
  return input;
}

export function validateAgentDescriptor(input) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    throw new RoomsError(400, 'Each agent descriptor must be an object.');
  }
  for (const key of Object.keys(input)) {
    const lower = key.toLowerCase();
    if (DESCRIPTOR_FORBIDDEN_KEYS.has(lower)) {
      throw new RoomsError(400, `Agent descriptor field "${key}" is not accepted: local paths, tools, and credentials never reach Bedrock.`);
    }
    if (!DESCRIPTOR_ALLOWED_KEYS.has(key)) {
      throw new RoomsError(400, `Unknown agent descriptor field "${key}". Allowed: ${[...DESCRIPTOR_ALLOWED_KEYS].join(', ')}.`);
    }
  }
  if (typeof input.display_name !== 'string' || !input.display_name.trim()) {
    throw new RoomsError(400, 'Agent descriptor requires display_name as a non-empty string.');
  }
  const descriptor = { display_name: input.display_name.trim() };
  if (input.description !== undefined) {
    if (typeof input.description !== 'string') throw new RoomsError(400, 'description must be a string.');
    descriptor.description = input.description.slice(0, 2048);
  }
  if (input.model_alias !== undefined) {
    if (typeof input.model_alias !== 'string') throw new RoomsError(400, 'model_alias must be a string.');
    descriptor.model_alias = input.model_alias.slice(0, 128);
  }
  if (input.skills_count !== undefined) {
    if (typeof input.skills_count !== 'number' || !Number.isInteger(input.skills_count)
        || input.skills_count < 0 || input.skills_count > 0xFFFFFFFF) {
      throw new RoomsError(400, 'skills_count must be an integer in 0..4294967295.');
    }
    descriptor.skills_count = input.skills_count;
  }
  if (input.subagent_names !== undefined) {
    if (!Array.isArray(input.subagent_names)
        || input.subagent_names.some((name) => typeof name !== 'string')) {
      throw new RoomsError(400, 'subagent_names must be an array of strings.');
    }
    descriptor.subagent_names = input.subagent_names.map((name) => name.slice(0, 128));
  }
  return descriptor;
}

// Canonical descriptor comparison for registration_key reuse: an exact retry
// returns the existing row; the same key with DIFFERENT content is a 409.
export function descriptorsEqual(a, b) {
  const keys = ['display_name', 'description', 'model_alias', 'skills_count', 'subagent_names'];
  return keys.every((key) => JSON.stringify(a?.[key] ?? null) === JSON.stringify(b?.[key] ?? null));
}

// Stable opaque per-descriptor idempotency key when the daemon does not
// supply one is NOT allowed — the daemon must send registration_key so a
// lost HTTP response cannot mint a twin. We only validate shape here.
export function validateRegistrationKey(input) {
  const value = String(input ?? '').trim();
  if (!value || value.length > 128) {
    throw new RoomsError(400, 'Each agent requires a stable registration_key (1-128 chars, daemon-supplied).');
  }
  return value;
}

// ── safe projection ───────────────────────────────────────────────────

// Public member projection. Omits owner_principal_token_id,
// registration_key, removed_by, and every other internal field.
export function publicMemberProjection(row) {
  if (!row) return null;
  const projection = {
    member_id: row.member_id,
    actor_type: row.actor_type,
    role_in_room: row.role_in_room,
    display_name: row.display_name,
    joined_at: row.joined_at?.toISOString?.() || row.joined_at,
  };
  if (row.owner_member_id) projection.owner_member_id = row.owner_member_id;
  if (row.public_agent_descriptor) projection.public_agent_descriptor = row.public_agent_descriptor;
  return projection;
}

// ── store ─────────────────────────────────────────────────────────────

export function createRoomsStore(pool) {
  async function withTransaction(work) {
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      const result = await work(client);
      await client.query('COMMIT');
      return result;
    } catch (error) {
      await client.query('ROLLBACK').catch(() => {});
      throw error;
    } finally {
      client.release();
    }
  }

  async function activeMember(client, roomId, principalTokenId) {
    const result = await client.query(
      `SELECT * FROM longhouse.room_members
       WHERE room_id = $1 AND owner_principal_token_id = $2
         AND actor_type = 'user' AND removed_at IS NULL
       LIMIT 1`,
      [roomId, principalTokenId],
    );
    return result.rows[0] || null;
  }

  return {
    // R-1: atomic correlation claim + owner row. Retry key (room_id, principal.id).
    // Concurrency: ON CONFLICT DO NOTHING waits out the competing insert, so
    // the follow-up read sees the committed winner — never a raw 23505/500.
    async register({ roomId, principal, title = null }) {
      return withTransaction(async (client) => {
        const claimed = await client.query(
          `INSERT INTO longhouse.ledger_correlations (id, title, metadata)
           VALUES ($1, $2, $3::jsonb)
           ON CONFLICT (id) DO NOTHING
           RETURNING id`,
          [roomId, title || roomId, JSON.stringify({
            federation_kind: 'room',
            owner_principal_token_id: principal.id,
          })],
        );
        if (claimed.rows.length) {
          const inserted = await client.query(
            `INSERT INTO longhouse.room_members
               (room_id, actor_type, role_in_room, display_name, owner_principal_token_id)
             VALUES ($1, 'user', 'owner', $2, $3)
             RETURNING *`,
            [roomId, principal.name || principal.id, principal.id],
          );
          return { status: 201, room_id: roomId, owner: publicMemberProjection(inserted.rows[0]) };
        }

        // Lost the insert (or the row predates us): re-read the winner.
        const correlation = await client.query(
          'SELECT id, metadata FROM longhouse.ledger_correlations WHERE id = $1',
          [roomId],
        );
        const meta = correlation.rows[0]?.metadata || {};
        if (meta.federation_kind !== 'room') {
          throw new RoomsError(409, 'A different correlation already exists with this id; it cannot be claimed as a room.');
        }
        if (meta.owner_principal_token_id !== principal.id) {
          throw new RoomsError(409, 'Room is already registered by another principal.');
        }
        const owner = await activeMember(client, roomId, principal.id);
        if (!owner || owner.role_in_room !== 'owner') {
          throw new RoomsError(409, 'Room correlation exists but its owner row is missing; contact an admin.');
        }
        return { status: 200, room_id: roomId, owner: publicMemberProjection(owner) };
      });
    },

    // Registered-room lookup for R-8 gating. Returns null for non-rooms.
    async findRoom(correlationId) {
      const result = await pool.query(
        `SELECT id, metadata FROM longhouse.ledger_correlations
         WHERE id = $1 AND metadata->>'federation_kind' = 'room'`,
        [correlationId],
      );
      return result.rows[0] || null;
    },

    async activeMembership(roomId, principalTokenId) {
      const result = await pool.query(
        `SELECT * FROM longhouse.room_members
         WHERE room_id = $1 AND owner_principal_token_id = $2 AND removed_at IS NULL`,
        [roomId, principalTokenId],
      );
      return result.rows;
    },

    async memberOwnedByPrincipal(roomId, memberId, principalTokenId) {
      const result = await pool.query(
        `SELECT 1 FROM longhouse.room_members
         WHERE room_id = $1 AND member_id = $2
           AND owner_principal_token_id = $3 AND removed_at IS NULL`,
        [roomId, memberId, principalTokenId],
      );
      return result.rows.length > 0;
    },

    async wasRemoved(roomId, principalTokenId) {
      const result = await pool.query(
        `SELECT 1 FROM longhouse.room_members
         WHERE room_id = $1 AND owner_principal_token_id = $2
           AND actor_type = 'user' AND removed_at IS NOT NULL
         LIMIT 1`,
        [roomId, principalTokenId],
      );
      return result.rows.length > 0;
    },

    // R-4: self-join after scoped redeem. Provenance + tombstone are checked
    // by the caller (server.mjs owns auth); this transaction owns identity.
    async selfJoin({ roomId, principal }) {
      return withTransaction(async (client) => {
        const room = await client.query(
          `SELECT id FROM longhouse.ledger_correlations
           WHERE id = $1 AND metadata->>'federation_kind' = 'room' FOR SHARE`,
          [roomId],
        );
        if (!room.rows.length) throw new RoomsError(404, 'Room is not registered.');

        // Tombstone check INSIDE the transaction (frozen: transactional).
        const tombstone = await client.query(
          `SELECT 1 FROM longhouse.room_members
           WHERE room_id = $1 AND owner_principal_token_id = $2
             AND actor_type = 'user' AND removed_at IS NOT NULL
           LIMIT 1`,
          [roomId, principal.id],
        );
        if (tombstone.rows.length) {
          throw new RoomsError(403, 'membership_revoked', 'membership_revoked');
        }

        const existing = await activeMember(client, roomId, principal.id);
        if (existing) {
          return { status: 200, member: publicMemberProjection(existing) };
        }
        // ON CONFLICT (not catch-23505): a unique violation aborts the whole
        // transaction (25P02), so the loser must not error at all. The partial
        // unique index is inferred by matching its predicate.
        const inserted = await client.query(
          `INSERT INTO longhouse.room_members
             (room_id, actor_type, role_in_room, display_name, owner_principal_token_id)
           VALUES ($1, 'user', 'member', $2, $3)
           ON CONFLICT (room_id, owner_principal_token_id)
             WHERE removed_at IS NULL AND actor_type = 'user'
             DO NOTHING
           RETURNING *`,
          [roomId, principal.name || principal.id, principal.id],
        );
        if (inserted.rows.length) {
          return { status: 201, member: publicMemberProjection(inserted.rows[0]) };
        }
        const winner = await activeMember(client, roomId, principal.id);
        if (winner) return { status: 200, member: publicMemberProjection(winner) };
        throw new RoomsError(409, 'Concurrent self-join conflict; retry.');
      });
    },

    // R-5: batch agent registration. ONE transaction, all-or-nothing.
    // registration_key is the idempotency identity; response preserves
    // request order. 201 ONLY when every requested row is new; mixed or
    // all-existing → 200. Same key + different descriptor content → 409.
    // Duplicate keys within one batch → 400 (validated by the caller).
    async registerAgents({ roomId, principal, agents }) {
      return withTransaction(async (client) => {
        const human = await activeMember(client, roomId, principal.id);
        if (!human) throw new RoomsError(403, 'Caller has no active human membership in this room.');

        const members = [];
        let createdCount = 0;
        for (const agent of agents) {
          const inserted = await client.query(
            `INSERT INTO longhouse.room_members
               (room_id, actor_type, role_in_room, display_name, owner_member_id,
                owner_principal_token_id, registration_key, public_agent_descriptor)
             VALUES ($1, 'agent', 'member', $2, $3, $4, $5, $6::jsonb)
             ON CONFLICT (room_id, owner_principal_token_id, registration_key)
               WHERE removed_at IS NULL AND actor_type = 'agent'
               DO NOTHING
             RETURNING *`,
            [roomId, agent.descriptor.display_name, human.member_id, principal.id,
             agent.registration_key, JSON.stringify(agent.descriptor)],
          );
          if (inserted.rows.length) {
            createdCount += 1;
            members.push(publicMemberProjection(inserted.rows[0]));
            continue;
          }
          // Lost or retried: read the winner and compare content — an exact
          // retry returns the existing row, changed content is a conflict,
          // never a silent stale identity.
          const winner = await client.query(
            `SELECT * FROM longhouse.room_members
             WHERE room_id = $1 AND owner_principal_token_id = $2
               AND registration_key = $3 AND actor_type = 'agent' AND removed_at IS NULL`,
            [roomId, principal.id, agent.registration_key],
          );
          const row = winner.rows[0];
          if (!row) throw new RoomsError(409, 'Concurrent agent registration conflict; retry.');
          if (!descriptorsEqual(row.public_agent_descriptor, agent.descriptor)) {
            throw new RoomsError(409, `registration_key reuse with different descriptor content for "${agent.descriptor.display_name}".`);
          }
          members.push(publicMemberProjection(row));
        }
        return { status: createdCount === agents.length ? 201 : 200, members };
      });
    },

    // R-6: shared active roster, safe projections only.
    async roster(roomId) {
      const result = await pool.query(
        `SELECT * FROM longhouse.room_members
         WHERE room_id = $1 AND removed_at IS NULL
         ORDER BY joined_at ASC, member_id ASC`,
        [roomId],
      );
      return result.rows.map(publicMemberProjection);
    },

    // R-7: soft removal. Owner removes any other member (a human's owned
    // agents go atomically); a member removes only self. Owner self-leave
    // and authorization are the caller's checks; this owns the transaction.
    async removeMember({ roomId, memberId, removedByPrincipalId }) {
      return withTransaction(async (client) => {
        const target = await client.query(
          'SELECT * FROM longhouse.room_members WHERE room_id = $1 AND member_id = $2 FOR UPDATE',
          [roomId, memberId],
        );
        const row = target.rows[0];
        if (!row) throw new RoomsError(404, 'Member not found in this room.');
        if (row.removed_at) {
          return { status: 200, removed: [] };   // idempotent repeat (caller re-authorized)
        }
        const removed = [];
        if (row.actor_type === 'user') {
          const agents = await client.query(
            `UPDATE longhouse.room_members
             SET removed_at = now(), removed_by = $3
             WHERE room_id = $1 AND owner_member_id = $2
               AND actor_type = 'agent' AND removed_at IS NULL
             RETURNING member_id`,
            [roomId, memberId, removedByPrincipalId],
          );
          removed.push(...agents.rows.map((agent) => agent.member_id));
        }
        await client.query(
          `UPDATE longhouse.room_members
           SET removed_at = now(), removed_by = $3
           WHERE room_id = $1 AND member_id = $2`,
          [roomId, memberId, removedByPrincipalId],
        );
        removed.push(row.member_id);
        return { status: 200, removed };
      });
    },
  };
}
