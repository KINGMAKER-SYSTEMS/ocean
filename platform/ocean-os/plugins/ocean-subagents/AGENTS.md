# ocean-subagents — Ocean Tool Plugin

## Purpose

Provide working, permission-gated subagent tools inside ordinary Ocean turns by dispatching bounded child turns through the existing daemon agent APIs.

## Ownership

- `ocean-subagents.py` owns plugin JSON-RPC, durable run metadata, lifecycle refresh, output projection, follow-up turns, token-bound child permission decisions, cancellation, and elapsed-time watchdogs.
- `plugin.toml` owns the model-visible tool contract.
- `agent/ocean-subagent-worker/` owns the fixed child profile that excludes the subagent plugin and prevents recursive delegation.
- `install.sh` installs only this plugin and fixed worker agent under the Ocean config root.

## Local Contracts

- Use `POST /v1/agent/turns`; never call a provider directly.
- Spawn returns immediately with durable run, turn, and session identifiers.
- Child execution remains an ordinary Ocean session with normal tool permissions; every child turn carries a private decision token, and the plugin accepts permission decisions only for the exact run/request/session/tool tuple.
- Enforce fixed worker-profile binding, maximum four active runs, bounded output, and an elapsed-time watchdog.
- The recursion guard fails closed: before every child turn, confirm the worker profile resolves in the daemon with a non-empty allowlist (`config.tools` plus `tools/`) that names no subagent tool. An unresolved named agent or an empty allowlist runs with every tool.
- Persist metadata atomically under `~/.local/state/ocean/subagents` by default. Refreshing an unchanged run writes nothing; finished runs are settled and only the newest 200 are retained.
- Do not claim exactly-once execution; daemon request/session truth wins during refresh.
- No path may leave a run active once the daemon cannot finish it. The request registry is volatile, so an active run whose request is absent settles as `lost` from session truth after a short grace, and a cancel answered `ok:false` settles from daemon truth. The watchdog retries a failed attempt because plugins start before the daemon's listener binds.
- `--check` reads state only; it must never start watchdogs or call the daemon.

## Work Guidance

- Keep the implementation Python-standard-library only.
- Stdout is exclusively JSON-RPC; diagnostics go to stderr.
- Keep names, descriptions, and schemas identical between `plugin.toml` and live `list_tools`.
- The test daemon mirrors the real control contract (volatile registry, HTTP 200 `ok:false` cancel refusals, `cancelling` before `cancelled`, 404 for an unknown session). Change it only alongside the daemon.

## Verification

- `python3 -m unittest plugins/ocean-subagents/test_ocean_subagents.py`
- `python3 plugins/ocean-subagents/test_wire.py`
- `python3 -m py_compile plugins/ocean-subagents/ocean-subagents.py`
- `sh -n plugins/ocean-subagents/install.sh`

## Child devlog Index

No child devlogs.
