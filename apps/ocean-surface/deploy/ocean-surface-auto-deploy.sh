#!/usr/bin/env bash
# Keep the operator's live Ocean Surface pinned to a verified origin/main build.
#
# launchd runs this idempotently. A new main revision is built in a disposable
# detached worktree; the live `current` symlink and deployed-rev marker move only
# after every gate and bundle validation pass. Failures preserve the last-good
# release. `--promote DIR REV` exercises just the atomic promotion contract.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/surface-release.sh"

SOURCE_DIR="${OCEAN_SURFACE_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
STATE_DIR="${OCEAN_SURFACE_STATE_DIR:-$OWNER_HOME/.config/ocean-surface}"
RELEASES_DIR="$STATE_DIR/releases"
CURRENT_LINK="$STATE_DIR/current"
MARKER="$STATE_DIR/deployed-rev"
WORKTREE_ROOT="${OCEAN_SURFACE_WORKTREE_ROOT:-/private/tmp/ocean-surface-auto-deploy}"
PROXY_LABEL="dev.risingtides.ocean-surface-proxy"
TAURI_LABEL="dev.ocean.surface-tauri"
TAURI_STALE_MARKER="$STATE_DIR/tauri-stale"
# TASK-87: a RESTART picks up new web assets (frontendDist), but NOT changes to
# the shell's own Rust code — that needs `cargo tauri build`, which this rail
# deliberately does not run (it must never rebuild over a running app, and a
# release bundle build is minutes long). When the shell's source changes we
# therefore record a distinct marker: staleness that a restart CANNOT clear.
# This exists because TASK-78 and TASK-85 (both native exec fixes) landed and
# were announced while /Applications/Ocean.app kept running the old binary for
# hours — "landed" and "deployed" are different claims for this crate.
TAURI_REBUILD_MARKER="$STATE_DIR/tauri-rebuild-required"
DOMAIN="gui/$(id -u)"

export PATH="$OWNER_HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$OWNER_HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

is_surface_dir() {
  [[ -f "$1/Cargo.toml" && -f "$1/Trunk.toml" && -f "$1/crates/ocean-surface-ui/Cargo.toml" ]]
}

# OCEAN_SURFACE_REPO accepts either the Git root or the Surface component.
# Keep Git operations at the root, but build and sync only this component.
REPO="$(git -C "$SOURCE_DIR" rev-parse --show-toplevel 2>/dev/null)" || fail "not a Git repository: $SOURCE_DIR"
if is_surface_dir "$SOURCE_DIR"; then
  SURFACE_DIR="$(cd "$SOURCE_DIR" && pwd -P)"
elif is_surface_dir "$REPO/apps/ocean-surface"; then
  SURFACE_DIR="$REPO/apps/ocean-surface"
else
  fail "cannot find the Ocean Surface component in $SOURCE_DIR"
fi
SURFACE_PREFIX="$(git -C "$SURFACE_DIR" rev-parse --show-prefix)"

