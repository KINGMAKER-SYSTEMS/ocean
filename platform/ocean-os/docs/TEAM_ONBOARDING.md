# Ocean onboarding preparation (macOS arm64)

This candidate prepares the published package and a host identity file. It does
not install a supervised daemon, activate federation, redeem invitations, or
provide an Ocean MCP server. Package installation, merged source, and verified
runtime behavior are separate outcomes.

## Prerequisites

- macOS on Apple silicon, Python 3.11 or newer, and bun or npm.
- GitHub CLI authenticated for the intended package with `read:packages` access.
- A published `@risingtides-dev/ocean` version with verified package access.
  The current package contains `ocean`, `ocean-daemon`, and `ocean-update`.
  See [the package contract](../packaging/npm/README.md); repository visibility
  alone does not establish package access or a published release.
- An agreed member identifier and a supported model alias.
- The identity route from PR41 must be reviewed, merged, and installed before
  the running daemon can project `member.toml` through `GET /v1/identity`.
  This script does not install that feature or establish cross-client identity.

## Prepare the machine

Log in explicitly; the script never starts an interactive login:

```bash
gh auth login
gh auth refresh -s read:packages
git clone https://github.com/KINGMAKER-SYSTEMS/ocean.git
cd ocean/platform/ocean-os
ops/onboard-teammate.sh --model MODEL --member MEMBER --dry-run
ops/onboard-teammate.sh --model MODEL --member MEMBER
```

Optional `--display-name NAME` supports quotes, backslashes and Unicode, up to
80 characters without controls. Member identifiers use ASCII letters, digits,
`.`, `_`, `@`, and `-`. The strict TOML file contains only `member_id` and an
optional `display_name`, matching the daemon-local identity projection.

The [script](../ops/onboard-teammate.sh) validates destinations and existing
identity before requesting package credentials. It refreshes only the relevant
scope/token entries in `~/.npmrc`, preserving unrelated entries, and installs
the package with bun or npm. The token travels through stdin and is never
printed or placed in process arguments. `.npmrc` and `member.toml` are atomically
written with mode `0600`; the dedicated identity directory is mode `0700`.
Symlink paths, nonregular or multiply linked files, foreign-owned destinations,
and malformed existing identities are refused. `--force` permits intentional
identity replacement; it does not bypass file custody checks.

Identity directory precedence is `OCEAN_CONFIG_DIR`, then
`XDG_CONFIG_HOME/ocean-rs`, then `~/.config/ocean-rs`. Nondefault paths must be
absolute and must match the configuration used by the eventual daemon.
`--dry-run` performs no package/auth/network/service calls or file changes.

## Start and verify separately

On a machine without a supervised daemon, the package TUI can launch its sibling
daemon. Start outside a Git checkout and select the model deliberately:

```bash
cd ~
OCEAN_MODEL=MODEL ocean
```

Use `/login` and `/model` in the TUI. Codex and ChatGPT login remain distinct
provider flows where the installed release supports them.

Existing supervised installations retain the [canonical installer](../ops/install-ocean-daemon.sh)
and [operations contract](OPERATIONS.md). That installer requires clean, freshly
fetched canonical `main`, preserves immutable artifacts, and verifies recovery
and revision. Observe its intake and Rooms migration gates. Updating the package
does not update an already-running supervised daemon.

After the identity feature is installed, verify separately:

```bash
curl -fsS http://127.0.0.1:4780/health
curl -fsS http://127.0.0.1:4780/v1/identity
```

Check the actual running revision and expected `member_id`. The identity answer
is host configuration, not authentication or authorization of a caller. It does
not prove that another client posts as that identity.

Federation requires a separately reviewed operator deployment. Current public
runtime configuration uses environment variables; this candidate supplies no
`federation.env` or Keychain loader and performs no federation activation.
Keep service origins, invitations, credentials, private host inventory, and
organization-specific access procedures outside this public runbook.

## Verification

```bash
bash -n ops/onboard-teammate.sh
PYTHONDONTWRITEBYTECODE=1 python3 ops/test_onboard_teammate.py
cargo xtask docs-check
```

Fixtures copy the script, redirect only its home-path token into a temporary
directory, preserve process `HOME`, and replace package/auth/platform commands
with mocks. They do not install software, operate services, read real
credentials, or contact a network.
