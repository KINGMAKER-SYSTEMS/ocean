# Ocean MCP

## Purpose

Connect Ocean to external MCP servers, and expose the local daemon to MCP hosts.

## Ownership

- The library owns MCP client transports and the runtime capability adapter.
- `src/bin/ocean_mcp.rs` owns the `ocean-mcp serve` stdio bridge and read-only
  `doctor` command. `src/bin/ocean_mcp/tests.rs` checks its daemon projections.
- `fake_server` is a test fixture, not the user-facing server.

## Local Contracts

- The bridge uses only a numeric loopback HTTP daemon origin, without redirects,
  proxy inheritance, credentials, operator headers or automatic mutation retries.
  It does not start a daemon, configure a host or install itself.
- Human join/post bodies carry no author/member identity. The daemon derives
  attribution and enforces room membership, agent authority and permissions.
- Local rosters use room participants; every shared access state uses only the
  daemon's access members. Preserve empty shared rosters and visible read errors.
- Room inspection composes snapshot, bindings, resource metadata and slot status
  from their actual read routes. The migration `/inspect` route is not an agent
  execution summary. Missing reads must not become "no authorized agents".
- Transcript reads use supported forward `after_seq`/`next_seq` pagination.
  Reject the older bridge's unsupported `before_seq`; do not label a prefix as
  the newest messages. Preserve thread/session links and room/session cursors.
- Bound stdio frames, HTTP replies and tool output; an oversized page is an error,
  never silently truncated JSON. Large prompt output is an explicitly marked
  excerpt that retains the completed turn's session, request and outcome. Only
  allowed metadata crosses projections.
- Prompt execution remains daemon-owned, with `yolo` false unless explicitly
  requested. Never auto-join, post, retry or change credentials after an error.

## Work Guidance

- Development build: `cargo build --locked -p ocean-mcp --bin ocean-mcp`. Configure the MCP
  host with that executable's absolute path and `serve` as its argument.
  Windows produces `ocean-mcp.exe`; `--daemon` can select another loopback port.
- Desktop candidates bundle this executable beside the daemon and verify its
  stdio connection in disposable state. The npm package does not distribute it.
  Building or bundling is not stable publication, host setup or two-machine
  acceptance. Existing host configurations are not modified by this crate.

## Verification

- `cargo test --locked -p ocean-mcp`
- `cargo clippy --locked -p ocean-mcp --all-targets -- -D warnings`
- `cargo xtask docs-check`
- A read-only actual stdio invocation against the running daemon verifies live
  projections separately from fixture tests; it must not invoke write tools.

## Child devlog Index

- None.
