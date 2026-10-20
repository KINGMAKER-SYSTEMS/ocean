# Team setup

Use `KINGMAKER-SYSTEMS/ocean` for source, updates and releases. A coworker
does not need to finish building Ocean to use Rooms. They do need a working,
released client and their own connected daemon.

## Platform support

| Computer | Released setup path | Desktop candidate |
| --- | --- | --- |
| Apple Silicon Mac | macOS arm64 command-line package plus `platform/ocean-os/ops/onboard-teammate.sh` | Native app and matching daemon in a DMG. |
| Intel Mac | No released desktop installer | Native app and matching daemon in a DMG. |
| Windows x64 | No released desktop installer or onboarding script | Native app and matching daemon in an NSIS installer. |

The command-line package and setup script accept only macOS arm64. The
[desktop candidate builder](../apps/ocean-surface/packaging/AGENTS.md) builds
each installer on its native platform, checks its contents and starts its
bundled daemon in disposable state. These unsigned artifacts are candidates,
not a stable release or proof of installation on a coworker's computer.
Signing, clean-machine installation and update/recovery remain release gates.
Do not give a Windows user the Mac install command or present source
compilation as a completed team installation.

The desktop candidate collects a starting model before launching its local
daemon. **Accounts**, in the existing app menu, connects Claude, Codex or
ChatGPT through the system browser. A Connected account reports available
credentials; verify a real reply separately. Desktop startup does not yet
configure the coworker's member identity or federation connection. The host
setup and shared-room acceptance below are still required.

## First coworker

1. Verify a release from canonical main and access to its package. The
   [package guide](../platform/ocean-os/packaging/npm/README.md) describes the
   existing distribution, including its separate supervised-daemon update.
2. Follow the [host setup guide](../platform/ocean-os/docs/TEAM_ONBOARDING.md). On a supported Mac, use the
   [setup script](../platform/ocean-os/ops/onboard-teammate.sh) with that
   coworker's member id, display name and chosen model. Its `--dry-run`
   validates preparation; it does not prove an installed or connected daemon.
3. Start Ocean outside a Git repository, authenticate the provider, and
   complete one real no-tools reply. A connected indicator is insufficient.
4. Connect their own membership to one shared room using the existing
   federation flow. In Rooms, choose **Add room → Join room**, paste the
   invitation link, then confirm. This works in the browser without an
   installed `ocean://` handler, but still requires that coworker's own
   connected daemon. Do not share another operator's proxy password or
   credential files as a substitute for coworker identity.
5. Use a real task in that room. Confirm both humans see the same message,
   the intended agent replies from its own host, and the reply opens the
   correct session. Reload both clients and confirm conversation continuity.

Keep the first room focused on work the team already does. Installation is
complete when a coworker can return to that conversation and get work done.
Adding more rooms and settings is not an onboarding milestone.

## Release acceptance

Record the source revision, installed daemon revision, served client revision,
platform and completed reply/session evidence for each machine. Require two
humans, two machines and their separately owned agents before claiming team
acceptance. One successful local agent reply proves only that host's path.

Provider refresh must reach the next turn without requiring a daemon restart.
Room access changes must update the client, and a refused or failed turn must
remain visible. Routine audit records belong outside the conversation while
their durable records remain intact.

The current app's automatic update and rollback path must also work. Preserve
the running release if its recovery executable is missing; restore verified
recovery material before promotion rather than bypassing the installer guard.
