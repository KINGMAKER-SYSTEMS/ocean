# internal/ — private material relocated from public Ocean repos

## Purpose

Holds proprietary/internal content extracted from the public Ocean repositories
during the 2026-07-19 knowledge-separation migration. This repo is private;
these files must never move back to a public repo without explicit operator
approval.

## Ownership

- **Scope:** `internal/` tree only
- **Parent contract:** `../AGENTS.md`

## Contents

- `ocean-os/skills/` — Ocean OS operational skills (software-factory,
  coworker-onboarding) removed from the public `ocean-os` repo. Live copies for
  Longhouse discovery are installed at `~/.config/ocean-rs/skills/`.

The factory/orchestration operating docs moved to `../docs/orchestrator/`
(the Longhouse workflow-discovery path `{cwd}/docs/orchestrator/workflows/`),
not here.
