#!/usr/bin/env bash
# Build reviewed main content, then transactionally install the supervised daemon.
# This intentionally restarts the named LaunchAgent. Coordinate an intake quiet
# window: the activity check is an observation, not an atomic intake drain.
set -euo pipefail
umask 077

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LABEL="dev.risingtides.ocean-daemon"
DOMAIN="gui/$(id -u)"
LIBEXEC="$HOME/.local/libexec/ocean-daemon"
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
COMMAND_LINK="$HOME/.local/bin/ocean-daemon"
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"
command -v python3 >/dev/null || { echo "FATAL: python3 is required for metadata validation." >&2; exit 69; }

# Stale refs, local main ahead of origin, and dirty source are not release inputs.
# Detached clean origin/main checkouts are supported.
git -C "$REPO" fetch origin main --quiet
SOURCE_SHA="$(git -C "$REPO" rev-parse HEAD)"
EXPECTED_REV="$(git -C "$REPO" rev-parse --short=12 HEAD)"
SOURCE_STATUS="$(git -C "$REPO" status --porcelain)"
if [[ "$SOURCE_SHA" != "$(git -C "$REPO" rev-parse origin/main)" ]] ||
   [[ -n "$SOURCE_STATUS" ]]; then
  echo "FATAL: deploy from a clean checkout at freshly fetched origin/main." >&2
  exit 64
fi

mkdir -p "$LIBEXEC" "$(dirname "$PLIST_DST")" "$(dirname "$COMMAND_LINK")"
LOCK="$LIBEXEC/.install-lock"
if ! mkdir "$LOCK" 2>/dev/null; then
  echo "FATAL: another install owns $LOCK; do not remove a live install lock." >&2
  exit 75
fi
STAGE=""
PROMOTED=0
SERVICE_TOUCHED=0
COMMITTED=0
PREVIOUS_LOADED=0
PREVIOUS_REV=""
PREVIOUS_TARGET=""

wait_unloaded() {
  local attempt
  for attempt in $(seq 1 50); do
    launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1 || return 0
    sleep 0.2
  done
  return 1
}

# Never print response bodies: readiness can contain private diagnostic context.
ready_matches() {
  local revision="$1" attempt
  for attempt in $(seq 1 30); do
    if curl -fsS -m 2 http://127.0.0.1:4780/health > "$STAGE/health.json" 2>/dev/null &&
       curl -fsS -m 2 http://127.0.0.1:4780/ready > "$STAGE/ready.json" 2>/dev/null &&
       python3 - "$revision" "$STAGE/health.json" "$STAGE/ready.json" <<'PY'
import json, sys
try:
    values = [json.load(open(path)) for path in sys.argv[2:]]
    valid = all(value.get("ok") is True and value.get("rev") == sys.argv[1]
                for value in values)
except (OSError, ValueError, AttributeError):
    valid = False
sys.exit(0 if valid else 1)
PY
    then
      return 0
    fi
    sleep 1
  done
  return 1
}

copy_path() {
  local source="$1" target="$2"
  if [[ -L "$source" ]]; then
    ln -s "$(readlink "$source")" "$target"
  else
    cp -p "$source" "$target"
  fi
}

snapshot_path() {
  local name="$1" path="$2"
  if [[ -e "$path" || -L "$path" ]]; then
    copy_path "$path" "$STAGE/$name"
  fi
}

publish_path() {
  local source="$1" path="$2" temporary="$2.install.$$"
  copy_path "$source" "$temporary" && mv -f "$temporary" "$path"
}

restore_path() {
  local name="$1" path="$2"
  if [[ -e "$STAGE/$name" || -L "$STAGE/$name" ]]; then
    publish_path "$STAGE/$name" "$path"
  else
    rm -f "$path"
  fi
}

rollback() {
  local failed=0
  echo "==> installation failed; restoring the previous release selection and configuration" >&2
  if [[ "$SERVICE_TOUCHED" == 1 ]]; then
    launchctl bootout "$DOMAIN/$LABEL" >/dev/null 2>&1 || true
    wait_unloaded || failed=1
  fi
  restore_path previous-current "$LIBEXEC/current" || failed=1
  restore_path previous-launcher "$LIBEXEC/launch.sh" || failed=1
  restore_path previous-plist "$PLIST_DST" || failed=1
  restore_path previous-command "$COMMAND_LINK" || failed=1
  if [[ "$SERVICE_TOUCHED" == 1 && "$PREVIOUS_LOADED" == 1 ]]; then
    if ! launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
      launchctl bootstrap "$DOMAIN" "$PLIST_DST" || failed=1
    fi
    launchctl kickstart -k "$DOMAIN/$LABEL" || failed=1
    ready_matches "$PREVIOUS_REV" || failed=1
  fi
  if [[ "$failed" == 0 ]]; then
    echo "==> previous installation restored; the install still failed" >&2
  else
    echo "FATAL: rollback could not prove recovery; backup retained at $STAGE" >&2
  fi
  return "$failed"
}

