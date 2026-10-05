# Couriers

Couriers are narrow delivery packages discovered from manifests. They resolve a
destination, confirm it in human terms, and ship a bounded payload through a
shared transport.

## Shared route

[`transport/slack.py`](transport/slack.py) is the public Slack transport
reference. It owns token resolution, destination parsing, API normalization,
upload flow, retries, rate limits, messages, threads, and canvas operations.
Packages must not duplicate that logic.

## Public package

| Courier | Mode | Purpose |
| --- | --- | --- |
| [`file-courier`](file-courier) | deterministic | Upload files or send text to a confirmed Slack destination |

Run the authoritative registry:

```bash
python3 hub/router.py list
```

## Adding a package

1. Create `couriers/<name>/courier.toml`.
2. Add `CLAUDE.md`, `AGENTS.md`, and a narrow `bin/courier` harness or an
   agentic prompt contract.
3. Import shared transport instead of writing direct API calls.
4. Keep the package organization-neutral and free of destinations or tokens.
5. Update the public indexes and checks.

## Configuration

Token resolution prefers `OCEAN_SLACK_BOT_TOKEN`, then `SLACK_BOT_TOKEN`, then
`OCEAN_SLACK_TOKEN_FILE` (default `~/.slack_token`). Local environment files are
gitignored.
