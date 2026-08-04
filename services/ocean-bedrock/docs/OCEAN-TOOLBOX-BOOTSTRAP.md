# Ocean Toolbox Bootstrap

Purpose: stage Bedrock so a coworker can download the local companion, authorize their computer for Ocean, sync opt-in data, and invoke the Rising Tides toolbox through MCP/agents.

## Target flow

```txt
operator issues scoped token/invite
  -> coworker opens local companion GUI
  -> computer stores token locally outside repo
  -> user picks folders/data sources
  -> Bedrock registers source_instance/source_stream records
  -> Bedrock indexes chunks, vectors, graph, ledger events
  -> MCP clients and agents use scoped tools/context to carry out tasks
```

## Toolbox manifest

Authenticated endpoint:

```txt
GET /api/v1/toolbox/manifest
```

The manifest describes:

- Bedrock URL,
- recommended auth model,
- MCP server command/env,
- available Bedrock MCP tools,
- resource roots,
- Rising Tides skill categories,
- local companion behavior,
- staged install/auth flow.

This is intentionally token-free. It is safe to show to a coworker, but they still need a scoped token/invite to use Bedrock.

## Local companion app

Run from repo today:

```bash
npm run ocean:app
```

Planned packaged version:

```txt
Ocean Companion.app / Ocean Companion.exe
```

V1 GUI responsibilities:

- accept one-time token/invite,
- save config at `~/.config/ocean-bedrock/bootstrap.json`,
- let user choose opt-in folders,
- run manual/scheduled sync,
- show source sync status,
- expose setup instructions for MCP clients.

## MCP server

Run:

```bash
export OCEAN_BEDROCK_URL='https://ocean-bedrock-production.up.railway.app'
export OCEAN_BEDROCK_TOKEN='<scoped-token>'
npm run mcp
```

Core tools:

```txt
bedrock_info
bedrock_list
bedrock_read
bedrock_write
bedrock_mkdir
bedrock_search
bedrock_semantic_search
bedrock_graph_neighborhood
bedrock_trace
bedrock_snapshot
bedrock_toolbox_manifest
bedrock_triage_daily
```

## Auth boundaries

Default coworker/device token:

```txt
role: contributor
scopes:
  /coworkers/<person>
  /sessions/<person>
  /context/ocean-bedrock
```

Contributor tokens can read/write/lock but cannot delete or administer tokens.

## Agent invocation model

Agents should act through scoped Bedrock/MCP context:

1. Read relevant files and semantic results.
2. Create an Ocean Ledger context snapshot before high-impact work.
3. Write outputs into `/sessions/<person>` or `/coworkers/<person>`.
4. Use graph/ledger traces to explain why actions were taken.
5. Ask for approval before publishing externally, deleting data, or making risky deployments.

## Next hardening steps

- one-time invite exchange instead of raw token paste,
- packaged GUI download,
- OS background scheduling,
- signed toolbox manifest,
- per-device token registry,
- revocation UX,
- richer skill bundle distribution.