finish() {
  local status="$?" retain=0 path
  trap - EXIT INT TERM
  set +e
  if [[ "$status" != 0 && "$PROMOTED" == 1 && "$COMMITTED" == 0 ]]; then
    rollback || retain=1
  fi
  for path in "$LIBEXEC/current" "$LIBEXEC/launch.sh" "$PLIST_DST" "$COMMAND_LINK"; do
    rm -f "$path.install.$$"
  done
  if [[ "$retain" == 0 && -n "$STAGE" ]]; then
    rm -rf "$STAGE"
  fi
  rmdir "$LOCK"
  exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
STAGE="$(mktemp -d "$LIBEXEC/.install.XXXXXX")"

echo "==> building ocean-daemon $EXPECTED_REV (release, legacy-chromium, locked)"
# Keep the existing interim production browser configuration. Explicit target-dir
# prevents inherited CARGO_TARGET_DIR from selecting a stale binary.
( cd "$REPO" && cargo build --locked -p ocean-daemon --release --features legacy-chromium --target-dir "$REPO/target" )
BUILT_SHA="$(git -C "$REPO" rev-parse HEAD)"
BUILT_STATUS="$(git -C "$REPO" status --porcelain)"
if [[ "$SOURCE_SHA" != "$BUILT_SHA" ]] || [[ -n "$BUILT_STATUS" ]]; then
  echo "FATAL: source changed during the build; refusing publication." >&2
  exit 64
fi
BIN="$REPO/target/release/ocean-daemon"
[[ -x "$BIN" ]] || { echo "FATAL: build did not produce $BIN" >&2; exit 70; }
DEST_BIN="$LIBEXEC/ocean-daemon-$SOURCE_SHA"
install -m 0755 "$BIN" "$STAGE/binary"
if [[ -e "$DEST_BIN" || -L "$DEST_BIN" ]]; then
  if [[ -L "$DEST_BIN" || ! -f "$DEST_BIN" || ! -x "$DEST_BIN" ]] || ! cmp -s "$STAGE/binary" "$DEST_BIN"; then
    echo "FATAL: immutable revision artifact already exists with different content: $DEST_BIN" >&2
    exit 65
  fi
else
  # Hard-link creation cannot overwrite an existing artifact, even on a race.
  ln "$STAGE/binary" "$DEST_BIN"
fi
install -m 0755 "$REPO/deploy/ocean-daemon.sh" "$STAGE/launcher"
# Escape replacement metacharacters in an operator home path.
RENDER_HOME="${HOME//\\/\\\\}"
RENDER_HOME="${RENDER_HOME//&/\\&}"
RENDER_HOME="${RENDER_HOME//|/\\|}"
sed "s|__OCEAN_HOME__|$RENDER_HOME|g" "$REPO/deploy/$LABEL.plist" > "$STAGE/plist"
plutil -lint "$STAGE/plist"
if grep -q "__OCEAN_HOME__" "$STAGE/plist"; then
  echo "FATAL: plist contains unexpanded placeholders." >&2
  exit 70
fi

# Respect persistent operator disablement before changing selection or stopping
# a loaded job. Neither successful install nor recovery writes that override.
launchctl print-disabled "$DOMAIN" > "$STAGE/disabled"
python3 - "$STAGE/disabled" "$LABEL" <<'PY'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
target = '"' + re.escape(sys.argv[2]) + '"'
# Older launchd prints true/false; macOS 26 prints disabled/enabled.
match = re.search(r'^\s*' + target + r'\s*=>\s*(true|false|disabled|enabled)\s*[,;]?\s*$', text, re.M)
if not re.search(r"disabled services\s*=\s*\{", text) or (re.search(target, text) and not match):
    print("FATAL: cannot establish the prior supervision override.", file=sys.stderr)
    sys.exit(70)
if match and match[1] in ("true", "disabled"):
    print("FATAL: the named daemon is operator-disabled; preserve that override.", file=sys.stderr)
    sys.exit(75)
PY
snapshot_path previous-current "$LIBEXEC/current"
snapshot_path previous-launcher "$LIBEXEC/launch.sh"
snapshot_path previous-plist "$PLIST_DST"
snapshot_path previous-command "$COMMAND_LINK"
if [[ -L "$LIBEXEC/current" ]]; then
  PREVIOUS_TARGET="$(readlink "$LIBEXEC/current")"
  [[ "$PREVIOUS_TARGET" == /* ]] || PREVIOUS_TARGET="$LIBEXEC/$PREVIOUS_TARGET"
fi
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  PREVIOUS_LOADED=1
  if [[ ! -L "$LIBEXEC/current" || ! -x "$LIBEXEC/current" ||
        ! -f "$LIBEXEC/launch.sh" || ! -f "$PLIST_DST" ]]; then
    echo "FATAL: loaded service has no complete recoverable installation." >&2
    exit 70
  fi
  # Observe execution plus queued/permission-waiting intake just before promotion.
  # Operators must keep intake quiet across this snapshot.
  curl -fsS -m 2 http://127.0.0.1:4780/metrics > "$STAGE/metrics"
  curl -fsS -m 2 http://127.0.0.1:4780/v1/requests > "$STAGE/requests.json"
  curl -fsS -m 2 http://127.0.0.1:4780/health > "$STAGE/previous-health.json"
  PREVIOUS_REV="$(python3 - "$STAGE" <<'PY'
import json, pathlib, sys
try:
    directory = pathlib.Path(sys.argv[1])
    metrics = [line.split() for line in (directory / "metrics").read_text().splitlines()]
    gauges = [float(parts[1]) for parts in metrics
              if len(parts) == 2 and parts[0] == "ocean_turns_in_flight"]
    requests = json.loads((directory / "requests.json").read_text())
    health = json.loads((directory / "previous-health.json").read_text())
    terminal = {"cancelled", "completed", "errored"}
    valid = (gauges == [0.0] and requests.get("ok") is True
             and isinstance(requests.get("requests"), list)
             and all(item.get("state") in terminal for item in requests["requests"])
             and health.get("ok") is True and isinstance(health.get("rev"), str)
             and bool(health["rev"]))
    if not valid:
        raise ValueError()
    print(health["rev"])
except (OSError, ValueError, KeyError, AttributeError, TypeError):
    print("FATAL: activity/provenance preflight failed; preserve the current daemon.", file=sys.stderr)
    sys.exit(75)
PY
)"
elif curl -fsS -m 2 http://127.0.0.1:4780/health >/dev/null 2>&1; then
  echo "FATAL: a daemon is serving outside the named LaunchAgent; reconcile ownership first." >&2
  exit 75
fi

echo "==> promoting $DEST_BIN; intake must remain quiet until restart completes"
ln -s "$DEST_BIN" "$STAGE/current"
ln -s "$LIBEXEC/current" "$STAGE/command"
PROMOTED=1
publish_path "$STAGE/current" "$LIBEXEC/current"
publish_path "$STAGE/launcher" "$LIBEXEC/launch.sh"
publish_path "$STAGE/plist" "$PLIST_DST"
publish_path "$STAGE/command" "$COMMAND_LINK"
SERVICE_TOUCHED=1
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
if ! wait_unloaded; then
  echo "FATAL: service did not unload within 10 seconds." >&2
  exit 75
fi
if ! launchctl bootstrap "$DOMAIN" "$PLIST_DST"; then
  sleep 1
  launchctl bootstrap "$DOMAIN" "$PLIST_DST" || { echo "FATAL: bootstrap failed twice." >&2; exit 70; }
fi
launchctl kickstart -k "$DOMAIN/$LABEL"
if ! ready_matches "$EXPECTED_REV"; then
  echo "FATAL: expected revision $EXPECTED_REV was not healthy and ready after bounded polling." >&2
  exit 70
fi
COMMITTED=1
echo "==> installed and ready: $DEST_BIN"
# Retire artifacts only after proven promotion. Preserve both selected and
# previous working artifacts, even if the latter is outside the newest three.
ls -t "$LIBEXEC"/ocean-daemon-* 2>/dev/null | tail -n +4 | while IFS= read -r old; do
  if [[ "$old" != "$DEST_BIN" && ! "$old" -ef "$PREVIOUS_TARGET" ]]; then
    rm -f "$old" || echo "WARN: could not retire $old; installed service remains ready." >&2
  fi
done
