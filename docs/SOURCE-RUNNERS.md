# Ocean Bedrock Source Runners

Status: first server-side runner wave.

Ocean Bedrock source runners are cloud-box substrate code. They fetch remote bytes/events, write normalized objects through the Bedrock storage router, and record source lineage. They do **not** redefine Ocean Longhouse or bypass daemon/runtime permissions.

## Source of truth

- Source registry tables: Postgres `longhouse.source_*` tables.
- Incremental checkpoints: `longhouse.ledger_source_cursors` is authoritative.
- `longhouse.source_streams.cursor` is only a convenience mirror for operators/UI.
- Secrets: `source_instances.secret_ref` stores a pointer only, currently `env:VAR_NAME`.

Raw API tokens must never be stored in `source_instances.config`, manifests, source record metadata, ledger payloads, or docs.

## Runner contract

A server-side runner implements:

```js
class SourceRunner {
  async plan(ctx) {}
  async *iterate(ctx) {}
  async commit(ctx, stats) {}
}
```

`iterate(ctx)` yields normalized packets:

```json
{
  "source_record_id": "contents:docs/HANDOFF.md",
  "virtual_path": "/context/github/org/repo/docs/HANDOFF.md",
  "content": "... or bytes ...",
  "content_type": "text/markdown; charset=utf-8",
  "content_sha256": "optional sha256",
  "source_updated_at": "2026-06-15T00:00:00Z",
  "metadata": { "adapter_id": "github" }
}
```

`runSourceSync()` handles the shared lifecycle: create `source_sync_run`, load `ledger_source_cursors`, dedupe unchanged content, write changed objects through the mount router, upsert `source_records`, and advance the cursor only after success.

## Capability truth

Capability flags are a contract. A flag is `true` only when the behavior exists.

| Adapter | Current sync mode | snapshot | incremental | webhook | delete detection | Notes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| `local_folder` | local courier endpoints | true | true | false | true | Existing opt-in coworker/local app flow. |
| `github` | server runner | true | true | false | true | REST contents/tree + compare API; webhook later. |
| `telegram` | server runner | false | true | false | false | Bot API `getUpdates`; webhook later. Media is metadata-only for now. |
| `slack` | not implemented | false | false | false | false | Future runner. |
| `notion` | not implemented | false | false | false | false | Future runner. |
| `linear` | not implemented | false | false | false | false | Future runner. |
| `google_drive` | not implemented | false | false | false | false | Future runner. |
| `r2` | not implemented | false | false | false | false | Object adapter/backfill future wave. |

Migration: `db/004_source_capabilities_truth.sql` reconciles the DB rows with this table. The API also returns `runner_implemented`, `sync_implemented`, and `sync_mode` from `GET /api/v1/sources/adapters`.

## Secret references

Supported now:

```txt
env:GITHUB_TOKEN
env:TELEGRAM_BOT_TOKEN
```

Reserved for later:

```txt
kv:path/to/secret
```

The server rejects `secret_ref` values that look like raw tokens.

## Manual sync route

```bash
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/sources/instances/<source-id>/sync" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  -H 'content-type: application/json' \
  -d '{ "mode": "incremental", "maxRecords": 50 }'
```

For GitHub, configure `config.owner`, `config.repo`, optional `config.branch`, optional `config.paths`, and optional `secret_ref: "env:GITHUB_TOKEN"` for private repos/rate limit. For Telegram, configure `config.chat_id` and `secret_ref: "env:TELEGRAM_BOT_TOKEN"`.

Telegram `getUpdates` is global to the bot, so use one active polling instance per bot until webhook fan-out is implemented.

## Follow-ups

- Verified webhook receivers for GitHub/Telegram.
- Slack/Notion/Linear/Drive/R2 runners.
- Queue-based large sync execution.
- Better per-instance secret model for separate API and webhook secrets.
