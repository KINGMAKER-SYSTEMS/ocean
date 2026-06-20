# Ocean Bedrock

**Status: canonical name for this cloud-box service/repo.**

Ocean Bedrock is the **cloud box / infrastructure base** where we let the team-facing Ocean support services run.

Canonical Longhouse meaning comes from `ocean-os/docs/LONGHOUSE.md`:

- `ocean-daemon` = local runtime/body and execution authority.
- `ocean-longhouse` = hive brain / coordination layer.
- Longhouse centralizes SOPs, routines, workflows, tool/MCP discovery, skills, data/memory layers, subagent specs/runtimes, and quorum/council workflows.
- Longhouse coordinates and recommends; it does **not** bypass daemon permissions.

Bedrock does not replace or redefine Longhouse. Bedrock is the cloud place that can host Longhouse-adjacent infrastructure.

## Cloud box role

```txt
Ocean Bedrock cloud box
  |
  +-- remote/local ocean-longhouse service
  +-- Ocean MCP gateway / MCP servers
  +-- storage adapters and couriers
  +-- Postgres metadata/event store
  +-- R2 object buckets
  +-- Vectorize semantic indexes
  +-- KV/cache namespaces
  +-- ingest/index/sync workers
  +-- observability and operator runbooks
```

## Relationship to canonical Ocean pieces

| Piece | Canonical role | Bedrock relationship |
| --- | --- | --- |
| `ocean-daemon` | Local runtime/body; sessions, events, provider/tool execution, local permissions | Local daemons may call Bedrock-hosted Longhouse services, but still own local execution authority. |
| `ocean-longhouse` | Hive brain / coordination layer | Bedrock can host the remote/team Longhouse deployment and its backing stores. |
| Ocean MCP | Capability/tool interface | Bedrock can host MCP servers/gateways that Longhouse discovers and daemons use. |
| Ocean Ledger / event history | Immutable/auditable context history concept | Bedrock Postgres can store team ledger/event tables if the canonical runtime chooses to use them. |
| Ocean Context | Retrieval/memory/indexing concept | Bedrock can run embeddings, graph, Vectorize, KV, and indexing jobs. |
| Ocean Surface | Client face/UI | Surface remains a client over daemon/session APIs; Bedrock is not the UI. |

## Practical meaning for this repo

The current `ocean-bedrock` service is the **Bedrock cloud-box prototype**:

- token-authenticated HTTP file/context store
- default shared directories
- lock leases
- audit/Ocean Ledger-style events
- Postgres schema drafts
Historical/conceptual examples used during early design included:

- Cloudflare R2 bucket: `ocean-longhouse`
- Cloudflare Vectorize index: `ocean-longhouse-context`
- Cloudflare KV namespace: `OCEAN_LONGHOUSE_CACHE`

These names are not automatically live deployment facts. Current live mounts, buckets, and runtime configuration belong in `docs/HANDOFF.md` and should be verified before implementation work.

It should not claim to be the canonical Longhouse quorum engine unless/until it is aligned with `ocean-os/crates/ocean-longhouse` and `ocean-os/docs/LONGHOUSE.md`.

## Guardrail

When documenting or building this repo, distinguish clearly:

```txt
Longhouse = canonical hive brain / coordination layer.
Bedrock   = cloud infrastructure box that can host/support it.
```

No invented product semantics should override the canonical `ocean-os` docs.
