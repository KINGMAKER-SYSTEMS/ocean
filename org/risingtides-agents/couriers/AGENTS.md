# Courier package contract

## Purpose

Own the manifest-driven courier hub, shared transport, and single-purpose delivery packages.

## Ownership

- `hub/router.py` discovers manifests and is authoritative for live package and command inventory.
- `transport/` owns shared delivery routes.
- `README.md` and `ARCHITECTURE.md` describe the courier class and hub architecture.
- Each courier directory owns its manifest, protocol, harness, state contract, and local devlog.

## Local Contracts

- A registered courier has a `courier.toml`; do not document absent packages as live.
- Do not bypass a courier harness or duplicate shared transport.
- Confirm shared-channel destinations before delivery and never commit tokens or destination secrets.
- Read the courier's child `AGENTS.md` before editing its files.

## Work Guidance

- Add packages through manifests, not router conditionals.
- Keep prose inventory aligned with `python3 hub/router.py list`.

## Verification

- `python3 hub/router.py list`
- Run the narrow checks defined by the touched courier's child contract.

## Child devlog Index

- `core-music-editor/` — Core Core acoustic edit waves → `core-music-editor/AGENTS.md`
- `file-courier/` — generic Slack file and message delivery → `file-courier/AGENTS.md`
- `folk-edit/` — Stillness edit and review waves → `folk-edit/AGENTS.md`
- `hopecore/` — earned-hope montage waves → `hopecore/AGENTS.md`
- `mon-distro/` — Mon Rovia distribution operations → `mon-distro/AGENTS.md`
- `mon-email-watcher/` — Mon Rovia email sweep operations → `mon-email-watcher/AGENTS.md`
- `pr-babysitter/` — unattended pull-request maintenance → `pr-babysitter/AGENTS.md`
- `sound-watcher/` — audio-tracking sweeps and canvases → `sound-watcher/AGENTS.md`
- `uiau/` — Us In Another Universe montage engine → `uiau/AGENTS.md`
- `yt-clipper/` — YouTube clip-factory autopilot → `yt-clipper/AGENTS.md`
