#!/usr/bin/env bash
# Prepare the published package and this host's daemon identity. This does not
# install/restart a service, configure federation, or authenticate a provider.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKG="@kingmaker-systems/ocean"
CONFIG_DIR="${OCEAN_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/ocean-rs}"
NPMRC="$HOME/.npmrc"
MODEL=""; MEMBER_ID=""; DISPLAY_NAME=""; FORCE=0; DRY=0

usage() {
  cat <<'USAGE'
Usage: ops/onboard-teammate.sh --model MODEL --member ID
                              [--display-name NAME] [--force] [--dry-run]

Requires macOS arm64, Python 3.11+, gh authenticated with read:packages,
and bun or npm. Installs @kingmaker-systems/ocean and writes member.toml.
--force replaces an existing different or malformed identity; it never bypasses
file custody checks. --dry-run validates and reports without authentication,
network access, credential reads, package installation, or file changes.

Identity projection requires the separately reviewed PR41 implementation in the
running daemon. It is host configuration, not proof of caller authentication.
No federation, MCP server, daemon promotion, or service restart is performed.
USAGE
}
fail() { printf 'FATAL: %s\n' "$1" >&2; exit "${2:-1}"; }
value() { [[ $# -ge 2 && -n "$2" ]] || fail "missing value for $1" 64; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --model) value "$@"; MODEL="$2"; shift 2 ;;
    --member) value "$@"; MEMBER_ID="$2"; shift 2 ;;
    --display-name) value "$@"; DISPLAY_NAME="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" 64 ;;
  esac
done
[[ "$MODEL" =~ ^[A-Za-z0-9][A-Za-z0-9._:/-]*$ ]] || fail "--model is required and must be a model alias" 64
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || fail "this script requires macOS arm64" 64
command -v python3 >/dev/null 2>&1 || fail "install Python 3.11 or newer first" 69
python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' || fail "Python 3.11 or newer is required" 69
FILES="$REPO/ops/onboarding_files.py"
identity_args=(--config-dir "$CONFIG_DIR" --member "$MEMBER_ID" --display-name "$DISPLAY_NAME")
[[ $FORCE -eq 0 ]] || identity_args+=(--force)
# Validate identity and both destinations before credentials or package work.
python3 "$FILES" validate --npmrc "$NPMRC" "${identity_args[@]}"
if [[ $DRY -eq 1 ]]; then
  printf 'Would configure owner-only npm access, install %s, and prepare member.toml. No changes made.\n' "$PKG"
  exit 0
fi
command -v gh >/dev/null 2>&1 || fail "install gh and run gh auth login, then gh auth refresh -s read:packages" 69
if command -v npm >/dev/null 2>&1; then
  npm_root="$(npm root --global)" || fail "could not inspect npm global packages" 69
  if [[ -d "$npm_root/@risingtides-dev/ocean" ]]; then
    fail "personal Ocean package is installed. Download the org package with 'npm pack @kingmaker-systems/ocean@latest', then remove the personal package with 'npm uninstall -g @risingtides-dev/ocean' and install the downloaded tarball with 'npm install -g ./<tarball>'. Rerun onboarding afterward. Nothing was changed." 78
  fi
fi
python3 "$FILES" check-commands || fail "an existing Ocean command resolves to a personal or unreadable installation. Fix its PATH/package links before onboarding; no credentials or config were changed." 78
if command -v bun >/dev/null 2>&1; then
  installer=(bun add -g)
elif command -v npm >/dev/null 2>&1; then
  installer=(npm install -g)
else
  fail "install bun or npm first" 69
fi
gh auth status >/dev/null 2>&1 || fail "run gh auth login first" 78
gh auth status 2>&1 | grep -q 'read:packages' || fail "run gh auth refresh -s read:packages first" 78
# The token travels only over stdin, never a process argument or printed output.
if ! package_token="$(gh auth token 2>/dev/null)"; then
  fail "could not obtain package credentials; npm configuration was not changed" 78
fi
printf '%s\n' "$package_token" | python3 "$FILES" npmrc --npmrc "$NPMRC"
unset package_token
"${installer[@]}" "$PKG@latest"
python3 "$FILES" verify-commands || fail "Ocean commands must resolve to the same @kingmaker-systems/ocean installation. Fix the package manager global bin PATH and rerun; member identity was not changed." 70
python3 "$FILES" identity "${identity_args[@]}"
cat <<CHECKLIST
Package and host identity prepared. No daemon was installed or restarted.

For an unsupervised package installation, start from a directory outside Git:
  cd ~
  OCEAN_MODEL=$MODEL ocean
Then use /login and /model in the TUI. Codex and ChatGPT login choices remain
separate provider flows where supported by the installed release.

Existing supervised daemons remain managed by ops/install-ocean-daemon.sh from
clean canonical main, including its intake, migration and recovery gates.
Package installation does not update that supervised binary.

After the identity implementation (PR41) is present in the running daemon:
  curl -fsS http://127.0.0.1:4780/v1/identity
Confirm member_id is your configured value; this is not caller authentication.
Federation and an Ocean MCP executable are separate integration prerequisites.
CHECKLIST
