-- 006_federated_rooms.sql — Gate-2 S1A: federated room membership.
-- Frozen by .stitchpad/artifacts/gate2-s1a-freeze.md v4 (ocean-surface repo).
-- Additive only. room_id IS the ledger correlation id; the correlation row is
-- marked with metadata {"federation_kind":"room","owner_principal_token_id":...}.

BEGIN;

CREATE TABLE IF NOT EXISTS longhouse.room_members (
  room_id TEXT NOT NULL REFERENCES longhouse.ledger_correlations(id),
  member_id UUID NOT NULL DEFAULT gen_random_uuid(),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('user', 'agent')),
  role_in_room TEXT NOT NULL DEFAULT 'member'
    CHECK (role_in_room IN ('owner', 'member')),
  display_name TEXT NOT NULL,
  owner_member_id UUID,
  owner_principal_token_id TEXT NOT NULL
    REFERENCES longhouse.auth_tokens(id),
  registration_key TEXT,            -- internal agent idempotency; never public
  public_agent_descriptor JSONB,
  joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  removed_at TIMESTAMPTZ,
  removed_by TEXT REFERENCES longhouse.auth_tokens(id),  -- internal audit only

  PRIMARY KEY (room_id, member_id),

  -- Referenced target for the 3-column composite FK below.
  CONSTRAINT room_members_identity_principal_uniq
    UNIQUE (room_id, member_id, owner_principal_token_id),

  -- Agent rows point at their owning human row in the SAME room under the
  -- SAME principal: the third FK leg forces the agent row's
  -- owner_principal_token_id to equal the parent row's, so a cross-principal
  -- agent insert fails in PostgreSQL, not just the store.
  CONSTRAINT room_members_owner_fk
    FOREIGN KEY (room_id, owner_member_id, owner_principal_token_id)
    REFERENCES longhouse.room_members (room_id, member_id, owner_principal_token_id),

  -- user rows have no owner/key/descriptor; agent rows require owner+key and
  -- can never hold the owner role
  CONSTRAINT room_members_actor_shape_chk CHECK (
    (actor_type = 'user' AND owner_member_id IS NULL
       AND registration_key IS NULL AND public_agent_descriptor IS NULL)
    OR
    (actor_type = 'agent' AND owner_member_id IS NOT NULL
       AND registration_key IS NOT NULL AND role_in_room = 'member')
  )
);

-- one active owner per room
CREATE UNIQUE INDEX IF NOT EXISTS room_members_one_active_owner_idx
  ON longhouse.room_members (room_id)
  WHERE removed_at IS NULL AND role_in_room = 'owner';

-- one active human row per (room, principal)
CREATE UNIQUE INDEX IF NOT EXISTS room_members_one_active_human_idx
  ON longhouse.room_members (room_id, owner_principal_token_id)
  WHERE removed_at IS NULL AND actor_type = 'user';

-- agent idempotency: one active row per internal registration key, so a lost
-- HTTP response cannot mint a twin on retry
CREATE UNIQUE INDEX IF NOT EXISTS room_members_registration_key_idx
  ON longhouse.room_members (room_id, owner_principal_token_id, registration_key)
  WHERE removed_at IS NULL AND actor_type = 'agent';

CREATE INDEX IF NOT EXISTS room_members_active_roster_idx
  ON longhouse.room_members (room_id)
  WHERE removed_at IS NULL;

-- Commit-time owner guard: every ACTIVE agent row must reference an ACTIVE
-- actor_type='user' owner. Covers (a) agent INSERT/UPDATE pointing at a
-- non-user or removed owner, and (b) human removal that would orphan active
-- agents. The composite FK cannot see actor_type or removed_at, so this
-- trigger closes both gaps at the database layer.
CREATE OR REPLACE FUNCTION longhouse.room_members_owner_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF NEW.actor_type = 'agent' AND NEW.removed_at IS NULL THEN
    IF NOT EXISTS (
      SELECT 1 FROM longhouse.room_members owner_row
      WHERE owner_row.room_id = NEW.room_id
        AND owner_row.member_id = NEW.owner_member_id
        AND owner_row.actor_type = 'user'
        AND owner_row.removed_at IS NULL
    ) THEN
      RAISE EXCEPTION
        'room_members owner guard: active agent % in room % must reference an active user owner',
        NEW.member_id, NEW.room_id
        USING ERRCODE = '23514';
    END IF;
  END IF;

  IF NEW.actor_type = 'user' AND NEW.removed_at IS NOT NULL THEN
    IF EXISTS (
      SELECT 1 FROM longhouse.room_members agent_row
      WHERE agent_row.room_id = NEW.room_id
        AND agent_row.owner_member_id = NEW.member_id
        AND agent_row.actor_type = 'agent'
        AND agent_row.removed_at IS NULL
    ) THEN
      RAISE EXCEPTION
        'room_members owner guard: removing user % in room % would orphan active agent rows',
        NEW.member_id, NEW.room_id
        USING ERRCODE = '23514';
    END IF;
  END IF;

  RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS room_members_owner_guard_trg ON longhouse.room_members;
CREATE CONSTRAINT TRIGGER room_members_owner_guard_trg
  AFTER INSERT OR UPDATE ON longhouse.room_members
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW
  EXECUTE FUNCTION longhouse.room_members_owner_guard();

COMMIT;
