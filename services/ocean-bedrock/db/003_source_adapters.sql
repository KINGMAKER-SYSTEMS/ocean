-- Ocean Bedrock source adapter registry.
-- Target: PostgreSQL 15+.
-- Apply after db/001_longhouse_core.sql and db/002_ocean_ledger.sql.

BEGIN;

CREATE SCHEMA IF NOT EXISTS longhouse;

CREATE TABLE IF NOT EXISTS longhouse.source_adapters (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  description TEXT,
  config_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
  stream_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
  capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS longhouse.source_instances (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  adapter_id TEXT NOT NULL REFERENCES longhouse.source_adapters(id),
  name TEXT NOT NULL,
  owner_name TEXT,
  owner_token_id TEXT,
  remote_prefix TEXT NOT NULL,
  config JSONB NOT NULL DEFAULT '{}'::jsonb,
  secret_ref TEXT,
  clearance TEXT NOT NULL DEFAULT 'CONFIDENTIAL' CHECK (clearance IN ('PUBLIC', 'UNCLASSIFIED', 'CONFIDENTIAL', 'SECRET', 'TOP_SECRET')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'failed', 'archived')),
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT source_instances_unique_name_adapter UNIQUE(adapter_id, name)
);

CREATE INDEX IF NOT EXISTS source_instances_adapter_idx ON longhouse.source_instances(adapter_id);
CREATE INDEX IF NOT EXISTS source_instances_owner_idx ON longhouse.source_instances(owner_name);
CREATE INDEX IF NOT EXISTS source_instances_prefix_idx ON longhouse.source_instances USING btree (remote_prefix text_pattern_ops);
CREATE INDEX IF NOT EXISTS source_instances_config_gin_idx ON longhouse.source_instances USING gin(config);

CREATE TABLE IF NOT EXISTS longhouse.source_streams (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_instance_id UUID NOT NULL REFERENCES longhouse.source_instances(id) ON DELETE CASCADE,
  stream_key TEXT NOT NULL,
  stream_type TEXT NOT NULL,
  remote_prefix TEXT NOT NULL,
  selection JSONB NOT NULL DEFAULT '{}'::jsonb,
  cursor JSONB NOT NULL DEFAULT '{}'::jsonb,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(source_instance_id, stream_key)
);

CREATE INDEX IF NOT EXISTS source_streams_instance_idx ON longhouse.source_streams(source_instance_id);
CREATE INDEX IF NOT EXISTS source_streams_type_idx ON longhouse.source_streams(stream_type);
CREATE INDEX IF NOT EXISTS source_streams_prefix_idx ON longhouse.source_streams USING btree (remote_prefix text_pattern_ops);
CREATE INDEX IF NOT EXISTS source_streams_selection_gin_idx ON longhouse.source_streams USING gin(selection);
CREATE INDEX IF NOT EXISTS source_streams_cursor_gin_idx ON longhouse.source_streams USING gin(cursor);

CREATE TABLE IF NOT EXISTS longhouse.source_sync_runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_instance_id UUID NOT NULL REFERENCES longhouse.source_instances(id) ON DELETE CASCADE,
  stream_id UUID REFERENCES longhouse.source_streams(id) ON DELETE SET NULL,
  status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed', 'cancelled')),
  started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at TIMESTAMPTZ,
  scanned_count INTEGER NOT NULL DEFAULT 0,
  changed_count INTEGER NOT NULL DEFAULT 0,
  uploaded_count INTEGER NOT NULL DEFAULT 0,
  skipped_count INTEGER NOT NULL DEFAULT 0,
  error_count INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  manifest_path TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS source_sync_runs_instance_idx ON longhouse.source_sync_runs(source_instance_id, started_at DESC);
CREATE INDEX IF NOT EXISTS source_sync_runs_stream_idx ON longhouse.source_sync_runs(stream_id, started_at DESC);
CREATE INDEX IF NOT EXISTS source_sync_runs_status_idx ON longhouse.source_sync_runs(status, started_at DESC);
CREATE INDEX IF NOT EXISTS source_sync_runs_manifest_idx ON longhouse.source_sync_runs USING btree (manifest_path text_pattern_ops);

CREATE TABLE IF NOT EXISTS longhouse.source_records (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_instance_id UUID NOT NULL REFERENCES longhouse.source_instances(id) ON DELETE CASCADE,
  stream_id UUID REFERENCES longhouse.source_streams(id) ON DELETE SET NULL,
  source_record_id TEXT NOT NULL,
  virtual_path TEXT,
  object_id UUID REFERENCES longhouse.objects(id) ON DELETE SET NULL,
  source_updated_at TIMESTAMPTZ,
  content_sha256 TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  deleted_at TIMESTAMPTZ,
  seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(source_instance_id, source_record_id)
);

CREATE INDEX IF NOT EXISTS source_records_instance_idx ON longhouse.source_records(source_instance_id);
CREATE INDEX IF NOT EXISTS source_records_stream_idx ON longhouse.source_records(stream_id);
CREATE INDEX IF NOT EXISTS source_records_path_idx ON longhouse.source_records USING btree (virtual_path text_pattern_ops);
CREATE INDEX IF NOT EXISTS source_records_object_idx ON longhouse.source_records(object_id);
CREATE INDEX IF NOT EXISTS source_records_sha_idx ON longhouse.source_records(content_sha256);
CREATE INDEX IF NOT EXISTS source_records_metadata_gin_idx ON longhouse.source_records USING gin(metadata);

