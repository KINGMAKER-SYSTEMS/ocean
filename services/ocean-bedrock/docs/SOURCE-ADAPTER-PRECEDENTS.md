# Source Adapter Table/Config Precedents

Research date: 2026-06-11

Purpose: define a grounded precedent for Ocean Bedrock source adapters: local folders, GitHub, Slack/Telegram, Notion, Linear, Google Drive, R2, etc.

## Executive summary

The common precedent across ETL, CDC, and RAG systems is:

```txt
adapter definition          = code-level connector type and config schema
source instance             = one configured source account/folder/repo/workspace
stream/resource catalog     = what the source can produce
cursor/state/offset         = where sync resumes
sync run/job table          = execution history and errors
raw/normalized records      = outputs with lineage metadata
secret references           = pointers to secrets, not raw secrets in config rows
```

For Ocean Bedrock, use a **database-backed source registry** plus small config manifests:

```txt
source_adapters      # adapter type definitions: local_folder, github, slack, notion, linear...
source_instances     # configured source: Alice laptop Documents, GitHub repo, Slack workspace...
source_streams       # folders/channels/repos/tables/pages selected for ingest
source_cursors       # per-stream state/cursor/offset/checkpoint
source_sync_runs     # run history and errors
source_records       # optional raw source object catalog / dedupe manifest
```

This mirrors established systems while fitting Ocean’s token/path/clearance model.

## Precedents

### 1. Airbyte — connector spec + catalog + state

Airbyte separates connector definition from configured source and stream catalog.

Relevant precedent:

- Low-code connectors use a YAML manifest with `streams`, `check`, and `spec` sections.
- The `spec` defines the connector configuration schema.
- A configured catalog defines which streams/tables/collections are selected.

Sources:

- https://docs.airbyte.com/platform/connector-development/config-based/low-code-cdk-overview
- https://github.com/airbytehq/airbyte/blob/master/docs/connector-development/config-based/tutorial/3-connecting-to-the-API-source.md
- https://docs.airbyte.com/understanding-airbyte/database-data-catalog

Ocean lesson:

```txt
source_adapters.config_schema    ~= Airbyte connector spec
source_streams.resource_schema   ~= Airbyte catalog stream schema
source_cursors.state             ~= Airbyte state/checkpoint
```

### 2. Singer / Meltano — config + catalog + state files

Singer taps establish a very clear adapter contract:

```txt
--config   required source configuration
--catalog  optional stream/field selection
--state    optional resume checkpoint
```

Meltano wraps Singer with plugin definitions, capabilities, settings, and sensitive config handling.

Sources:

- https://hub.meltano.com/singer/spec/
- https://github.com/singer-io/getting-started/blob/master/docs/SPEC.md
- https://docs.meltano.com/guide/configuration/
- https://docs.meltano.com/tutorials/custom-extractor/

Ocean lesson:

Use a Singer-like shape for every adapter:

```json
{
  "adapter": "github",
  "config": { "repo": "Risingtides-dev/ocean-os" },
  "catalog": { "streams": ["issues", "pulls", "contents"] },
  "state": { "last_seen": "..." }
}
```

But store this in Postgres so agents/coworkers can manage sources centrally.

### 3. Kafka Connect / Debezium — connector config + offsets + schema history

Kafka Connect and Debezium are strong precedent for durable source state:

- connector/task configuration is stored separately from runtime offsets,
- source offsets let connectors resume after restart,
- schema history is separate from the data stream,
- snapshots and incremental streams are explicit phases.

Sources:

- https://docs.aws.amazon.com/msk/latest/developerguide/msk-connect-kafka-connect-topics.html
- https://docs.aws.amazon.com/msk/latest/developerguide/msk-connect-manage-connector-offsets.html
- https://debezium.io/documentation/reference/stable/configuration/storage.html
- https://debezium.io/documentation/reference/stable/connectors/postgresql.html

Ocean lesson:

Do not just track “last sync time.” Track source-specific cursors:

