# Ocean Bedrock Local Sync App

Status: V0 companion app.

Purpose: make coworker onboarding simple. A coworker opens one local app, pastes a scoped invite/sync token once, chooses folders, and picks a sync schedule.

## Run

From the `ocean-bedrock` repo:

```bash
npm install
npm run ocean:app
```

The app binds to localhost and opens:

```txt
http://127.0.0.1:8765
```

Optional:

```bash
npm run ocean:app -- --port 8766 --no-open
```

## What the app does

- Stores local config at `~/.config/ocean-bedrock/bootstrap.json` with file mode `0600`.
- Uses existing scoped bearer tokens; no admin token belongs on coworker devices.
- Lets the user test the token against `/health` and `/api/v1/info`.
- Lets the user choose local folders using a native folder picker when available.
- Writes/refreshes a redacted source manifest in `/context/ocean-bedrock/sources`.
- Runs `scripts/ocean-ingest-local.mjs` in the background for manual sync.
- Uses the shared ingest secret-exclusion filters, so `.env`, SSH keys, credential files, password vaults, and similar paths are excluded by default.
- Uses server-side source/sync endpoints for lineage; coworker devices only need `serverUrl` + scoped bearer token.
- Can run scheduled sync while the app is open.
- Shows selected folders, latest run output, and recent app activity.

## Current integrations

Live in V0:

```txt
Local folders
```

Shown as upcoming in the UI:

```txt
GitHub
Telegram
Notion
Slack
Linear
Google Drive
R2
```

## Folder picker support

The app tries these native folder pickers:

- macOS: `osascript`
- Windows: PowerShell FolderBrowserDialog
- Linux: `zenity`, then `kdialog`

If none are available, the coworker can paste the folder path manually.

## Schedule behavior

Schedules are local to the running app process.

That means:

- sync runs only while the app is open,
- no OS service/daemon is installed yet,
- closing the terminal/app stops scheduled sync,
- reopening the app starts the next scheduled interval.

This is intentional for V0 because it is easy to reason about and safer for early coworker rollout.

## Token model

Create a contributor token on the operator machine and give the coworker only that token.

Example scopes for a coworker named `alice`:

```txt
/coworkers/alice
/sessions/alice
/context/ocean-bedrock
```

The coworker should set their app name to `alice`, so uploaded files land under:

```txt
/coworkers/alice/<device>/<folder-label>
```

Do not give coworker devices admin tokens. Contributor tokens should be scoped to the coworker's own `/coworkers/<name>`, `/sessions/<name>`, and shared `/context/ocean-bedrock` paths.

## V0 limitations

- Not packaged as a double-click desktop app yet.
- Scheduling only works while the app is open.
- Non-local integrations are UI placeholders for now.
- The server still owns the Postgres `DATABASE_URL`; if the server lacks it, lineage endpoints return unavailable while byte sync can still use the file API.
- There is not yet a full GUI preview/review panel for skipped sensitive files, but the underlying ingest dry-run now reports `skippedByReason` and `skippedFiles` with matched patterns.

## Next improvements

1. Package as a double-click desktop app using Electron/Tauri or a native wrapper.
2. Add one-time invite links so coworkers never see raw long tokens.
3. Add OS login/background scheduling for trusted devices.
4. Add GitHub and Telegram integration cards backed by real adapter runners.
5. Add record delete/tombstone reporting for removed local files.
