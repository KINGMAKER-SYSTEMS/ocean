#!/usr/bin/env bash
# Shared release custody for the installer, deployment rail and stable launcher.
# Callers set STATE_DIR and SURFACE_DIR before using the mutation helpers.
OWNER_HOME="${OCEAN_SURFACE_OWNER_HOME:-$HOME}"

release_fail() { echo "ERROR: $*" >&2; exit 1; }
proxy_hash() { shasum -a 256 "$1" | awk '{print $1}'; }

acquire_release_lock() {
  local lock="$STATE_DIR/auto-deploy.lock" owner
  RELEASE_LOCK_OWNED=0
  mkdir -p "$STATE_DIR"
  owner="$(cat "$lock/pid" 2>/dev/null || true)"
  # Only a direct installer child may borrow its parent's live lock.
  if [[ "${OCEAN_SURFACE_LOCK_OWNER:-}" == "$PPID" && "$owner" == "$PPID" ]]; then
    return 0
  fi
  if ! mkdir "$lock" 2>/dev/null; then
    owner="$(cat "$lock/pid" 2>/dev/null || true)"
    [[ "$owner" =~ ^[0-9]+$ ]] || return 75
    if [[ "$owner" =~ ^[0-9]+$ ]] && kill -0 "$owner" 2>/dev/null; then
      echo "BUSY: surface deployment already running as pid $owner" >&2
      return 75
    fi
    # Serialize stale-owner reclamation, then recheck under that claim. A
    # second contender must never remove a newly acquired live lock.
    mkdir "$lock/reclaim" 2>/dev/null || return 75
    owner="$(cat "$lock/pid" 2>/dev/null || true)"
    if [[ ! "$owner" =~ ^[0-9]+$ ]] || kill -0 "$owner" 2>/dev/null; then
      rmdir "$lock/reclaim"
      return 75
    fi
    rm -rf "$lock"
    mkdir "$lock" 2>/dev/null || return 75
  fi
  printf '%s\n' "$$" > "$lock/pid"
  RELEASE_LOCK_OWNED=1
  export OCEAN_SURFACE_LOCK_OWNER="$$"
}

release_lock() {
  if [[ "${RELEASE_LOCK_OWNED:-0}" == 1 && "$(cat "$STATE_DIR/auto-deploy.lock/pid" 2>/dev/null || true)" == "$$" ]]; then
    rm -rf "$STATE_DIR/auto-deploy.lock"
  fi
}

validate_release() {
  local release="$1" revision
  [[ -x "$release/bin/ocean-surface-proxy" && -f "$release/source-revision" && -f "$release/proxy-sha256" ]] || release_fail "release has no immutable proxy/provenance: $release"
  [[ "$(proxy_hash "$release/bin/ocean-surface-proxy")" == "$(cat "$release/proxy-sha256")" ]] || release_fail "release proxy checksum mismatch: $release"
  revision="$(cat "$release/source-revision")"
  if [[ "$revision" == unknown ]]; then
    [[ -f "$release/legacy-dist-path" && -f "$release/legacy-proxy-path" ]] || release_fail "unknown proxy provenance without legacy recovery metadata"
  else
    [[ -f "$release/dist/.deploy-sha" && "$(cat "$release/dist/.deploy-sha")" == "$revision" ]] || release_fail "proxy/bundle revision mismatch: $release"
  fi
}

# Resolve once: a running proxy keeps serving the bundle paired with its own
# executable even if current changes before its supervised restart.
resolve_proxy_release() {
  local selected="$1" physical candidate legacy
  physical="$(cd "$selected" && pwd -P)" || release_fail "selected bundle missing: $selected"
  candidate="$(dirname "$physical")"
  if [[ ! -f "$candidate/source-revision" ]]; then
    legacy="$STATE_DIR/legacy-recovery"
    [[ -d "$legacy" && -f "$legacy/legacy-dist-path" && "$(cat "$legacy/legacy-dist-path")" == "$physical" ]] || release_fail "legacy bundle has no captured proxy; run the installer before restarting"
    candidate="$(cd "$legacy" && pwd -P)"
  fi
  validate_release "$candidate"
  SURFACE_RELEASE_DIST="$candidate/dist"
  SURFACE_PROXY_BIN="$candidate/bin/ocean-surface-proxy"
  SURFACE_PROXY_REVISION="$(cat "$candidate/source-revision")"
  SURFACE_PROXY_HASH="$(cat "$candidate/proxy-sha256")"
}