```txt
local_folder: file path + sha256 + mtime
slack: channel_id + latest_ts
telegram: chat_id + update_id/message_id
github: repo + branch + commit SHA / event id
notion: page/database id + last_edited_time
linear: team_id/project_id + updatedAt cursor
```

### 4. Dagster — resources/config as external-system handles

Dagster’s resource model is precedent for representing external systems as configurable handles shared by jobs/assets.

Sources:

- https://docs.dagster.io/guides/build/external-resources
- https://docs.dagster.io/guides/build/external-resources/configuring-resources
- https://docs.dagster.io/guides/operate/configuration/run-configuration

Ocean lesson:

Keep adapter credentials and clients separate from specific ingest jobs. Jobs reference a source instance; they do not carry raw auth.

### 5. LangChain / LlamaIndex — loaders/readers normalize to Document + metadata

RAG systems use loaders/connectors/readers that convert many source types into a common document object with metadata.

Sources:

- https://reference.langchain.com/python/langchain-community/document_loaders
- https://developers.llamaindex.ai/python/framework/understanding/rag/loading/
- https://docs.llamaindex.ai/en/v0.12.15/understanding/loading/loading/

Ocean lesson:

Every source adapter should emit a normalized Bedrock document packet:

```json
{
  "source_instance_id": "...",
  "source_record_id": "slack:C123:171234.000100",
  "virtual_path": "/context/slack/payments/2026-06-11/message.md",
  "content": "...",
  "content_type": "text/markdown",
  "metadata": {
    "source_type": "slack",
    "channel": "#payments",
    "author": "...",
    "created_at": "...",
    "external_url": "..."
  }
}
```

### 6. Unstructured — source connectors + standardized metadata

Unstructured’s ingestion model is good precedent for unstructured document processing:

- source connectors pull from local/remote locations,
- ingestion config controls batching/processing,
- connector-derived metadata is added to processed documents.

Sources:

- https://docs.unstructured.io/open-source/ingestion/overview
- https://unstructured.readthedocs.io/en/main/metadata.html
- https://unstructured.io/insights/using-data-connectors-for-efficient-multi-source-ingestion

Ocean lesson:

Separate source intake from extraction/chunking/embedding. Source adapters fetch bytes/events; processing pipelines normalize, chunk, embed, and graph them.

## Recommended Ocean Bedrock schema

### `longhouse.source_adapters`

Defines adapter types and their config contract.

