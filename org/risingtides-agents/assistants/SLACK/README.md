# `[SLACK]` surface profile

`system.md` in this directory is the **house Slack profile** the Ocean daemon loads
for any assistant on the Slack surface. It is a **composed artifact — do not edit it
by hand**; edit the sources under `assistants/_base/SLACK/*.md` (and
`assistants/_shared/system.md`) and re-run the composer. See
[`../README.md`](../README.md) for the composition model and authoring flow.

## How a Slack turn actually loads this file

The daemon loads `<root>/SLACK/system.md` for a turn whose `client_type` is
`surface-slack`, where `<root>` comes from **`$OCEAN_ASSISTANTS_DIR`** (falling back
to `~/.config/ocean-rs/assistants`). If that env var doesn't point at this repo's
`assistants/` dir, **this profile never loads** and the bot runs a generic seed
prompt instead — silently.

The inbound→daemon→reply plumbing that sends `client_type = surface-slack`, the
`OCEAN_ASSISTANTS_DIR` mechanism in full, and the outbound `reply.py` path are all
documented in the **Slack bridge reference**:

- [`../bridge/README.md`](../bridge/README.md) — how it works (flow,
  `OCEAN_ASSISTANTS_DIR`, `reply.py`, env vars, tokens).
- [`../bridge/RUN.md`](../bridge/RUN.md) — how to run it (Slack app setup + start).
