# hub

The courier hub's **router** — the `slash-command → courier` dispatch layer.
See [`../ARCHITECTURE.md`](../ARCHITECTURE.md) for the full picture.

```bash
python3 router.py list                          # all couriers + commands
python3 router.py resolve /ship                 # who handles a command (JSON)
python3 router.py run --dry-run /ship --channel C0123ABCD
python3 router.py run /ship --channel C0123ABCD
```

- Discovers every `../*/courier.toml` — adding a courier needs no router edit.
- Router flags go **before** the slash (`--dry-run`, `--channel <id>`,
  `--thread-ts <ts>`, `--session-key <k>`); everything after the slash is the
  courier's own argv, passed through verbatim.
- **deterministic** couriers → execs the harness directly.
- **agentic** couriers → `POST /v1/prompt` to the Ocean daemon
  (`$OCEAN_DAEMON_URL`, default `http://127.0.0.1:4780`), scoped to the courier's
  dir so the daemon auto-loads its `CLAUDE.md`. The Slack session key
  (`slack:<courier>:<channel>:<thread_ts|dm>`, or `--session-key`) is folded into
  a deterministic UUIDv5 `session_id`, so the same thread resumes the same daemon
  session. Transient daemon failures (5xx / unreachable / timeout) are retried
  with backoff (`OCEAN_DAEMON_MAX_TRIES`, default 3; `OCEAN_DAEMON_TIMEOUT`,
  default 300s), and the agent's reply is posted back to `--channel` over the
  shared Slack transport.

The Slack slash-command intake (the front door that calls this router) lives in
[`../../assistants/bridge/slash_intake.py`](../../assistants/bridge/slash_intake.py),
riding the Socket Mode bridge — see [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