legacy_proxy_path() {
  local plist="$OWNER_HOME/Library/LaunchAgents/dev.risingtides.ocean-surface-proxy.plist"
  if [[ -n "${OCEAN_SURFACE_LEGACY_PROXY:-}" ]]; then
    printf '%s\n' "$OCEAN_SURFACE_LEGACY_PROXY"
  elif [[ -f "$plist" ]]; then
    # Read only path metadata, never auth/environment values. Older templates
    # identify their standalone component through WorkingDirectory.
    python3 - "$plist" <<'PYTHON'
import os, plistlib, sys
with open(sys.argv[1], 'rb') as source:
    config = plistlib.load(source)
component = config.get('EnvironmentVariables', {}).get('OCEAN_SURFACE_REPO') or config.get('WorkingDirectory')
if not isinstance(component, str) or not os.path.isabs(component):
    sys.exit('legacy proxy path unavailable; set OCEAN_SURFACE_LEGACY_PROXY explicitly')
print(os.path.join(component, 'target/release/ocean-surface-proxy'))
PYTHON
  else
    printf '%s\n' "$SURFACE_DIR/target/release/ocean-surface-proxy"
  fi
}

# Bundle-only releases do not prove a proxy source revision. Capture the prior
# executable before any Cargo build can replace it; never infer that revision
# from the bundle marker. The existing current link stays untouched.
capture_legacy_proxy() {
  local current="$STATE_DIR/current" physical binary digest release staged next
  [[ -L "$current" ]] || return 0
  physical="$(cd "$current" && pwd -P)" || release_fail "legacy selection is missing"
  if [[ -f "$(dirname "$physical")/source-revision" ]]; then
    validate_release "$(dirname "$physical")"
    return 0
  fi
  if [[ -f "$STATE_DIR/legacy-recovery/legacy-dist-path" && "$(cat "$STATE_DIR/legacy-recovery/legacy-dist-path")" == "$physical" ]]; then
    validate_release "$(cd "$STATE_DIR/legacy-recovery" && pwd -P)"
    return 0
  fi
  binary="$(legacy_proxy_path)" || release_fail "cannot identify the legacy proxy; selection unchanged"
  [[ -x "$binary" ]] || release_fail "cannot recover the legacy proxy at $binary; selection unchanged"
  digest="$(printf '%s\n%s\n' "$physical" "$(proxy_hash "$binary")" | shasum -a 256 | awk '{print $1}')"
  release="$STATE_DIR/releases/legacy-$digest"
  mkdir -p "$STATE_DIR/releases"
  staged="$(umask 077; mktemp -d "$STATE_DIR/releases/.legacy.XXXXXX")"
  mkdir -p "$staged/bin" "$staged/dist"
  cp "$binary" "$staged/bin/ocean-surface-proxy"
  rsync -a "$physical/" "$staged/dist/"
  printf '%s\n' unknown > "$staged/source-revision"
  printf '%s\n' "$physical" > "$staged/legacy-dist-path"
  printf '%s\n' "$binary" > "$staged/legacy-proxy-path"
  proxy_hash "$staged/bin/ocean-surface-proxy" > "$staged/proxy-sha256"
  validate_release "$staged"
  if [[ -d "$release" ]]; then
    validate_release "$release"
    rm -rf "$staged"
  else
    mv "$staged" "$release"
  fi
  next="$STATE_DIR/.legacy-recovery.$$"
  ln -s "releases/legacy-$digest" "$next"
  python3 -c 'import os,sys; os.replace(sys.argv[1], sys.argv[2])' "$next" "$STATE_DIR/legacy-recovery"
}

# Old installed launchers may still name the mutable path. Restore its exact
# captured bytes atomically during legacy rollback, without assigning provenance.
restore_legacy_proxy() {
  local physical legacy="$STATE_DIR/legacy-recovery" binary next
  [[ -L "$STATE_DIR/current" ]] || return 0
  physical="$(cd "$STATE_DIR/current" && pwd -P)" || return 1
  [[ -f "$legacy/legacy-dist-path" && "$(cat "$legacy/legacy-dist-path")" == "$physical" ]] || return 0
  ( validate_release "$legacy" ) || return 1
  binary="$(cat "$legacy/legacy-proxy-path")"
  next="$(dirname "$binary")/.proxy-recovery.$$"
  cp "$legacy/bin/ocean-surface-proxy" "$next" || { rm -f "$next"; return 1; }
  mv -f "$next" "$binary" || return 1
}

retain_recovery() {
  RECOVERY_FAILED=1
  echo "RECOVERY: $2; retained owner-only backup at $1" >&2
}

finish_client_backup() {
  if [[ "${RECOVERY_FAILED:-0}" == 1 || -f "$1/recovery-pending" ]]; then
    echo "RECOVERY: unverified; preserve owner-only backup at $1" >&2
  else
    rm -rf "$1"
  fi
}

