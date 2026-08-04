-- Ocean Longhouse core metadata schema
-- Target: PostgreSQL 15+.
-- Optional: pgvector for local vector search fallback.
-- Apply intentionally after confirming the target Railway Postgres database.

BEGIN;

CREATE SCHEMA IF NOT EXISTS longhouse;

-- Requires gen_random_uuid() to be available on the target Postgres.
-- Railway Postgres exposes it by default; installing extensions is intentionally left to a human operator.

-- Optional. If unavailable, skip this line and use Cloudflare Vectorize as the vector serving index.
-- CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS longhouse.mounts (
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL CHECK (type IN ('local', 'r2', 's3', 'github', 'webdav', 'sftp', 'remote-longhouse')),
  prefix TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'readwrite' CHECK (mode IN ('readonly', 'readwrite', 'writeonly')),
  priority INTEGER NOT NULL DEFAULT 100,
  config JSONB NOT NULL DEFAULT '{}'::jsonb,
  capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS mounts_prefix_priority_idx
  ON longhouse.mounts(prefix, priority DESC);

CREATE TABLE IF NOT EXISTS longhouse.objects (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  virtual_path TEXT NOT NULL UNIQUE,
  mount_id TEXT NOT NULL REFERENCES longhouse.mounts(id),
  backend_key TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('file', 'directory', 'object', 'external')),
  content_type TEXT,
  size_bytes BIGINT,
  sha256 TEXT,
  etag TEXT,
  version_ref TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
  canonical BOOLEAN NOT NULL DEFAULT TRUE,
  deleted_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS objects_mount_idx ON longhouse.objects(mount_id);
CREATE INDEX IF NOT EXISTS objects_path_prefix_idx ON longhouse.objects USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS objects_metadata_gin_idx ON longhouse.objects USING gin(metadata);
CREATE INDEX IF NOT EXISTS objects_tags_gin_idx ON longhouse.objects USING gin(tags);

CREATE TABLE IF NOT EXISTS longhouse.object_replicas (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  object_id UUID NOT NULL REFERENCES longhouse.objects(id) ON DELETE CASCADE,
  mount_id TEXT NOT NULL REFERENCES longhouse.mounts(id),
  backend_key TEXT NOT NULL,
  sha256 TEXT,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'current', 'stale', 'failed', 'deleted')),
  last_synced_at TIMESTAMPTZ,
  error TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(object_id, mount_id, backend_key)
);

CREATE TABLE IF NOT EXISTS longhouse.sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL,
  owner_token_id TEXT,
  owner_name TEXT,
  root_path TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'completed', 'archived')),
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS sessions_root_path_idx ON longhouse.sessions(root_path);

CREATE TABLE IF NOT EXISTS longhouse.graph_nodes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  node_type TEXT NOT NULL,
  name TEXT,
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  external_ref TEXT,
  properties JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(node_type, external_ref)
);

CREATE INDEX IF NOT EXISTS graph_nodes_type_idx ON longhouse.graph_nodes(node_type);
CREATE INDEX IF NOT EXISTS graph_nodes_virtual_path_idx ON longhouse.graph_nodes(virtual_path);
CREATE INDEX IF NOT EXISTS graph_nodes_properties_gin_idx ON longhouse.graph_nodes USING gin(properties);

CREATE TABLE IF NOT EXISTS longhouse.graph_edges (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  from_node_id UUID NOT NULL REFERENCES longhouse.graph_nodes(id) ON DELETE CASCADE,
  to_node_id UUID NOT NULL REFERENCES longhouse.graph_nodes(id) ON DELETE CASCADE,
  predicate TEXT NOT NULL,
  weight DOUBLE PRECISION NOT NULL DEFAULT 1.0,
  source_object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  properties JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(from_node_id, to_node_id, predicate)
);

CREATE INDEX IF NOT EXISTS graph_edges_from_idx ON longhouse.graph_edges(from_node_id, predicate);
CREATE INDEX IF NOT EXISTS graph_edges_to_idx ON longhouse.graph_edges(to_node_id, predicate);
CREATE INDEX IF NOT EXISTS graph_edges_properties_gin_idx ON longhouse.graph_edges USING gin(properties);

