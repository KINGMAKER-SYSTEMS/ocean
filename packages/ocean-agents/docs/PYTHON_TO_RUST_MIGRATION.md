# Public Python-to-Rust Migration

Status: direction; deletion requires verified parity.

## Goal

Move executable transport and discovery into typed Ocean extensions/runtime
components while keeping `ocean-agents` as portable package data: profiles,
manifests, schemas, SOPs, and fixtures.

## Current public Python paths

| Path | Role | Intended replacement |
| --- | --- | --- |
| `assistants/tools/compose_profile.py` | Deterministic profile composition | Rust xtask or runtime composition |
| `assistants/bonzai/harness/bonzai.py` | Git-hygiene reference harness | Typed extension/tool implementation |
| `couriers/hub/router.py` | Manifest discovery and dispatch | Daemon/extension package resolver |
| `couriers/transport/slack.py` | Shared outbound Slack transport | `ocean-slack` extension |
| `couriers/file-courier/harness/courier.py` | Reference delivery harness | Typed extension tools/contracts |

Deployment-specific intake adapters are outside this public repository.

## Cutover rule

For every path:

1. build the Rust replacement;
2. migrate all public callers;
3. prove behavior and error parity;
4. run both paths during bounded validation where appropriate; and
5. delete Python only after the replacement is accepted.

Do not bundle redesign into a parity extraction.

## Order

1. Shared Slack transport.
2. Typed package manifest parsing and discovery.
3. File-delivery tools and contracts.
4. Bonzai git primitives.
5. Profile composition.

The profile composer is low operational risk and can remain a build tool until
runtime composition has a stable contract.

## Completion gate

The migration is complete when:

- package discovery no longer depends on Python;
- transport/auth/retry behavior is typed and tested;
- public package manifests validate against a stable schema;
- generated profiles remain deterministic or runtime composition supersedes
  generated files;
- no public documentation references deleted harnesses; and
- `ocean-agents` contains only portable package data and fixtures.
