# Public/Private Repository Separation — Executed

Date: 2026-07-20
Status: executed

## Decision

The former mixed `Risingtides-dev/ocean-agents` repository was renamed to
`Risingtides-dev/risingtides-agents` and retained as a private production
repository. A fresh public `Risingtides-dev/ocean-agents` repository was created
from an explicit allowlist with new history.

No production files were deleted. The private repository preserves the original
Git history, issue/PR metadata, production package catalog, internal workflow
docs, and event ledger.

## Boundary rule

> Public contains reusable Ocean mechanism. Private contains Rising Tides
> operating behavior.

A package is public only when it can be used without Rising Tides accounts,
knowledge, destinations, artists, campaigns, media, private SOPs, internal
workflows, or Bedrock data.

## Public repository

The fresh public repository contains 92 files in its root commit, including:

- `_shared` and `_base` profile composition sources;
- generated profiles for ten client surfaces;
- profile composition tooling and CI drift checks;
- Bonzai as the generic named-assistant reference;
- courier manifest routing and generic Slack transport;
- file-courier as the generic courier reference; and
- public architecture and daemon interaction documentation.

It excludes:

- all production named assistants;
- all artist/campaign/distribution/media couriers;
- the production Slack Socket Mode bridge;
- named PR-maintenance automation tied to an existing deployment;
- factory/orchestrator docs and internal skills;
- production events, handoffs, media, and private roadmaps; and
- all credentials and destinations.

GitHub secret scanning and push protection were enabled immediately after
creation.

## Private repository

`risingtides-agents` retains:

- content-agent, creator-research, prompt-lab, voice-intake, and
  yt-prompt-bible;
- sound-watcher, email/distribution/media couriers, yt-clipper,
  pr-babysitter, and other production packages;
- Slack intake and deployment harnesses;
- `docs/orchestrator/` and `internal/`;
- package roadmaps, operating references, and the historical event ledger.

## Bedrock boundary

Private package behavior remains here. Authenticated company knowledge,
context, handoffs, durable records, graph/search, and team data remain in
`ocean-bedrock`. Bedrock is not a replacement package monorepo.

## Verification at cutover

- all ten public composed surface profiles matched authored sources;
- the public router listed only `/ship`, `/say`, and `/resolve`;
- public Python, TOML, and shell syntax checks passed;
- public Markdown local-link validation passed;
- prohibited private-name/path scans returned zero findings;
- credential-shape scans returned zero findings;
- public GitHub visibility, secret scanning, and push protection were verified;
- private GitHub visibility was verified after rename.

## Historical exposure note

The former repository had previously been public, so old commits must be treated
as disclosed even though the original history is now private. The fresh public
repository prevents continued mixing and establishes the licensing/provenance
boundary for future work; it does not revoke prior clones or caches.
