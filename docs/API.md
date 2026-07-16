# Ocean Bedrock API

Base URL: `http://host:8080`

Auth: `Authorization: Bearer <token>`

All filesystem paths are POSIX-style absolute paths inside the shared root, for example `/docs/brief.md`. `..`, backslashes, and NUL bytes are rejected.

## Roles

| Role | Permissions |
| --- | --- |
| `readonly` | list, stat, read, search |
| `contributor` | read/write/lock under scoped paths; no delete |
| `agent` | read/write/delete/lock under scoped paths |
| `readwrite` | same as `agent` |
| `admin` | all agent permissions plus token and audit management |

Tokens may also have path scopes such as `['/docs', '/context']`.

## Endpoints

### `GET /health`

Public health check.

### `GET /api/v1/info`

Returns instance metadata, default folders, upload limits, and current principal.

### `GET /api/v1/list?path=/docs&depth=1`

Lists a directory. Use `depth > 1` for recursive listing.

### `GET /api/v1/tree?path=/docs&depth=5`

Alias for recursive listing with a higher default depth.

### `GET /api/v1/stat?path=/docs/file.md`

Returns metadata for a file or directory.

### `GET /api/v1/file?path=/docs/file.md[&inline=1]`

Downloads a file. Returns an `ETag` header. Use `inline=1` to avoid attachment content-disposition.

### `PUT /api/v1/file?path=/docs/file.md`

Creates or replaces a file with the raw request body.

Concurrency helpers:

- `If-None-Match: *` prevents overwriting an existing file.
- `If-Match: "etag"` only writes if the current ETag matches.

### `DELETE /api/v1/file?path=/docs/file.md`

Deletes a file.

For directories, pass `recursive=true`:

`DELETE /api/v1/file?path=/docs/old-folder&recursive=true`

### `POST /api/v1/mkdir`

Body:

```json
{ "path": "/docs/projects" }
```

Creates a directory recursively.

### `POST /api/v1/move`

Body:

```json
{ "from": "/docs/a.md", "to": "/docs/archive/a.md", "overwrite": false }
```

Moves/renames a file or directory.

### `POST /api/v1/copy`

Same body as move, but copies recursively.

### `GET /api/v1/search?q=term&path=/docs&limit=50`

Simple text search across scoped files. Large/binary files are skipped.

## Semantic search, graph, Ocean Context, and toolbox

### `GET /api/v1/semantic/search?q=...&path=/context&limit=10`

Searches indexed chunks through Cloudflare Workers AI embeddings + Cloudflare Vectorize when configured. Results are filtered by token path scope. If semantic env is not configured, use `mode=lexical` or rely on lexical chunk fallback.

### `GET /api/v1/graph/nodes?path=/context&type=file&limit=100`

Lists graph nodes visible under a scoped path.

### `GET /api/v1/graph/neighborhood?path=/context/file.md&depth=2`

Returns nearby graph nodes/edges for a scoped file or graph node.

### `POST /api/v1/ocean-context/triage/daily`

Runs the daily Ocean Context triage, writes `/context/ocean-bedrock/triage/YYYY-MM-DD.md`, and appends an `ocean_context.triage.completed` ledger event.

### `GET /api/v1/toolbox/manifest`

Returns the token-free Ocean Toolbox manifest for MCP setup, local companion app behavior, skill categories, and staged computer-auth flow.

## Source registry and sync runs

Source registry endpoints are server-side Postgres-backed lineage APIs. Coworker devices should call these through Ocean Bedrock with their scoped bearer token; they should not receive `DATABASE_URL`.

### `GET /api/v1/sources/adapters`

Lists available source adapters such as `local_folder`, `github`, `telegram`, `slack`, `notion`, `linear`, `google_drive`, and `r2`.

### `POST /api/v1/sources/instances`

Creates or updates a configured source instance. The source `remote_prefix` must be inside the token's path scopes.

```json
{
  "adapter_id": "local_folder",
  "name": "alice-macbook",
  "owner_name": "alice",
  "remote_prefix": "/coworkers/alice/macbook",
  "config": { "device": "macbook" },
  "clearance": "CONFIDENTIAL"
}
```

### `GET /api/v1/sources/instances`