validate_bundle() {
  local dir="$1"
  dir="$(cd "$dir" && pwd -P)" || fail "bundle directory missing: $dir"
  [[ -f "$dir/index.html" ]] || fail "bundle has no index.html: $dir"

  local wasm_files=()
  while IFS= read -r -d '' wasm; do
    wasm_files+=("$wasm")
  done < <(find "$dir" -maxdepth 1 -type f -name '*_bg.wasm' -print0)
  (( ${#wasm_files[@]} == 1 )) || fail "bundle must contain exactly one *_bg.wasm: $dir"

  local magic
  magic="$(od -An -tx1 -N4 "${wasm_files[0]}" | tr -d '[:space:]')"
  [[ "$magic" == "0061736d" ]] || fail "bundle wasm is corrupt (magic=$magic): ${wasm_files[0]}"

  if grep -qE '127\.0\.0\.1:8080|localhost:8080|__trunk_address__' "$dir/index.html"; then
    fail "release bundle contains a Trunk development endpoint: $dir/index.html"
  fi
}

proxy_healthy() (
  local bind="${OCEAN_SURFACE_BIND:-0.0.0.0:8790}"
  resolve_proxy_release "$CURRENT_LINK"
  curl -fsS --max-time 2 "http://127.0.0.1:${bind##*:}/health" | python3 -c '
import json,sys
v=json.load(sys.stdin)
revision,checksum=sys.argv[1:]
healthy=v.get("ok") is True and v.get("service")=="ocean-surface-proxy"
# A captured legacy binary predates provenance reporting. Its source remains
# explicitly unknown; recovery checks listening, never invents a revision.
paired=revision=="unknown" or (v.get("release_revision")==revision and v.get("proxy_sha256")==checksum)
sys.exit(0 if healthy and paired else 1)
' "$SURFACE_PROXY_REVISION" "$SURFACE_PROXY_HASH" 2>/dev/null
)

bootstrap_label() {
  local label="$1" attempt plist="$OWNER_HOME/Library/LaunchAgents/$1.plist"
  [[ -f "$plist" ]] || return 1
  for attempt in 1 2 3 4 5; do
    launchctl bootstrap "$DOMAIN" "$plist" && return 0
    (( attempt == 5 )) && return 1
    sleep 1
  done
}

kickstart_proxy() {
  launchctl kickstart -k "$DOMAIN/$PROXY_LABEL" || return $?
  local attempt
  for attempt in 1 2 3 4 5; do
    if proxy_healthy; then
      return 0
    fi
    (( attempt == 5 )) && return 1
    sleep 1
  done
}

restart_proxy() {
  [[ "${OCEAN_SURFACE_NO_RESTART:-0}" == "1" ]] && return 0
  if [[ "${OCEAN_SURFACE_BOOTSTRAP:-0}" == 1 ]]; then
    local label
    for label in "$PROXY_LABEL" dev.risingtides.ocean-surface-auto-deploy; do
      launchctl bootout "$DOMAIN/$label" 2>/dev/null || true
      bootstrap_label "$label" || return 1
    done
  fi
  kickstart_proxy
}

recover_supervision() {
  local backup="$1" label status=0
  [[ -f "$backup/supervision" ]] || return 1
  for label in "$PROXY_LABEL" dev.risingtides.ocean-surface-auto-deploy; do
    launchctl bootout "$DOMAIN/$label" 2>/dev/null || true
    if launchctl print "$DOMAIN/$label" >/dev/null 2>&1; then
      status=1
      continue
    fi
    if [[ -f "$backup/loaded-$label" ]]; then
      bootstrap_label "$label" || status=1
      launchctl print "$DOMAIN/$label" >/dev/null 2>&1 || status=1
    fi
  done
  if [[ -f "$backup/loaded-$PROXY_LABEL" ]]; then
    kickstart_proxy || status=1
  fi
  return "$status"
}

# Restart Tauri only if it is not currently running — never kill an active
# operator session. When Tauri IS running, leave a staleness marker so the
# next Tauri start (or a future surface-side check) can act on it.
# TASK-87: record when the shell's own source changed since the last deploy, so
# an owed `cargo tauri build` is VISIBLE instead of silent. Compares the
# previously deployed revision against the new one; on the first deploy (no
# prior marker) it says nothing rather than crying wolf.
note_tauri_rebuild_needed() {
  local prev="$1" revision="$2"
  [[ -n "$prev" ]] || return 0
  [[ "$prev" == "$revision" ]] && return 0
  # Only ask for a rebuild when the shell's own sources moved. Frontend-only
  # changes are covered by the dist sync + restart above.
  local changed
  changed="$(git -C "$REPO" diff --name-only "$prev" "$revision" -- "${SURFACE_PREFIX}crates/ocean-tauri" 2>/dev/null || true)"
  if [[ -n "$changed" ]]; then
    printf '%s\n' "$revision" > "$TAURI_REBUILD_MARKER"
    echo "TAURI: ${SURFACE_PREFIX}crates/ocean-tauri changed ($prev -> $revision) — REBUILD REQUIRED"
    echo "TAURI: a restart will NOT pick this up; run ${SURFACE_PREFIX}scripts/rebuild-tauri-app.sh"
  fi
}

maybe_restart_tauri() {
  [[ "${OCEAN_SURFACE_NO_RESTART:-0}" == "1" || "${OCEAN_SURFACE_BOOTSTRAP:-0}" == "1" ]] && return 0
  local pid revision="${1:-}"
  pid="$(launchctl list "$TAURI_LABEL" 2>/dev/null | awk 'NR>1{print $1}' || true)"
  if [[ -z "$pid" || "$pid" == "-" ]]; then
    launchctl kickstart -k "$DOMAIN/$TAURI_LABEL" || true
    rm -f "$TAURI_STALE_MARKER"
  else
    printf '%s\n' "$revision" > "$TAURI_STALE_MARKER"
    echo "TAURI: running as pid $pid — staleness marker set, restart deferred"
  fi
}

rebuild_extension() {
  local deployed_dist="$1"
  local ext_dir="$SURFACE_DIR/extension"
  [[ -d "$ext_dir" && -f "$ext_dir/sidepanel.html" ]] || return 0

  local ext_dist="$ext_dir/dist"
  rm -rf "$ext_dist"
  mkdir -p "$ext_dist"
  # Trunk --release produces hashed names (ocean-surface-ui-HASH.js etc.);
  # map them to the stable names sidepanel.html expects.
  local js_file wasm_file
  js_file="$(ls "$deployed_dist"/ocean-surface-ui*.js 2>/dev/null | grep -v '_bg.wasm' | head -1 || true)"
  wasm_file="$(ls "$deployed_dist"/ocean-surface-ui*_bg.wasm 2>/dev/null | head -1 || true)"
  if [[ -z "$js_file" || ! -f "$js_file" ]] || [[ -z "$wasm_file" || ! -f "$wasm_file" ]]; then
    echo "EXTENSION: skipping — no wasm-bindgen files in $deployed_dist"
    return 0
  fi
  cp "$js_file"   "$ext_dist/ocean-surface-ui.js"
  cp "$wasm_file" "$ext_dist/ocean-surface-ui_bg.wasm"
  # Trunk --release produces hashed CSS names (tokens-HASH.css etc.);
  # sidepanel.html references stable names (tokens.css etc.). Strip the hash.
  local css base stable
  for css in "$deployed_dist"/*.css; do
    [[ -f "$css" ]] || continue
    base="$(basename "$css")"
    # tokens-b44329ae8bc1c369.css -> tokens.css (strip hash suffix)
    stable="${base%-*}.css"
    cp "$css" "$ext_dist/$stable"
  done
  if [[ -d "$deployed_dist/fonts" ]]; then
    mkdir -p "$ext_dist/fonts"
    cp "$deployed_dist/fonts"/* "$ext_dist/fonts/"
  fi
  for f in "$deployed_dist"/*.png "$deployed_dist"/*.webmanifest; do
    [[ -e "$f" ]] && cp "$f" "$ext_dist/" || true
  done
  echo "EXTENSION: rebuilt from $deployed_dist"
}

promote_bundle() (
  local source="$1"
  local revision="$2"
  local binary="$3"
  # TASK-87: capture the OUTGOING revision before $MARKER is overwritten below.
  # Reading it afterwards always yields the incoming one, which silently
  # disables the shell-rebuild detector — caught by actually running a promote,
  # not by the source-assertion test, which passed while it was broken.
  local previous_revision=""
  [[ -f "$MARKER" ]] && previous_revision="$(cat "$MARKER" 2>/dev/null || true)"
  [[ "$revision" =~ ^[0-9A-Za-z._-]+$ ]] || fail "unsafe revision: $revision"
  validate_bundle "$source"
  [[ -x "$binary" ]] || fail "proxy binary missing: $binary"
  capture_legacy_proxy
  mkdir -p "$RELEASES_DIR"

  # Inject freshness marker before atomic promotion so every release carries its
  # own identity. Surfaces read /.deploy-sha to detect staleness.
  local release="$RELEASES_DIR/$revision-proxy"
  local staged="$RELEASES_DIR/.${revision}.$$"
  local next_link="$STATE_DIR/.current.$$"
  local next_marker="$STATE_DIR/.deployed-rev.$$"
  local previous_link="" selected=0 marker_existed=0
  local backup="${OCEAN_SURFACE_CLIENT_BACKUP:-}" owns_backup=0
  if [[ -z "$backup" ]]; then
    snapshot_clients
    backup="$CLIENT_BACKUP"
    owns_backup=1
  fi
  [[ -f "$backup/complete" ]] || fail "client recovery snapshot is incomplete"
  snapshot_supervision "$backup"
  [[ -L "$CURRENT_LINK" ]] && previous_link="$(readlink "$CURRENT_LINK")"
  [[ -f "$MARKER" ]] && marker_existed=1
  cleanup_promotion() {
    local status=$? selection_restored=1
    RECOVERY_FAILED=0
    set +e
    if (( status != 0 )); then
      restore_clients "$backup" || retain_recovery "$backup" "client/config restoration incomplete"
    fi
    if (( status != 0 && selected == 1 )); then
      if [[ -n "$previous_link" ]]; then
        if ! { ln -s "$previous_link" "$next_link" && python3 -c 'import os,sys; os.replace(sys.argv[1], sys.argv[2])' "$next_link" "$CURRENT_LINK"; }; then
          selection_restored=0
          retain_recovery "$backup" "prior selection restoration incomplete"
        fi
      else
        rm -f "$CURRENT_LINK" || { selection_restored=0; retain_recovery "$backup" "failed selection removal incomplete"; }
      fi
      if (( marker_existed == 1 )); then
        { printf '%s\n' "$previous_revision" > "$next_marker" && mv -f "$next_marker" "$MARKER"; } || retain_recovery "$backup" "prior marker restoration incomplete"
      else
        rm -f "$MARKER" || retain_recovery "$backup" "failed marker removal incomplete"
      fi
      restore_legacy_proxy || retain_recovery "$backup" "prior legacy executable restoration incomplete"
      if (( selection_restored == 0 || RECOVERY_FAILED == 1 )); then
        echo "RECOVERY: prior restoration unverified; supervision recovery skipped" >&2
      elif [[ "${OCEAN_SURFACE_BOOTSTRAP:-0}" == 1 ]]; then
        recover_supervision "$backup" || retain_recovery "$backup" "prior supervision/listening unverified"
      elif [[ -n "$previous_link" ]]; then
        restart_proxy || retain_recovery "$backup" "prior listening unverified"
      fi
    fi
    rm -rf "$staged"
    rm -f "$next_link" "$next_marker"
    if (( RECOVERY_FAILED == 0 )) && { (( status == 0 )) || (( owns_backup == 1 )); }; then
      rm -f "$backup/recovery-pending" || retain_recovery "$backup" "backup cleanup incomplete"
    fi
    (( owns_backup == 0 )) || finish_client_backup "$backup"
    exit "$status"
  }
  trap cleanup_promotion EXIT
  rm -rf "$staged"
  rm -f "$next_link" "$next_marker"

  if [[ ! -d "$release" ]]; then
    mkdir -p "$staged/dist" "$staged/bin"
    rsync -a --delete "$source/" "$staged/dist/"
    printf '%s\n' "$revision" > "$staged/dist/.deploy-sha"
    cp "$binary" "$staged/bin/ocean-surface-proxy"
    printf '%s\n' "$revision" > "$staged/source-revision"
    proxy_hash "$staged/bin/ocean-surface-proxy" > "$staged/proxy-sha256"
    validate_bundle "$staged/dist"
    validate_release "$staged"
    mv "$staged" "$release"
  else
    validate_bundle "$release/dist"
    validate_release "$release"
    [[ "$(cat "$release/source-revision")" == "$revision" ]] || fail "release source revision mismatch"
  fi

  # Copy from the validated release, never from a dist directory that this
  # sync modifies. Installation promotes that same component dist in place.
  # Complete client preparation before advancing the live release or marker.
  local repo_dist="$SURFACE_DIR/dist"
  mkdir -p "$repo_dist"
  rsync -a --delete "$release/dist/" "$repo_dist/"
  validate_bundle "$repo_dist"
  rebuild_extension "$release/dist"

  ln -s "releases/$revision-proxy/dist" "$next_link"
  python3 -c 'import os,sys; os.replace(sys.argv[1], sys.argv[2])' "$next_link" "$CURRENT_LINK"
  selected=1
  restart_proxy
  printf '%s\n' "$revision" > "$next_marker"
  mv -f "$next_marker" "$MARKER"
  note_tauri_rebuild_needed "$previous_revision" "$revision"
  maybe_restart_tauri "$revision"

  echo "DEPLOYED: $revision -> $CURRENT_LINK"
)

if acquire_release_lock; then
  :
else
  status=$?
  [[ $# == 0 && "$status" == 75 ]] && exit 0
  exit "$status"
fi

worktree=""
cleanup() {
  local status=$?
  if [[ -n "$worktree" ]]; then
    git -C "$REPO" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  fi
  release_lock
  exit "$status"
}
trap cleanup EXIT INT TERM

if [[ "${1:-}" == "--promote" ]]; then
  (( $# == 3 || $# == 4 )) || fail "usage: $0 --promote BUNDLE_DIR REVISION [PROXY_BINARY]"
  promote_bundle "$2" "$3" "${4:-${OCEAN_SURFACE_PROXY_BIN:-$SURFACE_DIR/target/release/ocean-surface-proxy}}"
  exit 0
fi
(( $# == 0 )) || fail "usage: $0 [--promote BUNDLE_DIR REVISION [PROXY_BINARY]]"

if [[ -n "${OCEAN_SURFACE_TARGET_REV:-}" ]]; then
  target="$OCEAN_SURFACE_TARGET_REV"
else
  git -C "$REPO" fetch origin main --quiet
  target="$(git -C "$REPO" rev-parse origin/main)"
fi

capture_legacy_proxy
if [[ -f "$MARKER" && "$(tr -d '[:space:]' < "$MARKER")" == "$target" && -L "$CURRENT_LINK" && -f "$RELEASES_DIR/$target-proxy/source-revision" ]]; then
  current_release="$CURRENT_LINK"
  validate_bundle "$current_release"
  resolve_proxy_release "$current_release"
  [[ "$SURFACE_PROXY_REVISION" == "$target" ]] || fail "selected proxy revision does not match deployed marker"
  if [[ "${OCEAN_SURFACE_NO_RESTART:-0}" != 1 ]] && ! proxy_healthy; then
    restart_proxy
  fi
  echo "CURRENT: $target"
  exit 0
fi

worktree="$WORKTREE_ROOT-$target"
git -C "$REPO" worktree remove --force "$worktree" >/dev/null 2>&1 || true
rm -rf "$worktree"
git -C "$REPO" worktree add --detach "$worktree" "$target" --quiet

worktree_surface="$worktree/$SURFACE_PREFIX"
is_surface_dir "$worktree_surface" || fail "target revision has no Ocean Surface component at $SURFACE_PREFIX"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SURFACE_DIR/target}"
(
  cd "$worktree_surface"
  cargo test -p ocean-surface-proxy
  cargo clippy -p ocean-surface-proxy --all-targets -- -D warnings
  cargo check -p ocean-surface-ui --target wasm32-unknown-unknown
  cargo test -p ocean-surface-ui
  cargo clippy -p ocean-surface-ui --target wasm32-unknown-unknown -- -D warnings
  cargo fmt --all -- --check
  node scripts/surface-auto-deploy.test.mjs
  env -u NO_COLOR trunk build --release
  cargo build -p ocean-surface-proxy --release
)

promote_bundle "$worktree_surface/dist" "$target" "$CARGO_TARGET_DIR/release/ocean-surface-proxy"
