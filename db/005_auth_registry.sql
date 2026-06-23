-- Ocean Bedrock auth registry tables.
-- Target: PostgreSQL 15+.
-- Apply after db/001_longhouse_core.sql.

BEGIN;

CREATE SCHEMA IF NOT EXISTS longhouse;

CREATE TABLE IF NOT EXISTS longhouse.auth_tokens (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  hash TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL CHECK (role IN ('readonly', 'contributor', 'readwrite', 'agent', 'admin')),
  scopes TEXT[] NOT NULL DEFAULT ARRAY['/']::TEXT[],
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_by TEXT,
  expires_at TIMESTAMPTZ,
  revoked_at TIMESTAMPTZ,
  revoked_by TEXT
);

CREATE INDEX IF NOT EXISTS auth_tokens_hash_idx ON longhouse.auth_tokens(hash);
CREATE INDEX IF NOT EXISTS auth_tokens_role_idx ON longhouse.auth_tokens(role);
CREATE INDEX IF NOT EXISTS auth_tokens_active_idx ON longhouse.auth_tokens(created_at DESC) WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS longhouse.auth_invites (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  hash TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL CHECK (role IN ('readonly', 'contributor', 'readwrite', 'agent', 'admin')),
  scopes TEXT[] NOT NULL DEFAULT ARRAY['/']::TEXT[],
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_by TEXT,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  consumed_by_token_id TEXT REFERENCES longhouse.auth_tokens(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS auth_invites_hash_idx ON longhouse.auth_invites(hash);
CREATE INDEX IF NOT EXISTS auth_invites_active_idx ON longhouse.auth_invites(created_at DESC) WHERE consumed_at IS NULL;

COMMIT;