Lists source instances visible to the token. Optional filters: `adapter_id`, `owner_name`, `status`, `enabled`, `limit`.

### `GET /api/v1/sources/instances/{id}`

Returns one source instance with streams and recent sync runs.

### `PATCH /api/v1/sources/instances/{id}`

Updates mutable source instance fields such as `remote_prefix`, `config`, `status`, and `enabled`.

### `POST /api/v1/sources/streams`

Creates or updates a selected stream/resource under a source instance.

```json
{
  "source_instance_id": "uuid",
  "stream_key": "src_docs",
  "stream_type": "folder",
  "remote_prefix": "/coworkers/alice/macbook/docs",
  "selection": { "label": "docs" }
}
```

### `GET /api/v1/sources/streams`

Lists streams visible to the token. Optional filters: `source_instance_id`, `stream_type`, `enabled`, `limit`.

### `PATCH /api/v1/sources/streams/{id}`

Updates mutable stream fields such as `remote_prefix`, `selection`, `cursor`, and `enabled`.

### `POST /api/v1/sync-runs`

Starts a sync run for a source instance.

```json
{ "source_instance_id": "uuid", "metadata": { "reason": "manual" } }
```

### `GET /api/v1/sync-runs/{id}`

Returns sync run status and counters.

### `POST /api/v1/sync-runs/{id}/complete`

Completes a sync run with counters and an optional manifest path.

### `POST /api/v1/sync-runs/{id}/fail`

Marks a sync run failed with error details.

## Local folder sync helper endpoints

These are higher-level endpoints used by `scripts/ocean-ingest-local.mjs` and the local GUI companion app.

### `POST /api/v1/sync/local-folder/plan`

Registers/updates the local-folder source instance, creates streams, and starts a sync run.

### `POST /api/v1/sync/local-folder/records:batch`

Upserts file/message/source records after bytes are uploaded through `PUT /api/v1/file`.

### `POST /api/v1/sync/local-folder/commit`

Flushes final run stats and marks the sync run complete or failed.

## Ocean Ledger

Ocean Ledger is append-only context history across files, messages, tickets, sessions, deploys, and other company events.

### `GET /api/v1/ledger/events`

Lists ledger events visible to the token. Optional filters:

```txt
correlation_id
actor_id
actor_name
actor_type
event_type
source_id
path
limit
```

### `POST /api/v1/ledger/events`

Appends an immutable ledger event.

Body:

```json
{
  "event_type": "decision.recorded",
  "correlation_id": "cor-ocean-001",
  "lab": "communications",
  "virtual_path": "/docs/ocean/longhouse.md",
  "payload": { "decision": "Use Cloudflare R2 + Postgres + Vectorize." },
  "clearance": "CONFIDENTIAL",
  "tags": ["architecture", "cloudflare"]
}
```

### `GET /api/v1/ledger/trace?correlation_id=cor-ocean-001`

Returns the visible event chain for a correlation ID.

### `POST /api/v1/ledger/snapshots`

Creates a context snapshot event for what an agent/human could see at a point in time.

Body:

```json
{
  "name": "pre-write-sales-draft",
  "correlation_id": "cor-sales-001",
  "virtual_path": "/sessions/writer/2026-06-08/context.json",
  "files": ["/docs/sales/offer.md", "/context/customer/persona.md"],
  "events": ["evt_abc", "evt_def"],
  "summary": "Context visible before drafting follow-up sequence.",
  "clearance": "CONFIDENTIAL"
}
```

### `GET /api/v1/ledger/verify`

Admin only. Verifies the local hash chain.

## Locks

Locks are lease-based and expire automatically. They are enforced for writes/deletes/moves/copies when a different token owns an intersecting lock.

### `GET /api/v1/locks[?path=/docs/file.md]`

Lists active locks.

### `POST /api/v1/locks`

Body:

```json
{
  "path": "/docs/file.md",
  "ttlSeconds": 900,
  "note": "editing section 2"
}
```

### `DELETE /api/v1/locks/{lockId}`

Releases a lock. Admins can release any lock; agents can release their own locks.

## Tokens

Admin only.

### `GET /api/v1/tokens`

Lists token records without token secrets/hashes.

