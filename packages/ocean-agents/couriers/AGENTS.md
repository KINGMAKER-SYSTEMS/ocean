# Courier package contract

## Purpose

Own the manifest-driven public router, shared delivery transport, and reusable
reference packages.

## Ownership

- `hub/router.py` discovers manifests and is authoritative for inventory.
- `transport/` owns shared delivery routes and API normalization.
- `file-courier/` is the deterministic public reference package.
- `README.md` and `ARCHITECTURE.md` describe the class and boundaries.

## Contracts

- A registered package has `courier.toml`.
- Add packages through manifests, not router conditionals.
- Confirm shared destinations before sending.
- Never commit credentials or destination IDs.
- Do not bypass a package harness or duplicate shared transport.
- Public packages must remain organization-neutral.
- Read a package's child `AGENTS.md` before editing it.

## Verification

- `python3 hub/router.py list`
- `python3 -m py_compile hub/router.py transport/slack.py`
- run the narrow checks in the touched package contract

## Child index

- `file-courier/` — generic Slack file/message delivery →
  `file-courier/AGENTS.md`