CREATE TABLE IF NOT EXISTS longhouse.embedding_chunks (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE CASCADE,
  virtual_path TEXT NOT NULL,
  chunk_index INTEGER NOT NULL,
  chunk_text TEXT NOT NULL,
  chunk_sha256 TEXT NOT NULL,
  token_count INTEGER,
  embedding_provider TEXT NOT NULL DEFAULT 'cloudflare',
  embedding_model TEXT NOT NULL,
  dimensions INTEGER NOT NULL,
  vectorize_index TEXT,
  vectorize_id TEXT,
  -- If pgvector is enabled, add a vector column in a follow-up migration sized to the chosen model.
  -- Example for Cloudflare bge-base-en-v1.5 / 768 dims:
  -- embedding vector(768),
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(object_id, chunk_index, embedding_model)
);

CREATE INDEX IF NOT EXISTS embedding_chunks_object_idx ON longhouse.embedding_chunks(object_id);
CREATE INDEX IF NOT EXISTS embedding_chunks_path_idx ON longhouse.embedding_chunks USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS embedding_chunks_sha_idx ON longhouse.embedding_chunks(chunk_sha256);
CREATE INDEX IF NOT EXISTS embedding_chunks_vectorize_idx ON longhouse.embedding_chunks(vectorize_index, vectorize_id);

CREATE TABLE IF NOT EXISTS longhouse.kv_cache (
  namespace TEXT NOT NULL,
  key TEXT NOT NULL,
  value JSONB NOT NULL,
  etag TEXT,
  expires_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY(namespace, key)
);

CREATE INDEX IF NOT EXISTS kv_cache_expires_idx ON longhouse.kv_cache(expires_at);

CREATE TABLE IF NOT EXISTS longhouse.ingest_jobs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  job_type TEXT NOT NULL CHECK (job_type IN ('index_object', 'embed_object', 'sync_replica', 'extract_graph', 'cache_refresh')),
  status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'failed', 'cancelled')),
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  payload JSONB NOT NULL DEFAULT '{}'::jsonb,
  attempts INTEGER NOT NULL DEFAULT 0,
  max_attempts INTEGER NOT NULL DEFAULT 5,
  run_after TIMESTAMPTZ NOT NULL DEFAULT now(),
  locked_by TEXT,
  locked_at TIMESTAMPTZ,
  error TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS ingest_jobs_ready_idx
  ON longhouse.ingest_jobs(status, run_after, created_at)
  WHERE status = 'queued';

CREATE TABLE IF NOT EXISTS longhouse.audit_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  ts TIMESTAMPTZ NOT NULL DEFAULT now(),
  actor_token_id TEXT,
  actor_name TEXT,
  action TEXT NOT NULL,
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  mount_id TEXT REFERENCES longhouse.mounts(id) ON DELETE SET NULL,
  details JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS audit_events_ts_idx ON longhouse.audit_events(ts DESC);
CREATE INDEX IF NOT EXISTS audit_events_path_idx ON longhouse.audit_events USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS audit_events_action_idx ON longhouse.audit_events(action, ts DESC);

INSERT INTO longhouse.mounts (id, type, prefix, mode, priority, config, capabilities)
VALUES
  ('managed-local', 'local', '/', 'readwrite', 100, '{"root":"data/files"}', '{"read":true,"write":true,"delete":true,"move":true}'),
  ('r2-content-posting-lab', 'r2', '/assets/content-posting-lab', 'readwrite', 80, '{"bucket":"content-posting-lab"}', '{"read":true,"write":true,"delete":true,"signedUrls":true}'),
  ('r2-rt-html-previews', 'r2', '/previews/html', 'readwrite', 80, '{"bucket":"rt-html-previews"}', '{"read":true,"write":true,"delete":true,"signedUrls":true}'),
  ('r2-pi-video-gallery', 'r2', '/assets/video-gallery', 'readwrite', 80, '{"bucket":"pi-video-gallery"}', '{"read":true,"write":true,"delete":true,"signedUrls":true}'),
  ('r2-sales-pitch-pdfs', 'r2', '/sales/pitch-pdfs', 'readwrite', 80, '{"bucket":"rt-sales-pitch-pdfs"}', '{"read":true,"write":true,"delete":true,"signedUrls":true}'),
  ('r2-financial-invoices', 'r2', '/finance/invoices', 'readonly', 80, '{"bucket":"financial-dashboard-invoices"}', '{"read":true,"write":false,"delete":false,"signedUrls":true}'),
  ('r2-pi-backups', 'r2', '/archive/pi-backups', 'readonly', 60, '{"bucket":"pi-backups"}', '{"read":true,"write":false,"delete":false,"signedUrls":true}')
ON CONFLICT (id) DO NOTHING;

COMMIT;