### `POST /api/v1/tokens`

Body:

```json
{
  "name": "research-agent",
  "role": "agent",
  "scopes": ["/docs", "/context"],
  "ttlDays": 30
}
```

Response includes a `token` field exactly once. Store it immediately.

### `DELETE /api/v1/tokens/{tokenId}`

Revokes a token.

## Audit

### `GET /api/v1/audit?limit=100`

Admin only. Returns recent write/delete/lock/token events.

## Federated rooms (Gate-2 S1A)

Room membership lives in Postgres (`longhouse.room_members`) and requires the
single shared database authority: auth, ledger, and rooms must resolve to the
same `DATABASE_URL` (with `OCEAN_AUTH_DATABASE_URL` absent or equal). In any
split, file, or JSONL configuration the five `/api/v1/rooms/...` routes and
the `room_id` branch of `POST /api/v1/invites` return an explicit `503` and
never log a database URL; the generic invite branch and ordinary ledger
behavior remain unchanged (no registered rooms can exist there, so the room
gates never engage).

### `POST /api/v1/rooms/{room_id}/register`

Admin + `/rooms/{room_id}` scope. Optional `{ "title" }` body. Atomically
claims the ledger correlation (id = room_id, metadata `federation_kind=room`)
and creates the owner human row from the authenticated principal. `201
{ room_id, owner }`; same-principal retry `200`; unrelated existing
correlation or concurrent loser `409`.

### `POST /api/v1/invites` (room branch)

Body `{ room_id, recipient_name?, ttl_minutes? }` from an ACTIVE room owner.
Role (`contributor`) and exact scope (`/rooms/{room_id}`) are derived
server-side; supplying `role`/`scopes` is `400`. TTL default 1440, bounds
1..10080. `201 { code, invite }` — safe fields only. The generic admin branch
(no `room_id`) is unchanged.

### `POST /api/v1/rooms/{room_id}/members/self`

No body. Requires the contributor token minted by a CONSUMED invite scoped
exactly to the room (provenance proven in-transaction). `201 { member }`;
retry `200`; removed principal on the same token `403 membership_revoked`.

### `POST /api/v1/rooms/{room_id}/members/agents`

Body `{ agents: [{ registration_key, display_name, description?,
model_alias?, skills_count?, subagent_names? }] }`, max 32, one all-or-nothing
transaction, request order preserved. Local paths/tools/credentials are
rejected. All-new `201`; retry/mixed `200`; same key with different content
`409`.

### `GET /api/v1/rooms/{room_id}/members`

Active roster for active members: safe projections only —
`owner_principal_token_id`, `registration_key`, `removed_by`, and bearer
material never serialize.

### `DELETE /api/v1/rooms/{room_id}/members/{member_id}`

Owner removes any other member (owned agents removed atomically); a member
removes only self; owner self-leave `409`. Repeat delete by a
still-authorized caller `200 { removed: [] }`; a self-removed caller `403
membership_revoked`.

### Ledger gating for registered rooms

Any ledger read/write/trace identifying a registered room (by correlation id
or a `/rooms/{room_id}` path, descendants included) requires room scope plus
ACTIVE membership; admin does not bypass, and broad reads suppress foreign
room rows per-event. Writes additionally require the canonical pair
(`correlation_id = room_id`, `virtual_path = /rooms/{room_id}`) and an
`actor_member_id` owned by the calling principal. Non-room ledger behavior is
unchanged.

### Producer idempotency and cursor paging (Gate-2 S1B)

`POST /api/v1/ledger/events` with a `(source_id, source_sequence)` tuple is
idempotent ONLY for identical normalized content — the prior event is
returned. Any content delta on a reused tuple is `409` with
`{ source_id, source_sequence, existing_event_id }`. A `source_id` matching
`^room:` requires `source_sequence` as a canonical decimal u64 STRING (JSON
numbers are rejected `400`, before any I/O); other sources keep arbitrary
text.

`GET /api/v1/ledger/events?after_sequence=N` pages the ledger as a cursor:
canonical decimal sequence string, strictly greater, ascending, bounded by
`limit`; pass the last returned sequence to continue. Without the parameter,
the existing newest-window behavior is unchanged.
