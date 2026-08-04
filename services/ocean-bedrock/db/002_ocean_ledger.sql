-- Ocean Ledger: append-only temporal context ledger for Ocean Longhouse.
-- Target: PostgreSQL 15+.
-- Apply after db/001_longhouse_core.sql.

BEGIN;

CREATE SCHEMA IF NOT EXISTS longhouse;

-- Requires gen_random_uuid() to be available on the target Postgres.
-- Railway Postgres exposes it by default; installing extensions is intentionally left to a human operator.

CREATE TABLE IF NOT EXISTS longhouse.ledger_events (
  id TEXT PRIMARY KEY,
  schema_version INTEGER NOT NULL DEFAULT 1,
  sequence BIGINT NOT NULL UNIQUE,
  event_type TEXT NOT NULL,
  correlation_id TEXT,
  lab TEXT NOT NULL DEFAULT 'longhouse',
  actor_type TEXT NOT NULL CHECK (actor_type IN ('user', 'agent', 'system', 'service', 'adapter')),
  actor_id TEXT,
  actor_name TEXT,
  source_id TEXT NOT NULL DEFAULT 'longhouse',
  source_sequence TEXT,
  source_ref TEXT,
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  payload JSONB NOT NULL DEFAULT '{}'::jsonb,
  visible_context JSONB,
  context_snapshot JSONB,
  clearance TEXT NOT NULL DEFAULT 'UNCLASSIFIED' CHECK (clearance IN ('PUBLIC', 'UNCLASSIFIED', 'CONFIDENTIAL', 'SECRET', 'TOP_SECRET')),
  tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
  event_timestamp TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  prev_hash TEXT,
  hash TEXT NOT NULL UNIQUE,
  inserted_by TEXT,
  adapter_version TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS ledger_events_source_dedupe_idx
  ON longhouse.ledger_events(source_id, source_sequence)
  WHERE source_sequence IS NOT NULL;

CREATE INDEX IF NOT EXISTS ledger_events_correlation_idx ON longhouse.ledger_events(correlation_id, sequence);
CREATE INDEX IF NOT EXISTS ledger_events_actor_idx ON longhouse.ledger_events(actor_type, actor_id, sequence);
CREATE INDEX IF NOT EXISTS ledger_events_type_idx ON longhouse.ledger_events(event_type, sequence);
CREATE INDEX IF NOT EXISTS ledger_events_time_idx ON longhouse.ledger_events(event_timestamp DESC);
CREATE INDEX IF NOT EXISTS ledger_events_path_idx ON longhouse.ledger_events USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS ledger_events_payload_gin_idx ON longhouse.ledger_events USING gin(payload);
CREATE INDEX IF NOT EXISTS ledger_events_context_gin_idx ON longhouse.ledger_events USING gin(context_snapshot);
CREATE INDEX IF NOT EXISTS ledger_events_tags_gin_idx ON longhouse.ledger_events USING gin(tags);

CREATE TABLE IF NOT EXISTS longhouse.ledger_correlations (
  id TEXT PRIMARY KEY,
  title TEXT,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'resolved', 'archived')),
  root_event_id TEXT REFERENCES longhouse.ledger_events(id) ON DELETE SET NULL,
  summary TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS ledger_correlations_metadata_gin_idx ON longhouse.ledger_correlations USING gin(metadata);

CREATE TABLE IF NOT EXISTS longhouse.context_snapshots (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT,
  snapshot_type TEXT NOT NULL DEFAULT 'agent_context',
  correlation_id TEXT REFERENCES longhouse.ledger_correlations(id) ON DELETE SET NULL,
  root_event_id TEXT REFERENCES longhouse.ledger_events(id) ON DELETE SET NULL,
  start_sequence BIGINT,
  end_sequence BIGINT,
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  actor_type TEXT CHECK (actor_type IN ('user', 'agent', 'system', 'service', 'adapter')),
  actor_id TEXT,
  actor_name TEXT,
  manifest JSONB NOT NULL DEFAULT '{}'::jsonb,
  summary TEXT,
  sha256 TEXT,
  clearance TEXT NOT NULL DEFAULT 'UNCLASSIFIED' CHECK (clearance IN ('PUBLIC', 'UNCLASSIFIED', 'CONFIDENTIAL', 'SECRET', 'TOP_SECRET')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS context_snapshots_correlation_idx ON longhouse.context_snapshots(correlation_id, created_at DESC);
CREATE INDEX IF NOT EXISTS context_snapshots_actor_idx ON longhouse.context_snapshots(actor_type, actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS context_snapshots_path_idx ON longhouse.context_snapshots USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS context_snapshots_manifest_gin_idx ON longhouse.context_snapshots USING gin(manifest);

CREATE TABLE IF NOT EXISTS longhouse.ledger_source_cursors (
  source_id TEXT PRIMARY KEY,
  source_type TEXT NOT NULL,
  cursor_value TEXT,
  last_event_timestamp TIMESTAMPTZ,
  last_ingested_at TIMESTAMPTZ,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'failed')),
  error TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION longhouse.prevent_ledger_event_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  RAISE EXCEPTION 'Ocean Ledger events are append-only. Add a correcting event instead.';
END;
$$;

DROP TRIGGER IF EXISTS ledger_events_no_update ON longhouse.ledger_events;
CREATE TRIGGER ledger_events_no_update
  BEFORE UPDATE ON longhouse.ledger_events
  FOR EACH ROW EXECUTE FUNCTION longhouse.prevent_ledger_event_mutation();

DROP TRIGGER IF EXISTS ledger_events_no_delete ON longhouse.ledger_events;
CREATE TRIGGER ledger_events_no_delete
  BEFORE DELETE ON longhouse.ledger_events
  FOR EACH ROW EXECUTE FUNCTION longhouse.prevent_ledger_event_mutation();

CREATE OR REPLACE VIEW longhouse.ledger_trace AS
SELECT
  correlation_id,
  sequence,
  id,
  event_type,
  lab,
  actor_type,
  actor_id,
  actor_name,
  source_id,
  source_ref,
  virtual_path,
  clearance,
  event_timestamp,
  payload
FROM longhouse.ledger_events
WHERE correlation_id IS NOT NULL
ORDER BY correlation_id, sequence;

COMMIT;
