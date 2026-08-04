-- Gate-2 S2 bedrock blockers (gate2-s2-bedrock-blockers-freeze-v4.md).
-- 005/006 are shipped and immutable; both bridge-authority DDLs land here.
--
-- B1: idempotent invite redemption — the redeeming daemon supplies a
--     client-generated redemption_id (uuid) so an exact replay after
--     response loss returns the same consumed record instead of a 403.
-- B2: authenticated room message payloads — the authorized actor's opaque
--     member id is retained on the hash-chained ledger row itself.

BEGIN;

ALTER TABLE longhouse.auth_invites
  ADD COLUMN IF NOT EXISTS redemption_id uuid;

CREATE UNIQUE INDEX IF NOT EXISTS auth_invites_redemption_id_key
  ON longhouse.auth_invites (redemption_id)
  WHERE redemption_id IS NOT NULL;

ALTER TABLE longhouse.ledger_events
  ADD COLUMN IF NOT EXISTS actor_member_id uuid;

COMMIT;
