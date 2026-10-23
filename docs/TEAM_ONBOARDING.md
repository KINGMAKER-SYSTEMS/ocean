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
credentials; verify a real reply separately. The daemon creates and persists
its local owner through `GET /v1/me`; a new desktop installation does not
require `member.toml` to create, join or post in local Rooms. An existing owner
keeps its identity across restarts and later configuration changes.

Shared-room membership comes from the accepted invitation and its scoped
credential, not the local owner's name or a manually chosen member id.
**Team**, in the same app menu, saves the initial team server and offers an
explicit restart. Configured means the daemon loaded that server; invitation
acceptance and a shared conversation prove membership and connectivity.
The Team control is newer than installer source `8ca79ae2`; use a rebuilt,
validated candidate before handing off these steps.

The desktop candidate also includes `ocean-mcp` (`ocean-mcp.exe` on Windows)
beside its daemon. An external MCP host can run that installed executable with
the argument `serve`; it connects to the same local daemon. On macOS it lives
inside `Ocean.app/Contents/MacOS/`; on Windows it lives in the app's installation
directory. Host configuration is still an explicit setup step. The bridge's
`doctor` command checks the daemon connection and local owner identity.
Room reads, joins and posts remain available through MCP. Direct prompts are
unavailable through this bridge; use Ocean's CLI or app to run a direct turn
with its permission controls.

## First coworker

1. Use the approved installer for that computer. Until a stable desktop release
   is available, unsigned candidates are installation tests, not the released
   setup path. Record the artifact's source revision.
2. Open Ocean, choose the starting model, connect the corresponding account
   through **Accounts**, and complete one real no-tools reply. A connected
   indicator is insufficient.
3. Open **Team**, enter the existing team's server address and save. Choose
   **Restart Ocean** when offered. If the service is managed outside the app,
   restart it through its existing supervisor, then choose **Check again**.
   Keep the real origin and access procedures in the private operator runbook.
   Existing installations retain `OCEAN_FEDERATION_URL`; this setup cannot
   replace an existing destination or redirect stored membership credentials.
   A default installation can use local Rooms before team setup.
4. Redeem that coworker's invitation: in Rooms, choose **Add room → Join room**,
   paste the invitation link, then confirm. The invitation supplies the
   recipient's membership and shared display name. This also works in the
   browser without an installed `ocean://` handler, but still requires that
   coworker's own connected daemon. Do not share another operator's proxy
   password or credential files as a substitute for coworker identity.
5. Use a real task in that room. Confirm both humans see the same message,
   the intended agent replies from its own host, and the reply opens the
   correct session. Reload both clients and confirm conversation continuity.

Keep the first room focused on work the team already does. Installation is
complete when a coworker can return to that conversation and get work done.
Adding more rooms and settings is not an onboarding milestone.

For the existing Apple Silicon command-line distribution, use the
[package guide](../platform/ocean-os/packaging/npm/README.md) and
[host setup guide](../platform/ocean-os/docs/TEAM_ONBOARDING.md). That optional
preparation script installs the package and seeds a fresh host's local owner
from `member.toml`; it does not grant shared-room membership. Its `--dry-run`
does not prove installation or connectivity. A package update also does not
update an already-running supervised daemon.

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
