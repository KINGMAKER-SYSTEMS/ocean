# Ocean Ledger

Ocean Ledger is the shared temporal substrate for Ocean Longhouse.

In plain language: it is **snapshot/version control for context across company communications**. It connects the intent, messages, tickets, files, commits, deployments, incidents, agent sessions, and decisions that usually live in separate tools.

Agents should not have to rely on human memory to know why a file exists, who requested it, what decision changed it, or what context was visible when another agent acted. Ocean Ledger makes that history queryable.

## What it solves

Company work happens across disconnected timelines:

```txt
Notes / docs      -> intent and plans
Linear / tasks    -> work items
GitHub            -> code and reviews
Slack / Telegram  -> decisions and coordination
Longhouse files   -> docs, context, sessions, handoffs
Railway/CF        -> deploys, incidents, infrastructure
Agents            -> claims, context snapshots, actions
```

Ocean Ledger gives them one append-only event stream and correlation layer.

## Core concepts

### Event

An immutable fact that happened.

Examples:

```txt
intent.created
ticket.created
decision.recorded
session.started
agent.context_snapshot
file.created
file.updated
github.pr_opened
deploy.completed
incident.detected
incident.correlated
```

### Correlation ID

A thread that links events across tools and time.

Example:

```txt
cor-payments-stripe-001
```

Querying that correlation returns the story:

```txt
intent -> ticket -> agent claim -> decision -> file/code changes -> deploy -> incident
```

### Context snapshot

A point-in-time record of what an agent or human could see when they acted.

This is the accountability layer:

```txt
What ticket text did the agent see?
What docs were included?
What files were visible?
What clearance level was active?
What token/session was used?
```

### Hash chain

Each event stores the previous event hash and its own canonical hash. If an old event is modified, verification fails.

This makes the ledger tamper-evident.

### Clearance

Events can be marked:

```txt
PUBLIC
UNCLASSIFIED
CONFIDENTIAL
SECRET
TOP_SECRET
```

Tokens/agents only see events they are cleared for and scoped to.

## Event schema

Canonical JSON shape:

```json
{
  "id": "evt_...",
  "schema_version": 1,
  "sequence": 42,
  "event_type": "decision.recorded",
  "correlation_id": "cor-payments-stripe-001",
  "lab": "communications",
  "actor_type": "user",
  "actor_id": "john",
  "actor_name": "John",
  "source_id": "slack",
  "source_sequence": "1728347729.000100",
  "virtual_path": "/docs/payments/stripe.md",
  "object_id": null,
  "payload": {
    "channel": "#payments-dev",
    "message": "Ship the MVP first, add stricter validation in v2.",
    "decision_type": "scope_change",
    "rationale": "Timeline pressure"
  },
  "visible_context": null,
  "context_snapshot": null,
  "clearance": "CONFIDENTIAL",
  "tags": ["payments", "stripe", "mvp"],
  "timestamp": "2026-06-08T10:31:20.274Z",
  "received_at": "2026-06-08T10:31:21.001Z",
  "prev_hash": "...",
  "hash": "..."
}
```

## Snapshot model

Ocean Ledger gives version-control-like context snapshots without forcing every tool into Git.

A snapshot can reference:

- a range of ledger event sequences
- Longhouse virtual files
- source tool IDs
- object hashes
- visible context packets
- vector/chunk IDs for semantic retrieval
- graph node IDs for relationship traversal

Example snapshot packet:

```json
{
  "event_type": "agent.context_snapshot",
  "correlation_id": "cor-sales-sequence-001",
  "actor_type": "agent",
  "actor_id": "writer-agent-02",
  "virtual_path": "/sessions/writer-agent-02/2026-06-08/context.json",
  "payload": {
    "snapshot_kind": "pre_write",
    "reason": "Drafting sales follow-up sequence",
    "visible_files": [
      "/docs/sales/offer.md",
      "/context/customer/persona.md"
    ],
    "visible_events": ["evt_abc", "evt_def"],
    "token_scope": ["/docs", "/context", "/sessions/writer-agent-02"],
    "clearance_level": "CONFIDENTIAL"
  },
  "clearance": "CONFIDENTIAL"
}
```

## Adapter ingest

Every integration becomes a ledger producer:

| Adapter | Events |
| --- | --- |
| Longhouse filesystem | `file.created`, `file.updated`, `file.deleted`, `lock.created` |
| Telegram/Slack | `message.received`, `decision.recorded`, `handoff.created` |
| GitHub | `github.issue_created`, `github.pr_opened`, `git.commit`, `review.submitted` |
| Linear | `ticket.created`, `ticket.updated`, `ticket.completed` |
| Railway/Cloudflare | `deploy.started`, `deploy.completed`, `incident.detected` |
| Agents | `session.started`, `job.claimed`, `agent.context_snapshot`, `agent.action_taken` |

Adapters should store external IDs in `source_id`, `source_sequence`, and `payload.external_ref` so ingestion is idempotent.

## Relationship with Longhouse

Ocean Longhouse is the filesystem/context hub.

Ocean Ledger is the historical memory and causality layer inside it.

```txt
Longhouse virtual file: /docs/payments/stripe.md
        |
        +--> object metadata in Postgres
        +--> bytes in local/R2/GitHub/etc.
        +--> ledger events recording every meaningful change
        +--> graph nodes/edges linking it to people, tasks, decisions, sessions
        +--> embedding chunks for semantic retrieval
```

## API

Current prototype endpoints:

```txt
GET  /api/v1/ledger/events
POST /api/v1/ledger/events
GET  /api/v1/ledger/trace?correlation_id=...
POST /api/v1/ledger/snapshots
GET  /api/v1/ledger/verify
```

Example write:

```bash
curl -X POST "$OCEAN_BEDROCK_URL/api/v1/ledger/events" \
  -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"decision.recorded",
    "correlation_id":"cor-ocean-001",
    "lab":"communications",
    "virtual_path":"/docs/ocean/longhouse.md",
    "payload":{"decision":"Use Cloudflare R2 + Postgres + Vectorize for Longhouse."},
    "clearance":"CONFIDENTIAL"
  }'
```

Example trace:

```bash
curl -H "Authorization: Bearer $OCEAN_BEDROCK_TOKEN" \
  "$OCEAN_BEDROCK_URL/api/v1/ledger/trace?correlation_id=cor-ocean-001"
```

## Storage modes

V1 supports two storage modes:

```txt
OCEAN_LEDGER_STORE=jsonl     # force local JSONL prototype mode
OCEAN_LEDGER_STORE=postgres # durable Postgres-backed ledger; default when DATABASE_URL is set
```

The local JSONL file defaults to:

```txt
data/.ocean-bedrock/ocean-ledger.jsonl
```

The Postgres schema is declared in:

```txt
db/002_ocean_ledger.sql
```

Postgres should become the authoritative production ledger store. JSONL remains useful as a local export/sync format.

Check schema readiness:

```bash
npm run db:check
```

## Design rules

1. **Append only.** Do not edit old events. Add correcting events.
2. **Correlate aggressively.** Every event should carry a correlation ID when possible.
3. **Snapshot before meaningful action.** Agents record what they saw before edits, sends, deploys, or destructive operations.
4. **Keep bytes separate from history.** Files live in Longhouse mounts; history lives in Ocean Ledger.
5. **Clearance matters.** Avoid leaking secret context into low-clearance events.
6. **Derived indexes are rebuildable.** Vector, graph, and cache data should be reconstructable from files + ledger + source adapters.
