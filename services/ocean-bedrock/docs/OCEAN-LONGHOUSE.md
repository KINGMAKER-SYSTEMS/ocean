# Ocean Longhouse

Ocean Longhouse is the storage and context hub for Ocean OS: a stateful, token-authenticated virtual filesystem that can route agent reads/writes across many backing storage resources.

It should feel like one company shared directory to agents, while internally it can use local disk, mounted NAS paths, Cloudflare R2/S3, GitHub repos, WebDAV/SFTP, Google Drive, Notion exports, and future peer Longhouse nodes.

## Core idea

Longhouse is not just storage. It is a **control plane + virtual filesystem**.

```txt
Agents / Humans / MCP clients
          |
          v
Ocean Longhouse API + MCP server
  - token auth and scopes
  - virtual paths
  - locks/leases
  - search/index
  - audit log
  - policy router
  - mount registry
          |
          v
Storage adapters
  - local managed disk
  - mounted NAS via SMB/NFS
  - Cloudflare R2 / S3-compatible buckets
  - GitHub repositories
  - WebDAV / SFTP
  - Google Drive / Workspace
  - Notion-backed docs exports
  - remote Longhouse peers
```

## Repo roles

Suggested split for the Ocean repos:

| Repo | Role |
| --- | --- |
| `ocean-os` | Core contracts, SDK types, shared schemas, policy language, MCP tool definitions |
| `ocean-longhouse` | This service: virtual filesystem, mount registry, adapters, tokens, locks, audit, replication |
| `ocean-surface` | Web UI/dashboard for browsing files, managing mounts/tokens, viewing sessions/audit/search |
| public `ocean-agents` | Reusable profiles, package conventions, and generic client/skill wrappers |
| private `risingtides-agents` | Rising Tides production clients, prompts, skills, MCP configs, and workflow behavior |

The current `ocean-bedrock` prototype is the cloud-box support layer for Longhouse-adjacent storage, context, ingest, and MCP-facing infrastructure. It should align with canonical `ocean-os/docs/LONGHOUSE.md` rather than redefining Longhouse.

## Source-of-truth rules

Longhouse should explicitly declare authority per workflow:

| Data | Source of truth |
| --- | --- |
| Mount registry, routing policies, issued Longhouse tokens, locks, audit | Longhouse DB |
| File bytes | The selected backing mount, unless stored in Longhouse-managed local storage |
| Search index and previews | Derived cache; rebuildable |
| Git-backed docs | GitHub/Git is authoritative; Longhouse is a gateway/cache |
| External app state, e.g. Notion page metadata | The external app is authoritative |
| Secrets/auth | Prefer secrets manager or encrypted secret objects; Longhouse stores references/handles where possible |

Do not silently stripe important files across free tiers. Use one canonical location plus optional replicas.

## Virtual namespace

Longhouse exposes friendly paths while routing them to mounts:

```txt
/docs              durable docs, usually GitHub or managed storage
/context           shared agent context and knowledge packets
/sessions          session state and artifacts
/handoffs          cross-agent handoffs
/shared            general shared files
/assets            large/public assets, often R2/S3
/archive           cold storage, often local NAS or cheap bucket
/vault             encrypted secret handles only; avoid raw secrets
/mounts/<id>       raw view of mounted backend when needed
```

Example mount registry:

```json
{
  "mounts": [
    {
      "id": "managed-local",
      "type": "local",
      "prefix": "/",
      "root": "data/files",
      "mode": "readwrite",
      "priority": 100
    },
    {
      "id": "company-nas",
      "type": "local",
      "prefix": "/archive/nas",
      "root": "/mnt/company-nas",
      "mode": "readwrite",
      "priority": 90
    },
    {
      "id": "r2-assets",
      "type": "s3",
      "prefix": "/assets",
      "bucket": "content-posting-lab",
      "endpointSecretRef": "cloudflare-r2-endpoint",
      "credentialsSecretRef": "cloudflare-r2-longhouse",
      "mode": "readwrite",
      "priority": 80
    },
    {
      "id": "ocean-docs-github",
      "type": "github",
      "prefix": "/docs/ocean",
      "repo": "risingtides-dev/ocean-os",
      "branch": "main",
      "root": "docs",
      "mode": "readwrite",
      "priority": 70
    }
  ]
}
```

The router picks the most specific matching `prefix`. For example `/docs/ocean/README.md` routes to GitHub, while `/sessions/foo/state.json` stays on managed local storage.

## Adapter contract

Every backend should implement the same storage adapter interface:

```ts
interface StorageAdapter {
  id: string;
  type: string;
  capabilities: {
    read: boolean;
    write: boolean;
    delete: boolean;
    move: boolean;
    nativeVersioning?: boolean;
    signedUrls?: boolean;
    maxObjectBytes?: number;
  };

  list(path: string, options: { depth: number; ctx: RequestContext }): Promise<ListResult>;
  stat(path: string, ctx: RequestContext): Promise<FileInfo>;
  read(path: string, ctx: RequestContext): Promise<ReadableStream | Uint8Array>;
  write(path: string, body: ReadableStream | Uint8Array, ctx: WriteContext): Promise<FileInfo>;
  delete(path: string, ctx: RequestContext): Promise<void>;
  mkdir?(path: string, ctx: RequestContext): Promise<FileInfo>;
  move?(from: string, to: string, ctx: RequestContext): Promise<FileInfo>;
  copy?(from: string, to: string, ctx: RequestContext): Promise<FileInfo>;
}
```

Longhouse owns auth, scopes, locks, audit, and path routing before calling adapters.

