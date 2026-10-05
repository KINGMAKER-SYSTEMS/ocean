# Ocean Surface — Ops

## ocean-surface-proxy is supervised by launchd (OCEAN-161 / OCEAN-385)

The **surface proxy** (`crates/ocean-surface-proxy`) executes the immutable proxy
paired with the compiled PWA release selected
by `~/.config/ocean-surface/current` and reverse-proxies `/v1/*` to the Ocean
daemon. It listens on **`0.0.0.0:8790`** by default.

Two launchd LaunchAgents own the live surface: the proxy respawns on crash and
starts at login; the deploy watcher polls `origin/main` every two minutes and
promotes a new release only after a clean detached-main build passes its gates.

| Thing | Value |
|---|---|
| Proxy launchd label | `dev.risingtides.ocean-surface-proxy` |
| Deploy launchd label | `dev.risingtides.ocean-surface-auto-deploy` |
| Version-controlled plists | `deploy/dev.risingtides.ocean-surface-*.plist` |
| Installed launchers | `~/.config/ocean-surface/bin/ocean-surface-{proxy,auto-deploy}.sh` plus `surface-release.sh` |
| Installed plist path | `~/Library/LaunchAgents/dev.risingtides.ocean-surface-*.plist` |
| Bind address | `0.0.0.0:8790` (env `OCEAN_SURFACE_BIND`) |
| Bundle served | `~/.config/ocean-surface/current` (atomic symlink; repo `dist/` is dev-only) |
| Paired release | `~/.config/ocean-surface/releases/<revision>-proxy/{bin,dist}` |
| Proxy provenance | Release `source-revision` and `proxy-sha256`; fixed fields in `/health` |
| Deployed revision | `~/.config/ocean-surface/deployed-rev` |
| Daemon proxied to | `http://127.0.0.1:4780` (env `OCEAN_DAEMON_URL`) |
| Auth env file | `~/.config/ocean-surface/proxy-auth.env` (0600; sourced by the launcher) |
| Logs | `/private/tmp/ocean-surface-{proxy,auto-deploy}.log` |

> The proxy serves a **prebuilt immutable release** — it does not build on
> respawn. The deploy watcher fetches `origin/main`, builds in a disposable
> detached worktree, runs the WASM check/tests/strict Clippy/format gate, builds
> the release WASM and proxy, validates the wasm magic and release HTML, then
> selects one immutable executable/bundle pair and advances `deployed-rev` after
> the restart health check. Sync/restart failure restores the previous selection,
> client assets and supervision files; a failed recovery reports listening as
> unverified. A local `trunk serve` / `run-surface.sh`
> loop cannot touch the live release.
>
> Secrets stay out of the plist and releases. Voice/provider credential authority
> stays in the daemon. Operator login is
> **on by default** and requires operator-supplied creds in
> `~/.config/ocean-surface/proxy-auth.env` (0600) exporting
> `OCEAN_SURFACE_AUTH=on`, `OCEAN_SURFACE_USER`, and `OCEAN_SURFACE_PASS`
> (plus `OCEAN_SURFACE_COOKIE_SECURE=on` behind the public HTTPS tunnel).
> There are **no**
> built-in operator credentials. The tracked plist template does **not** set
> `OCEAN_SURFACE_AUTH`; do not put USER/PASS in the plist. For a trusted
> localhost throwaway only, export `OCEAN_SURFACE_AUTH=off` in the process
> environment (never commit that override into the template).

### Install / enable supervision

```bash
# Stage only — builds/stages HEAD, client assets and supervision files:
ops/install-surface-proxy.sh

# Build/promote and start both supervised jobs now:
ops/install-surface-proxy.sh --bootstrap
```

The installer hard-fails unless HEAD is `main` or detached exactly at
`origin/main`; `--allow-non-main` is the explicit escape hatch. It builds the
proxy and release bundle, seeds the immutable release store and `current`
symlink, copies both launchers out of the mutable shared checkout, validates
both plist files, then either prints the launchctl commands or bootstraps both
jobs. Stage-only invokes no launchctl commands and does not prove what an
already-running proxy serves. Bootstrap starts enabled jobs even at the same
source revision. Named operator-disabled or malformed overrides block before
config/selection changes or bootout; no enable/disable overrides are written.
Failure restores each job's exact prior loaded/cold state and checks listening
only for a previously loaded proxy. Both templates are rendered for the active home and physical Surface
component, including `apps/ocean-surface/` in the monorepo. Paths are serialized
as plist strings so spaces and XML-sensitive characters remain valid. Installed
jobs use that component's target cache; credentials remain in the existing auth
file. Promoting the built component `dist/` preserves its contents, and a failed
client sync or proxy restart restores the previous selection, revision marker,
client copies and installed launchers/plists. Incomplete recovery preserves the
remaining owner-only backup, reports its path as unverified and keeps the original
failure code. The installer checks committed, clean source before and after build
and refuses a changed HEAD before stamping provenance. The installer and explicit
`--promote BUNDLE REVISION [PROXY_BINARY]` use the same lock as scheduled ticks.
Known releases compare health revision/checksum against the selected pair;
same-revision ticks repair missing or stale listeners without rebuilding.

Before building over a legacy mutable proxy, the installer captures its exact
bytes from the prior installed component path. `OCEAN_SURFACE_LEGACY_PROXY`
can identify a nonstandard prior executable explicitly. Missing recovery bytes
block before build/selection. Legacy recovery keeps source revision `unknown`;
the old bundle marker cannot prove the executable's source. Both old and new
launchers can use the retained legacy selection. Later main revisions deploy
automatically once supervision is enabled.

Deployment still requires compatibility with the active daemon, actual running
binary provenance and the served WASM hash. Neither stage-only nor fixture
success establishes live delivery. `OCEAN_SURFACE_OWNER_HOME` is available for
non-live fixtures; production paths continue to default to the real owner home.

### Check status

```bash
# Is it listening?
lsof -nP -iTCP:8790 -sTCP:LISTEN

# launchd's view:
launchctl print gui/$(id -u)/dev.risingtides.ocean-surface-proxy | grep -E 'state|pid|last exit'
launchctl print gui/$(id -u)/dev.risingtides.ocean-surface-auto-deploy | grep -E 'state|pid|last exit'

# Exact revision selected by the live symlink:
cat ~/.config/ocean-surface/deployed-rev
readlink ~/.config/ocean-surface/current

# Unauthenticated health endpoint:
curl -fsS http://127.0.0.1:8790/health && echo
```

### Restart / read logs

```bash
# Force a proxy restart:
launchctl kickstart -k gui/$(id -u)/dev.risingtides.ocean-surface-proxy

# Trigger an immediate main check/deployment:
launchctl kickstart -k gui/$(id -u)/dev.risingtides.ocean-surface-auto-deploy

# Tail logs:
tail -f /private/tmp/ocean-surface-proxy.log
tail -f /private/tmp/ocean-surface-auto-deploy.log
```

### Uninstall / stop supervision

```bash
ops/uninstall-surface-proxy.sh
```

Boots both jobs out of launchd and removes both installed plist files. The repo,
built artifacts, and immutable deployed releases are left untouched.

> **Note on the daemon:** the Ocean **daemon** (`:4780`) is separate from these
> surface LaunchAgents. Re-verify supervision with
> `launchctl list | grep -i ocean` instead of assuming process state.
