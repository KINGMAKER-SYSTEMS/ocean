# sound-watcher — Agent Protocol (Warner tracker route)

> An Ocean courier. The *brain* is provider-agnostic (DeepSeek / Anthropic /
> OpenAI / Google — whatever the runtime is). The *hands* are the production
> Warner Sound Watcher engine at `~/Projects/active/warner-sound-watcher`
> (override via `$OCEAN_SOUND_WATCHER_ENGINE_DIR`), reached through the thin
> wrapper in `bin/courier`. This file is the operating protocol any agent loads
> before running the courier.

## Mission

Mirror **new** TikTok sound links from the *Audio Tracking* tab of the Rising
Tides × Warner Records Tracker Google Sheet into Slack `#warner-ugc-sounds`:
each new link is appended to its page-style canvas (silent backup) and the team
gets **one** consolidated roll-up ping per cycle pointing at the Sound Board
webpage. The engine already runs this hourly under launchd — this courier is
the **on-demand identity**: sweep now, or preview what the next sweep would do.

## Capabilities (the engine handles the mechanics — don't reinvent them)

Do **not** hand-roll Sheets reads, Slack API calls, or diff logic. Use the
wrapper:

- **`preview`** — the engine's own dry-run (`watcher.py --dry-run`): reads the
  sheet via the already-authenticated `gws` CLI, diffs against `state.json`,
  and prints the new links and the roll-up ping that WOULD post. No Slack
  writes, no board publish, no state change. Safe to run any time.
- **`sweep`** — the engine's real production entry (`run.sh`, exactly what the
  hourly launchd job runs): one watcher cycle (poll → diff → append canvases →
  one roll-up ping → save state), then a Sound Board rebuild + push to GitHub
  Pages. This posts into a shared team channel and pushes a live webpage.

Engine one-timers (`bootstrap.py`, `reset.py`, `install-launchd.sh`) are
**deliberately not exposed** as slash commands — they create/destroy canvases
and state. If setup work is ever needed, it happens in the engine dir with the
operator, per the engine's own AGENTS.md.

## Operating protocol (follow in order)

1. **Preview first when the operator's intent is "what's new?"** Run
   `bin/courier preview` and relay the new links (`Title — Artist · page
   style`) and the roll-up text. Many requests end here — a preview answers
   "anything new on the tracker?" without touching Slack.
2. **Confirm before a real sweep.** A sweep posts a `@channel` roll-up into
   `#warner-ugc-sounds` and republishes the public Sound Board. If the hourly
   launchd job is due imminently anyway, say so — an on-demand sweep is for
   "the team needs this now", not routine.
3. **Sweep.** Run `bin/courier sweep`. The engine sources its own Slack bot
   env, reads the sheet, diffs, writes, saves state, and republishes the board.
4. **Report.** Relay the cycle summary from the engine log output (`N new
   link(s), M new section(s)`) plus the Sound Board URL from the roll-up. If
   the engine printed `unrecognized header(s): [...]`, surface it — that's a
   new sheet section needing a `config.json` `page_styles` entry (operator
   decision, in the engine dir).
5. **On failure, stop and report.** A non-zero exit means the cycle aborted
   (e.g. missing `state.json`, sheet read failure). Never "fix" state to make
   a run pass — see Hard rules.

## Hard rules

- **The engine stays in place and the launchd job stays live.** This package
  never moves, edits, or reinstalls the engine or its schedule. On-demand
  invocation only.
- **Additive diff only — never break `state.json`.** It is the single source
  of what's already been posted; only *new* links notify, and edits/removals
  just stop appearing. Never hand-edit, truncate, or delete it, and never make
  removals trigger messages. Corrupting state = duplicate `@channel` pings to
  the whole team.
- **One `@channel` ping per cycle, no matter how many links land.** The cap
  lives in the engine (`notify_mention` in `config.json`). Don't work around
  it with extra messages.
- **Never run `bootstrap.py` or `reset.py` from this courier.** They
  create/delete canvases and state — operator-supervised, engine-dir-only.
- **Never hardcode tokens, channel ids, or the spreadsheet id** in code or
  commits. Reads ride the authenticated `gws` CLI; writes use the bot token
  env file the engine's `config.json` names. Nothing sensitive lives in this
  package.
- **Don't bypass the wrapper/engine.** The engine owns sheet parsing (incl.
  tolerant alias matching for hand-typed headers), the diff, rate-limited
  Slack writes, atomic state saves, and the board publish. Your job is
  orchestration + confirmation, not transport.

## Secrets & scopes

Nothing to configure in this package. Sheet **reads** go through the
already-authenticated `gws` CLI (no new OAuth). Slack **writes** use the
`eric-claude` bot token, loaded by the engine itself as `SLACK_BOT_TOKEN` from
the env file named in the engine's `config.json` (`slack_env`); required
scopes are `canvases:write` and `chat:write`. Reference tokens by env-var NAME
only — never inline values here, in `courier.env`, or in commits. Local
overrides (e.g. `OCEAN_SOUND_WATCHER_ENGINE_DIR`) go in `courier.env`
(gitignored); see `courier.env.example`.

## Quick reference

```bash
bin/courier preview        # dry-run: what would the next sweep post? (safe)
bin/courier sweep          # real cycle + Sound Board republish (confirm first)
bin/courier help           # usage

# via the hub
python3 ../hub/router.py run /sounds-preview
python3 ../hub/router.py run /sounds

# engine-side observability (read-only)
tail -20 "$OCEAN_SOUND_WATCHER_ENGINE_DIR/watcher.log"
```
