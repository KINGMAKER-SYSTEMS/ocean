#!/usr/bin/env bash
# Supervised launcher for the Ocean daemon (OCEAN-253).
#
# launchd execs the installed COPY of this script at
# ~/.local/libexec/ocean-daemon/launch.sh (rendered plist points there), so a
# dev checkout's working-tree state can never affect supervision (TASK-15).
# The repo copy is the source of truth; the installer refreshes the copy.
# It exec's the PREBUILT release binary with the production env. A supervised
# service must respawn fast and deterministically, so this script does NOT run
# `cargo build` — the binary is built once at install time (see
# ops/install-ocean-daemon.sh) from MAIN, per the operator's build-from-main
# rule. To pick up new code: rebuild from main, then kickstart -k (see ops/README).
#
# cwd is NEUTRAL ($HOME), not the repo. The daemon is workspace-agnostic
# (turns carry their own cwd); its startup guard refuses to boot from inside a
# git repo so unbound fallback turns don't bind to ocean-os (see main.rs,
# OCEAN_ALLOW_REPO_CWD). Pre-guard this mirrored the hand-launch
# (cd <repo> && OCEAN_YOLO=1 ./target/release/ocean-daemon); the guard made that
# cwd invalid, so we run from $HOME instead. BIN is resolved absolutely, so
# repo-cwd isn't needed to find the binary. Override via OCEAN_DAEMON_CWD.
set -euo pipefail

# Toolchain + common bins on PATH (launchd starts with a minimal PATH). The
# daemon shells out to tools (git, ripgrep, etc.) for its own tool calls, so a
# sane PATH matters even though this script doesn't compile anything.
# ${HOME:-} so an unset HOME degrades to a system PATH instead of tripping
# `set -u` before the clearer NEUTRAL_CWD diagnostics below can run.
export PATH="${HOME:-}/.rustup/toolchains/stable-aarch64-apple-darwin/bin:${HOME:-}/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

# TASK-7: the supervised binary is an IMMUTABLE installed artifact, never the
# repo's mutable target/release/ output. A cargo build in the checkout (any
# branch) must not be able to silently become the running daemon at the next
# restart — that is exactly how the -dirty health revs happened. The installer
# copies each build to a versioned path and atomically flips `current`.
BIN="${OCEAN_DAEMON_BIN:-${HOME:-}/.local/libexec/ocean-daemon/current}"
if [[ ! -x "$BIN" ]]; then
  echo "FATAL: no installed daemon at $BIN." >&2
  echo "       Run ops/install-ocean-daemon.sh (from MAIN) to build, install a" >&2
  echo "       versioned artifact, and flip the 'current' symlink atomically." >&2
  echo "       The repo's target/release/ output is deliberately NOT launched." >&2
  exit 127
fi

# Production env. Mirrors the prior hand-launch exactly; override via the plist's
# EnvironmentVariables block or by exporting before launch.
#   OCEAN_YOLO=1            -> operator default: tools run without per-call gating.
#   OCEAN_BIND (optional)   -> defaults to 127.0.0.1:4780 inside the binary.
#   OCEAN_ASSISTANTS_DIR    -> optional; defaults to ~/.config/ocean-rs/assistants.
#   OCEAN_PROMPT_CAPTURE_DIR -> optional owner-only local JSON request captures;
#                               includes private prompt/transcript/tool content.
#   ~/.config/ocean-rs/federation.env -> optional KEY=VALUE file for the
#                               federated-room Bedrock bridge: OCEAN_FEDERATION_URL
#                               (origin only) and, on an owner daemon only,
#                               OCEAN_FEDERATION_OWNER_TOKEN. Parsed as data, never
#                               executed: it must be owned by this user and not
#                               group/other-writable, only OCEAN_FEDERATION_* keys
#                               are read, and values already set (plist or caller)
#                               win. Without it every credentialed room sits in
#                               `recovering`.
export OCEAN_YOLO="${OCEAN_YOLO:-1}"
load_federation_env() {
  local file="$1" mode line key value
  local blank_re='^[[:space:]]*(#|$)'
  local pair_re='^(OCEAN_FEDERATION_[A-Z0-9_]+)=(.*)$'
  local dq_re='^"(.*)"$' sq_re="^'(.*)'$"
  local mode_re='^[0-7]{3,4}$'
  [[ -e "$file" ]] || return 0
  if [[ ! -f "$file" || ! -r "$file" || ! -O "$file" ]]; then
    echo "WARNING: ignoring $file: not a readable regular file owned by this user." >&2
    return 0
  fi
  # Pick the stat dialect explicitly: GNU `stat -f` means --file-system, so a
  # BSD-then-GNU fallback chain would capture filesystem data plus the mode.
  if stat -c %a "$file" >/dev/null 2>&1; then
    mode="$(stat -c %a "$file" 2>/dev/null || true)"
  else
    mode="$(stat -f %Lp "$file" 2>/dev/null || true)"
  fi
  [[ "$mode" =~ $mode_re ]] || mode=777
  if (( 8#$mode & 8#022 )); then
    echo "WARNING: ignoring $file: group/other-writable (mode $mode); chmod 600 it." >&2
    return 0
  fi
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ $blank_re ]] && continue
    line="${line#export }"
    if [[ ! "$line" =~ $pair_re ]]; then
      echo "WARNING: $file: skipped a line that is not OCEAN_FEDERATION_*=value." >&2
      continue
    fi
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    if [[ "$value" =~ $dq_re || "$value" =~ $sq_re ]]; then value="${BASH_REMATCH[1]}"; fi
    [[ -n "${!key+x}" ]] && continue
    export "$key=$value"
  done < "$file"
}
load_federation_env "${HOME:-}/.config/ocean-rs/federation.env"
FEDERATION_STATE=off
if [[ -n "${OCEAN_FEDERATION_URL:-}" ]]; then FEDERATION_STATE=on; fi

# Run from a NEUTRAL cwd so the startup guard's repo-cwd check passes and the
# unbound-turn fallback anchor is harmless (home, not ocean-os).
#
# Guarded explicitly rather than leaning on `${..:-$HOME}` under `set -u`: if
# HOME is unset/empty (LaunchDaemon context, odd session bootstraps) or
# OCEAN_DAEMON_CWD points at a missing dir, fail with a clear FATAL line
# instead of a cryptic bash error inside a 10s KeepAlive crash loop.
NEUTRAL_CWD="${OCEAN_DAEMON_CWD:-${HOME:-}}"
if [[ -z "$NEUTRAL_CWD" ]]; then
  echo "FATAL: no neutral cwd — HOME is unset/empty and OCEAN_DAEMON_CWD is not set." >&2
  echo "       Set OCEAN_DAEMON_CWD to a directory outside any git repo." >&2
  exit 78 # EX_CONFIG
fi
if [[ ! -d "$NEUTRAL_CWD" ]]; then
  echo "FATAL: neutral cwd '$NEUTRAL_CWD' does not exist (check OCEAN_DAEMON_CWD)." >&2
  exit 78 # EX_CONFIG
fi
echo "==> ocean-daemon: cwd=$NEUTRAL_CWD (neutral) bin=$BIN yolo=$OCEAN_YOLO bind=${OCEAN_BIND:-127.0.0.1:4780} federation=$FEDERATION_STATE"
cd "$NEUTRAL_CWD"
exec "$BIN"
