-- Ocean Bedrock source adapter capability truth reconciliation.
-- Target: PostgreSQL 15+.
-- Apply after db/003_source_adapters.sql.
--
-- Capability flags are a contract. Keep webhook/incremental/delete_detection false
-- unless there is implemented code behind the behavior.

BEGIN;

CREATE SCHEMA IF NOT EXISTS longhouse;

UPDATE longhouse.source_adapters
SET capabilities = CASE id
  WHEN 'local_folder' THEN '{"snapshot":true,"incremental":true,"webhook":false,"delete_detection":true,"binary_files":true,"text_events":true}'::jsonb
  WHEN 'github' THEN '{"snapshot":true,"incremental":true,"webhook":false,"delete_detection":true,"binary_files":true,"text_events":true}'::jsonb
  WHEN 'telegram' THEN '{"snapshot":false,"incremental":true,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":true}'::jsonb
  WHEN 'slack' THEN '{"snapshot":false,"incremental":false,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":false}'::jsonb
  WHEN 'notion' THEN '{"snapshot":false,"incremental":false,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":false}'::jsonb
  WHEN 'linear' THEN '{"snapshot":false,"incremental":false,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":false}'::jsonb
  WHEN 'google_drive' THEN '{"snapshot":false,"incremental":false,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":false}'::jsonb
  WHEN 'r2' THEN '{"snapshot":false,"incremental":false,"webhook":false,"delete_detection":false,"binary_files":false,"text_events":false}'::jsonb
  ELSE capabilities
END,
updated_at = now()
WHERE id IN ('local_folder', 'github', 'telegram', 'slack', 'notion', 'linear', 'google_drive', 'r2');

COMMIT;
