# Ocean Bedrock MCP

Ocean Bedrock can be exposed to agents as an MCP server over stdio.

## Run

```bash
export OCEAN_BEDROCK_URL=https://<bedrock-host>
export OCEAN_BEDROCK_TOKEN=<scoped-token>
npm run mcp
```

The MCP server wraps the same authenticated HTTP API. Use scoped coworker/agent tokens, not admin tokens.

## Tools

```txt
bedrock_info()
bedrock_list(path, depth?)
bedrock_read(path)
bedrock_write(path, content, contentType?, ifNoneMatch?)
bedrock_mkdir(path)
bedrock_search(query, path?, limit?)
bedrock_semantic_search(query, path?, limit?, mode?)
bedrock_graph_neighborhood(path?, nodeId?, depth?, limit?)
bedrock_toolbox_manifest()
bedrock_triage_daily(reportPath?, correlationId?)
bedrock_lock(path, ttlSeconds?, note?)
bedrock_unlock(lockId)
bedrock_trace(correlationId, limit?)
bedrock_snapshot(name, correlationId?, virtualPath?, files?, events?, summary?, clearance?, metadata?)

Semantic/graph/triage tools require server-side Postgres. Semantic mode requires Cloudflare Workers AI + Vectorize env; otherwise use `mode: "lexical"` for chunk fallback.
```

## Resources

```txt
ocean-bedrock://docs
ocean-bedrock://context
ocean-bedrock://coworkers
ocean-bedrock://sessions
ocean-bedrock://handoffs
```

## Example local smoke

```bash
OCEAN_BEDROCK_BOOTSTRAP_TOKEN=dev-token-change-me npm start

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bedrock_info","arguments":{}}}' \
| OCEAN_BEDROCK_URL=http://127.0.0.1:8080 OCEAN_BEDROCK_TOKEN=dev-token-change-me npm run mcp
```
