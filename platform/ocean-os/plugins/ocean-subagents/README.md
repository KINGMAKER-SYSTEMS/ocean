# Ocean Subagents

A real Ocean tool plugin that lets an Ocean agent start and manage other Ocean agents. Child workers are ordinary daemon-owned sessions—not direct provider calls or an alternate runtime.

## Tools available inside Ocean

- `plugin__ocean-subagents__spawn` — start a child turn and return immediately.
- `plugin__ocean-subagents__status` — refresh daemon truth and collect output.
- `plugin__ocean-subagents__wait` — wait up to 20 seconds for progress.
- `plugin__ocean-subagents__send` — continue a finished worker session.
- `plugin__ocean-subagents__permissions` — inspect a child's pending permission requests.
- `plugin__ocean-subagents__decide` — resolve a reviewed child permission through its bound secret.
- `plugin__ocean-subagents__cancel` — cancel its active turn.
- `plugin__ocean-subagents__list` — list durable runs.

Every plugin call uses Ocean's existing permission gate. Every child has its own durable Ocean session. The fixed `ocean-subagent-worker` folder agent deliberately excludes the subagent plugin from the child's tool allowlist, preventing recursive fan-out. The daemon keeps every tool when the named agent does not resolve, when its allowlist is empty, or when its allowlist matches no tool that exists (names are exact), and it adds an agent's subprocess-capability tools after narrowing. So `spawn` and `send` first confirm the worker profile resolves, names at least one built-in tool every daemon has, names no subagent tool, and binds no subprocess capability, and refuse otherwise.

## Install

```bash
plugins/ocean-subagents/install.sh
launchctl kickstart -k "gui/$(id -u)/dev.risingtides.ocean-daemon"
```

Then ask Ocean:

```text
Use the ocean-subagents plugin to start a reviewer for this repository. Wait for it and report its findings.
```

Approve the subagent plugin calls when Ocean asks. Child mutating tools remain separately permission-gated: Ocean first calls `permissions`, then calls `decide` with the reviewed permission id and matching tool name. The plugin binds each child turn to a private decision token and refuses cross-run or tool-name-mismatched decisions.

## Runtime behavior

- Maximum four active children per plugin instance; `send` counts against the same cap.
- Default elapsed-time ceiling: 600 seconds; configurable per spawn from 30–1800 seconds.
- `model` is passed to the daemon as given; omit it to use the daemon's current model. A name the daemon cannot route fails the child turn, and the failure lists the ready catalog ids. `send` reuses the run's model, so recover with a new `spawn`.
- `thinking_level` (`off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`) sets the child's reasoning effort and is reused by `send`; omit it for the model's default.
- State: `$OCEAN_SUBAGENT_STATE_DIR/runs.json`, `$XDG_STATE_HOME/ocean/subagents/runs.json`, or `~/.local/state/ocean/subagents/runs.json`. The 200 most recently finished runs are kept; active runs are never pruned.
- Daemon URL: `$OCEAN_DAEMON_URL`, default `http://127.0.0.1:4780`.
- Child default cwd: `$OCEAN_SUBAGENT_DEFAULT_CWD`, otherwise `$HOME`; callers should normally pass their workspace path.
- The daemon launches plugins with a cleared environment (only `PATH` and the plugin directory as cwd), so the three variables above apply when the plugin is started by hand or by a host that sets them. A daemon-launched plugin uses the defaults, including the default daemon URL.
- Completed output is projected from the child's daemon-owned transcript and capped at 24,000 bytes.
- The daemon's request registry is in memory. A run whose request is gone (daemon restart, or a finished turn evicted after an hour) settles as `lost`; its output is whatever the child wrote in that turn, never an earlier turn's answer, and `send` continues it. A cancel the daemon refuses settles the run the same way instead of leaving it `cancelling`. A run settles even when its session cannot be read at that moment; a later `status` collects the output.
- `wait` returns at once for a permission prompt it has not reported yet, including one raised between two waits, and keeps its full budget for a prompt already reported.

This is at-least-once local lifecycle metadata around daemon-owned turns. The daemon remains authoritative for execution and session state.

## Verify

```bash
python3 -m unittest plugins/ocean-subagents/test_ocean_subagents.py
python3 plugins/ocean-subagents/test_wire.py
python3 -m py_compile plugins/ocean-subagents/ocean-subagents.py
sh -n plugins/ocean-subagents/install.sh
```
