# couriers

A **class** of agents that all run the same route with the same workflow, and
differ only in *what they ship*. A courier:

1. **resolves** a destination the operator names at run time (a Slack channel link),
2. **confirms** it in human terms (bot identity / channel name) before sending into
   a shared channel,
3. **ships** its payload over the shared route — paced, rate-limit-aware, retried,
   ledgered for clean resume.

Because every courier rides the **same route**, that route is extracted once and
shared. Couriers are thin orchestration on top of it.

## The route

[`transport/slack.py`](transport/slack.py) — the shared Slack transport. Pure
stdlib (no `slack_sdk`/`requests`/curl). Owns: token resolution, link parsing
(`/archives/C…`, `?thread_ts=…&cid=…`, raw id), `auth.test`/`conversations.info`,
message posting, the modern 3-step file upload
(`getUploadURLExternal` → PUT bytes → `completeUploadExternal`), rate-limit +
transient retry, channel threading, and canvas ops (create / edit
replace-append-prepend / best-effort read / list). Every courier species
imports this; none reimplements Slack.

> Prior route: Telegram — retired. [`transport/parked/`](transport/parked) is
> the archive slot for retired routes; the transport shape is identical if one
> ever needs to come back.

## Species

| Courier | Mode | Ships |
|---|---|---|
| [`file-courier`](file-courier) | deterministic | files from a dropbox folder + text messages; deletes files on confirmed upload |
| [`sound-watcher`](sound-watcher) | deterministic | Warner Audio Tracking Sheet sweeps → Slack canvases + one roll-up ping |
| [`pr-babysitter`](pr-babysitter) | agentic | one unattended PR-maintenance pass — fix CI/conflicts, merge safe, report |
| [`mon-email-watcher`](mon-email-watcher) | deterministic | Mon Rovîa Gmail sweeps → Calendar markers + Slack drafts |
| [`mon-distro`](mon-distro) | deterministic | read-first ops window into the Mon Rovia evergreen distro engine |
| [`uiau`](uiau) | deterministic | Us In Another Universe engine — scrape viral clips, cook 9:16 montages to Slack review |
| [`folk-edit`](folk-edit) | agentic | one Stillness wave — moment-select → cut → mantra → Slack review gate |
| [`hopecore`](hopecore) | agentic | earned-hope quotes + Mon Rovîa footage + music → one wave to Slack review |
| [`core-music-editor`](core-music-editor) | agentic | one Core Core acoustic-reel edit wave → Slack review previews |
| [`yt-clipper`](yt-clipper) | agentic | clip-factory autopilot pass — source → clip → Slack QC gate → scoreboard |

Every species' entry is `bin/courier` (declared in its `courier.toml`). Run
`python3 hub/router.py list` for the live slash-command registry — the router
reads the manifests, so this table may lag.

## Adding a courier species

1. Create `couriers/<name>/` with a `courier.toml` manifest (the router
   discovers it — no router edits), its own `CLAUDE.md` (operating protocol),
   and an entry point (`bin/courier` by convention).
2. `import slack` from `../transport` — use `Slack.post_message` / `upload_file`
   / `call`. Do not hand-roll API calls.
3. Keep the species focused on *what* to ship and *how to confirm*; the route
   handles *how to send*.

## Config / secrets

Token resolution (first hit wins): `$OCEAN_SLACK_BOT_TOKEN` → `$SLACK_BOT_TOKEN`
→ file at `$OCEAN_SLACK_TOKEN_FILE` (default `~/.slack_token`). Bot token
(`xoxb-…`) needs `chat:write`, `files:write`, a channel read scope, and
`canvases:write` if the species publishes canvases. Put
shared config in `couriers/courier.env` (gitignored); a species may override in
its own `courier.env`.
