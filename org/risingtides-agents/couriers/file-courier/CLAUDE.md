# file-courier — Agent Protocol (Slack route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the deterministic
> harness in `harness/courier.py`, which rides the shared Slack route at
> `../transport/slack.py`. This file is the operating protocol any agent loads
> before running the courier.

## Mission

Deliver everything in `dropbox/` to a Slack channel the operator names **at run
time** (by pasting a channel link). The destination is never hardcoded. Files
are **deleted on confirmed upload**. Can also post text messages.

## Capabilities (the route handles the mechanics — don't reinvent them)

Do **not** hand-roll Slack API calls. Use the harness:

- **`resolve`** — parse a Slack link (or raw channel id), verify the bot token
  via `auth.test`, and look up the channel name via `conversations.info` so you
  can confirm the destination in human terms.
- **`send`** — upload each file in a directory via Slack's modern external-upload
  flow (`getUploadURLExternal` → PUT bytes → `completeUploadExternal`), pace
  uploads, honor rate limits, write a resume `courier.ledger`, and delete each
  file on confirmed delivery.
- **`message`** — post a text message to a channel (optionally threaded).

## Operating protocol (follow in order)

1. **Confirm there's something to send.** List `dropbox/`. If empty, ask the
   operator to drop files in (or point `--dir` elsewhere). Report count + a
   size/type breakdown. Flag very large files (Slack workspace upload limits
   vary by plan; ~1 GB max, but free workspaces are smaller).
2. **Ask for the destination link.** The operator pastes a Slack channel link —
   e.g. `https://workspace.slack.com/archives/C0123ABCD`. Never assume a previous
   destination. **The bot must be a member of the channel** (invite it if not).
3. **Resolve + verify.** Run `bin/courier resolve "<link>"`. Read back the bot
   identity, team, and **channel name**. Show the operator the resolved
   destination in plain language and the file count. **Get an explicit "go"
   before sending** — this posts into a shared workspace channel.
4. **Send.** Run `bin/courier send --channel <id>` (add `--thread-ts <ts>` to
   land in a thread). The harness deletes on success.
5. **Report.** Relay the final `sent=N failed=M` line. The ledger means
   re-running `send` skips delivered files and retries only the rest.

## Hard rules

- **Confirm the destination before sending.** Outward-facing, shared workspace.
- **The bot must be in the channel.** `not_in_channel` → ask the operator to
  `/invite` the bot, then retry.
- **Never hardcode a channel id or token** in code or commits. Token comes from
  env / `courier.env`; destination comes from the operator at run time.
- **Don't bypass the harness.** It owns the upload flow, rate limits, retries,
  ledger, and deletion. Your job is orchestration + confirmation, not transport.

## Secrets & scopes

Bot token (`xoxb-…`) resolution order: `$OCEAN_SLACK_BOT_TOKEN` →
`$SLACK_BOT_TOKEN` → file at `$OCEAN_SLACK_TOKEN_FILE` (default `~/.slack_token`).
Required scopes: `chat:write`, `files:write`, and a channel read scope
(`channels:read` / `groups:read`) for name resolution. Put local config in
`courier.env` (gitignored); see `courier.env.example`.

## Quick reference

```bash
bin/courier resolve "https://workspace.slack.com/archives/C0123ABCD"
bin/courier send --channel C0123ABCD                 # --dir defaults to dropbox/
bin/courier send --channel C0123ABCD --thread-ts 1700000000.000100 --keep
bin/courier message --channel C0123ABCD "deploy finished ✅"
```