# Only explicit bootstrap observes supervision. Do not stop a loaded but
# operator-disabled job that could not subsequently be restored.
snapshot_supervision() {
  local backup="$1" label
  [[ "${OCEAN_SURFACE_BOOTSTRAP:-0}" == 1 && ! -f "$backup/supervision" ]] || return 0
  launchctl print-disabled "$DOMAIN" > "$backup/disabled" || release_fail "cannot read supervision overrides"
  python3 - "$backup/disabled" <<'PYTHON'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
if not re.search(r'disabled services\s*=\s*\{', text):
    sys.exit('cannot establish prior supervision overrides')
for label in ['dev.risingtides.ocean-surface-proxy', 'dev.risingtides.ocean-surface-auto-deploy']:
    target = '"' + re.escape(label) + '"'
    matches = re.findall(r'^\s*' + target + r'\s*=>\s*(true|false)\s*[,;]?\s*$', text, re.M)
    if re.search(target, text) and len(matches) != 1:
        sys.exit('cannot establish named supervision override')
    if matches == ['true']:
        sys.exit('named Surface job is operator-disabled; preserve that override')
PYTHON
  launchctl print "$DOMAIN" >/dev/null 2>&1 || release_fail "cannot read supervision domain"
  for label in dev.risingtides.ocean-surface-proxy dev.risingtides.ocean-surface-auto-deploy; do
    if launchctl print "$DOMAIN/$label" >/dev/null 2>&1; then
      [[ -f "$OWNER_HOME/Library/LaunchAgents/$label.plist" ]] || release_fail "loaded job has no recoverable plist: $label"
      if [[ "$label" == dev.risingtides.ocean-surface-proxy ]]; then
        [[ -L "$STATE_DIR/current" ]] || release_fail "loaded proxy has no recoverable selected release"
      fi
      touch "$backup/loaded-$label"
    fi
  done
  touch "$backup/supervision"
}

# Only generated client assets and installed supervision configuration are
# copied. The owner auth file is never copied, replaced or bundled.
snapshot_clients() {
  CLIENT_BACKUP="$(umask 077; mktemp -d "$STATE_DIR/.client-backup.XXXXXX")"
  local name path
  for name in dist extension launchers proxy-plist deploy-plist; do
    case "$name" in
      dist) path="$SURFACE_DIR/dist" ;;
      extension) path="$SURFACE_DIR/extension/dist" ;;
      launchers) path="$OWNER_HOME/.config/ocean-surface/bin" ;;
      proxy-plist) path="$OWNER_HOME/Library/LaunchAgents/dev.risingtides.ocean-surface-proxy.plist" ;;
      deploy-plist) path="$OWNER_HOME/Library/LaunchAgents/dev.risingtides.ocean-surface-auto-deploy.plist" ;;
    esac
    if [[ -e "$path" || -L "$path" ]]; then
      cp -a "$path" "$CLIENT_BACKUP/$name"
      touch "$CLIENT_BACKUP/had-$name"
    fi
  done
  # Allocate the retention marker before any mutation. Retention after failure
  # must not depend on being able to allocate another file on an exhausted disk.
  touch "$CLIENT_BACKUP/recovery-pending"
  touch "$CLIENT_BACKUP/complete"
}

restore_clients() {
  local backup="$1" name path status=0
  # A failed snapshot has not yet allowed any client/config mutation. Never
  # treat its incomplete inventory as evidence that an existing path was absent.
  [[ -f "$backup/complete" ]] || return 0
  for name in dist extension launchers proxy-plist deploy-plist; do
    case "$name" in
      dist) path="$SURFACE_DIR/dist" ;;
      extension) path="$SURFACE_DIR/extension/dist" ;;
      launchers) path="$OWNER_HOME/.config/ocean-surface/bin" ;;
      proxy-plist) path="$OWNER_HOME/Library/LaunchAgents/dev.risingtides.ocean-surface-proxy.plist" ;;
      deploy-plist) path="$OWNER_HOME/Library/LaunchAgents/dev.risingtides.ocean-surface-auto-deploy.plist" ;;
    esac
    if [[ -f "$backup/had-$name" ]]; then
      # Idempotent when the promotion child already restored this snapshot.
      if [[ -e "$backup/$name" || -L "$backup/$name" ]]; then
        rm -rf "$path" || { status=1; continue; }
        mkdir -p "$(dirname "$path")" || { status=1; continue; }
        mv "$backup/$name" "$path" || status=1
      fi
    else
      rm -rf "$path" || status=1
    fi
  done
  return "$status"
}