INSERT INTO longhouse.source_adapters (id, display_name, description, config_schema, stream_schema, capabilities)
VALUES
  (
    'local_folder',
    'Local Folder',
    'Opt-in local directory tree selected by a coworker or operator.',
    '{"type":"object","properties":{"device":{"type":"string"},"local_path_label":{"type":"string"},"include_extensions":{"type":"array","items":{"type":"string"}},"ignore":{"type":"array","items":{"type":"string"}},"max_file_bytes":{"type":"integer"}},"required":["device"]}'::jsonb,
    '{"type":"object","properties":{"local_path":{"type":"string"},"remote_prefix":{"type":"string"},"recursive":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":false,"delete_detection":true,"binary_files":true,"text_events":true}'::jsonb
  ),
  (
    'github',
    'GitHub Repository',
    'Repository contents, issues, pull requests, releases, and webhook events.',
    '{"type":"object","properties":{"owner":{"type":"string"},"repo":{"type":"string"},"branch":{"type":"string","default":"main"},"paths":{"type":"array","items":{"type":"string"}}},"required":["owner","repo"]}'::jsonb,
    '{"type":"object","properties":{"path":{"type":"string"},"resource_type":{"type":"string","enum":["contents","issues","pulls","releases"]},"recursive":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":true,"delete_detection":true,"binary_files":true,"text_events":true}'::jsonb
  ),
  (
    'telegram',
    'Telegram Chat',
    'Telegram chat messages, media metadata, and bot webhook events.',
    '{"type":"object","properties":{"chat_id":{"type":"string"},"chat_type":{"type":"string","enum":["group","channel","private"]}},"required":["chat_id"]}'::jsonb,
    '{"type":"object","properties":{"message_types":{"type":"array","items":{"type":"string"}},"include_media":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":false,"incremental":true,"webhook":true,"delete_detection":false,"binary_files":true,"text_events":true}'::jsonb
  ),
  (
    'slack',
    'Slack Workspace',
    'Slack channels, threads, files, and decision/event messages.',
    '{"type":"object","properties":{"workspace":{"type":"string"},"channel_id":{"type":"string"},"capture_threads":{"type":"boolean","default":true},"decision_keywords":{"type":"array","items":{"type":"string"}}},"required":["workspace","channel_id"]}'::jsonb,
    '{"type":"object","properties":{"channel":{"type":"string"},"include_threads":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":true,"delete_detection":false,"binary_files":true,"text_events":true}'::jsonb
  ),
  (
    'notion',
    'Notion Workspace',
    'Notion pages and databases selected for internal knowledge sync.',
    '{"type":"object","properties":{"workspace_id":{"type":"string"},"page_ids":{"type":"array","items":{"type":"string"}},"database_ids":{"type":"array","items":{"type":"string"}}},"required":["workspace_id"]}'::jsonb,
    '{"type":"object","properties":{"object_type":{"type":"string","enum":["page","database"]},"object_id":{"type":"string"}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":true,"delete_detection":true,"binary_files":false,"text_events":true}'::jsonb
  ),
  (
    'linear',
    'Linear Workspace',
    'Linear issues, projects, cycles, and workspace planning objects.',
    '{"type":"object","properties":{"team_id":{"type":"string"},"project_id":{"type":"string"},"include_archived":{"type":"boolean","default":false}},"required":["team_id"]}'::jsonb,
    '{"type":"object","properties":{"resource_type":{"type":"string","enum":["issues","projects","cycles"]}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":true,"delete_detection":true,"binary_files":false,"text_events":true}'::jsonb
  ),
  (
    'google_drive',
    'Google Drive',
    'Google Drive folders, files, and shared-drive documents.',
    '{"type":"object","properties":{"root_folder_id":{"type":"string"},"include_shared_drives":{"type":"boolean","default":false},"mime_types":{"type":"array","items":{"type":"string"}}},"required":["root_folder_id"]}'::jsonb,
    '{"type":"object","properties":{"folder_id":{"type":"string"},"recursive":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":true,"delete_detection":true,"binary_files":true,"text_events":true}'::jsonb
  ),
  (
    'r2',
    'Cloudflare R2 Bucket',
    'Cloudflare R2 object prefixes for canonical byte/object sync.',
    '{"type":"object","properties":{"bucket":{"type":"string"},"prefix":{"type":"string","default":""},"endpoint":{"type":"string"}},"required":["bucket"]}'::jsonb,
    '{"type":"object","properties":{"prefix":{"type":"string"},"include_subdirectories":{"type":"boolean","default":true}}}'::jsonb,
    '{"snapshot":true,"incremental":true,"webhook":false,"delete_detection":true,"binary_files":true,"text_events":false}'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  display_name = EXCLUDED.display_name,
  description = EXCLUDED.description,
  config_schema = EXCLUDED.config_schema,
  stream_schema = EXCLUDED.stream_schema,
  capabilities = EXCLUDED.capabilities,
  enabled = TRUE,
  updated_at = now();

COMMIT;