```sql
CREATE TABLE longhouse.source_adapters (
  id TEXT PRIMARY KEY,                       -- local_folder, github, slack, notion, linear
  display_name TEXT NOT NULL,
  description TEXT,
  config_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
  stream_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
  capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Example capabilities:

```json
{
  "snapshot": true,
  "incremental": true,
  "webhook": false,
  "delete_detection": false,
  "binary_files": true,
  "text_events": true
}
```

### `longhouse.source_instances`

One configured source account/folder/repo/workspace.

```sql
CREATE TABLE longhouse.source_instances (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  adapter_id TEXT NOT NULL REFERENCES longhouse.source_adapters(id),
  name TEXT NOT NULL,
  owner_name TEXT,
  owner_token_id TEXT,
  remote_prefix TEXT NOT NULL,
  config JSONB NOT NULL DEFAULT '{}'::jsonb,
  secret_ref TEXT,
  clearance TEXT NOT NULL DEFAULT 'CONFIDENTIAL',
  status TEXT NOT NULL DEFAULT 'active',
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Rules:

- `config` may contain non-secret identifiers: repo, folder label, channel id.
- `secret_ref` points to encrypted/managed credentials.
- raw access tokens do not belong here.

### `longhouse.source_streams`

Selected streams/resources inside a source.

```sql
CREATE TABLE longhouse.source_streams (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_instance_id UUID NOT NULL REFERENCES longhouse.source_instances(id) ON DELETE CASCADE,
  stream_key TEXT NOT NULL,                  -- folder path, channel id, repo path, database id
  stream_type TEXT NOT NULL,                 -- folder, channel, repo_contents, issues, pages
  remote_prefix TEXT NOT NULL,
  selection JSONB NOT NULL DEFAULT '{}'::jsonb,
  cursor JSONB NOT NULL DEFAULT '{}'::jsonb,
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(source_instance_id, stream_key)
);
```

### `longhouse.source_sync_runs`

Run history.

```sql
CREATE TABLE longhouse.source_sync_runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source_instance_id UUID NOT NULL REFERENCES longhouse.source_instances(id) ON DELETE CASCADE,
  stream_id UUID REFERENCES longhouse.source_streams(id) ON DELETE SET NULL,
  status TEXT NOT NULL DEFAULT 'running',
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
```

### `longhouse.source_records`

Optional dedupe/catalog table for raw source objects.

```sql
CREATE TABLE longhouse.source_records (
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
```

## Adapter config examples

### Local folder

```json
{
  "adapter_id": "local_folder",
  "name": "Alice Documents",
  "remote_prefix": "/coworkers/alice/macbook/documents",
  "config": {
    "device": "alice-macbook",
    "local_path_label": "Documents",
    "include_extensions": [".md", ".txt", ".pdf", ".docx"],
    "ignore": [".git", "node_modules", ".venv", ".cache"],
    "max_file_bytes": 10485760
  }
}
```

### GitHub repo

```json
{
  "adapter_id": "github",
  "name": "ocean-os repo",
  "remote_prefix": "/github/Risingtides-dev/ocean-os",
  "secret_ref": "github:ocean-bedrock-app",
  "config": {
    "owner": "Risingtides-dev",
    "repo": "ocean-os",
    "branch": "main",
    "paths": ["docs", "crates/ocean-longhouse"]
  }
}
```

### Slack channel

```json
{
  "adapter_id": "slack",
  "name": "payments-dev channel",
  "remote_prefix": "/communications/slack/payments-dev",
  "secret_ref": "slack:rising-tides-bot",
  "config": {
    "workspace": "rising-tides",
    "channel_id": "C123456",
    "capture_threads": true,
    "decision_keywords": ["decided", "ship", "blocker", "approved"]
  }
}
```

## Recommended event/tag model

Every adapter should emit Ocean Ledger events with a consistent payload envelope:

```json
{
  "event_type": "source.record.ingested",
  "correlation_id": "cor-source-...",
  "lab": "ocean-context",
  "source_id": "github:ocean-os",
  "source_sequence": "commit-sha-or-message-ts",
  "virtual_path": "/github/Risingtides-dev/ocean-os/docs/LONGHOUSE.md",
  "payload": {
    "adapter_id": "github",
    "source_instance_id": "...",
    "source_record_id": "...",
    "tags": ["github", "docs", "ocean-os"]
  },
  "clearance": "CONFIDENTIAL",
  "tags": ["source-ingest", "github", "ocean-bedrock"]
}
```

## Design guidance for Ocean

1. Use **source adapter tables** for operational state, not just static JSON files.
2. Keep static config export/import possible for local bootstrap and GitOps.
3. Store secrets as `secret_ref`, never raw tokens in shared manifests.
4. Track source-specific cursors; do not rely only on timestamps.
5. Normalize every source into common object/document/chunk/ledger shapes.
6. Preserve lineage: source instance → stream → record → object → chunks → graph.
7. Treat Vectorize/KV as rebuildable indexes; Postgres/source bytes are authoritative.
8. Support delete detection later, but do not enable destructive remote mirroring by default.

## V1 implementation recommendation

Add the tables in a new migration:

```txt
db/003_source_adapters.sql
```

Seed adapters:

```txt
local_folder
github
telegram
slack
notion
linear
google_drive
r2
```

Immediately wire `ocean-bootstrap` and `ocean-ingest-local` to create:

```txt
source_instance: local_folder
source_stream: selected local folder
source_sync_run: each ingest run
source_record: each uploaded file
```

Then build GitHub and Telegram adapters next because they are high-value and have clear incremental cursors.