## Local NAS integration patterns

### 1. NAS mounted on Longhouse host

Mount the NAS on the server with SMB/NFS:

```bash
/mnt/company-nas
```

Register it as a `local` adapter under `/archive/nas` or `/shared/nas`.

This is simplest when Longhouse and NAS are on the same LAN/VPN.

### 2. NAS as an edge Longhouse peer

Run a small Longhouse edge daemon near the NAS. The daemon connects outbound to central Longhouse, receives sync/jobs, and exposes selected NAS folders as remote mounts.

This is better when the NAS is behind NAT or a firewall.

### 3. NAS as backup target

Use restic/rsync jobs to snapshot Longhouse-managed data to NAS. In this mode the NAS is not active shared storage; it is disaster recovery.

## Free-tier bundling strategy

Use free/cheap tiers as **placement targets**, not as a fragile single logical disk.

Recommended classes:

| Data class | Good primary | Good replicas/cache |
| --- | --- | --- |
| Markdown docs, SOPs, prompts | GitHub repo | Longhouse cache, R2 snapshot |
| Agent sessions/context JSON/MD | Longhouse managed storage | GitHub snapshot or R2 |
| Large media/assets | R2/S3/NAS | NAS/R2 mirror |
| Cold archives | NAS | R2/Backblaze-style object storage |
| Public static artifacts | R2/Cloudflare | GitHub release if small |
| Secrets/auth | Secrets manager/encrypted vault | never plain replicated |

Avoid hidden split-file storage at first. If a provider fails or changes quota, agents should still know which location is canonical.

## MCP layer

Longhouse HTTP API remains canonical. The MCP server wraps it with agent-friendly tools and resources.

Suggested MCP tools:

```txt
longhouse_list(path, depth?)
longhouse_read(path)
longhouse_write(path, content, lockId?)
longhouse_delete(path)
longhouse_search(query, path?)
longhouse_lock(path, ttlSeconds?, note?)
longhouse_unlock(lockId)
longhouse_create_session(name, metadata?)
longhouse_write_handoff(toAgent, summary, files?, context?)
longhouse_mount_status()
```

Suggested MCP resources:

```txt
longhouse://docs/...
longhouse://context/...
longhouse://sessions/...
longhouse://handoffs/...
```

Agents receive Longhouse tokens, not provider credentials. Longhouse maps the token to scopes and backend access.

## Ocean Ledger

Ocean Ledger is the snapshot/version-control layer for context across company communications. It records immutable events from files, chats, tickets, GitHub, deploys, incidents, and agent sessions, then connects them with correlation IDs.

Detailed design: [`OCEAN-LEDGER.md`](OCEAN-LEDGER.md)

## Data plane

Detailed Cloudflare/Postgres/vector/graph/cache plan: [`OCEAN-LONGHOUSE-DATA-PLANE.md`](OCEAN-LONGHOUSE-DATA-PLANE.md)

Initial Postgres schema drafts:

- [`../db/001_longhouse_core.sql`](../db/001_longhouse_core.sql)
- [`../db/002_ocean_ledger.sql`](../db/002_ocean_ledger.sql)

## Phased build plan

### Phase 1 — Virtual filesystem router **(SHIPPED 2026-06-15)**

- ~~Add `mounts.json` registry.~~ → Postgres `longhouse.mounts` table (7 seeded rows), 30s TTL cache, no-DB synthetic fallback.
- ~~Refactor current file operations through a `local` adapter.~~ → `LocalStorageAdapter` in `src/mounts/adapters/local.mjs`; all stat/list/read/write/delete/move/copy/mkdir/search route through it.
- ~~Add `GET/POST/DELETE /api/v1/mounts` for admins.~~ → Live; admin-gated, config redaction, local root containment validation.
- ~~Keep existing file API stable.~~ → `npm run smoke` green; no breaking API changes.

### Phase 2 — First external adapters

- Add `s3` adapter for Cloudflare R2/S3-compatible storage.
- Add `github` adapter for docs/prompts/handoffs.
- Add `webdav` or `sftp` adapter for easy NAS/hosted storage.
- Add `remote-longhouse` adapter for peer hubs.

### Phase 3 — Index, replication, and policies

- Move metadata to SQLite/Postgres.
- Add content hashing/manifests: path -> canonical location -> replicas.
- Add background sync jobs.
- Add lifecycle policies: pin, mirror, archive, cache, evict.
- Add quota/cost/status tracking per mount.

### Phase 4 — Ocean hub productization

- Expose MCP server.
- Connect `ocean-surface` dashboard.
- Publish reusable `ocean-agents` client/skill wrappers; keep deployment-specific configuration in private `risingtides-agents`.
- Add org/team/project abstractions.
- Add approval flows for destructive actions and secret access.

## Immediate next implementation step

Phase 1 is shipped. Next: Phase 2 first external adapters.

Priority: `s3`/R2 adapter — keys exist in DB (`r2-*` mounts seeded), need Cloudflare R2 S3 access keys in Railway env and an `S3StorageAdapter` in `src/mounts/adapters/s3.mjs`.

Current file layout (live):

```txt
src/
  server.mjs
  auth.mjs
  metadata.mjs
  mounts/
    adapter.mjs       ← HttpError + createUnimplementedAdapter
    registry.mjs      ← loadMounts, invalidateMountRegistry
    router.mjs        ← resolveMount
    adapters/
      local.mjs       ← LocalStorageAdapter (LIVE)
  mcp/
    server.mjs
```

Once `local` works through the adapter interface, every new storage backend becomes a plugin rather than a rewrite.
