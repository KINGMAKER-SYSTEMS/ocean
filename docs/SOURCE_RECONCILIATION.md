# Ocean source reconciliation

Kingmaker Ocean is the canonical destination for shared development.
The personal repositories are preserved as source history and point their
GitHub descriptions and homepage links to this repository.

## Verified source tips — 2026-10-05

| Component | Personal source tip | Source commits after original import | State |
| --- | --- | --- | --- |
| Runtime | `b7839a45584845b80d5429f157111de40b0c3a33` | 298 | Diverged; reconcile selected changes with Kingmaker fixes |
| Surface | `1b88f862ac9f04a843c53e3464e0b8600aa873c2` | 176 | Diverged; reconcile selected changes with Kingmaker fixes |
| Reusable packages | `f21d05f610d05aaf024f7e272719ea66bd206142` | 0 | Source tip already represented in the publication snapshot |

Counts describe source ancestry since the original import, not missing Kingmaker
commits: Kingmaker already contains independently integrated runtime and client
fixes. The public main checked was `0b7e20f1a9c0f719f165cbb98cbe6fb32a927344`.
Its Build scope, Build Ocean, and Build Surface jobs all passed.

## Content merge preview

A local comparison using the original source snapshots as content bases found
56 conflicting paths. The conflicts span Rooms state/authority/resources, provider
encoding and catalog, OAuth custody, runtime event handling, client hydration/UI,
and monorepo deployment rails. These are not resolved by selecting the newest
whole tree: each side contains changes that must be preserved.

No source changes from that preview were pushed or merged. Temporary comparison
commits have no place in public history. Existing model/UI work in public PR #2
remains separate and is not declared landed.

## Reconciled slices

Bounded slices landed on Kingmaker main, newest first. The October 6 receipts
identify public merge commits and landed behavior; they do not establish source
parity, deployment, or live verification.

- 2026-10-06 — Minimizer M2 output economy
  ([PR #49](https://github.com/KINGMAKER-SYSTEMS/ocean/pull/49), merge
  `1e3655e4946c428a50871bcccdb4a0cde1854343`): provider-only tool-result
  projections, bounded artifact pinning through `ArtifactLease`/`PinBudget`,
  the `execute_for_run`/argv-mode capability seam, and the M2 characterization
  suites. The merged head includes the redacted `ArtifactLease` Debug formatter
  and sentinel regression for unrelated session output exposure (issue #50).
  `SessionContext::command_output_minimization` remains default-off, with no
  production setter in this change; profile enablement requires separate review.
- 2026-10-06 — Observatory durability cluster
  ([PR #31](https://github.com/KINGMAKER-SYSTEMS/ocean/pull/31), merge
  `8c7a1465ad33657bf64a94efaa38e4dc0d501407`): versioned restart-safe schema
  migrations, retention/archive persistence, bounded envelope replay, admission
  wiring and observer-token gates, plus daemon retention, checkpointing,
  durability-pump, and summary-token rotation work. Replay validates the retention
  boundary and reads its page under one store lock; cursors strictly before the
  boundary return 410, while equality can resume. Snapshot cursor and retention
  state are read under the projection lock, and response headers use the returned
  projection watermark. These consistency fixes were included before merge.
- 2026-10-06 — Rooms S0 participant retirement
  ([PR #25](https://github.com/KINGMAKER-SYSTEMS/ocean/pull/25), merge
  `855e0c40029fb74e684fd935f9c4d419370ffc32`): the operator-authenticated
  `POST /v1/rooms/persistent/{key}/participants/{id}/retire` route and governing
  specification, with retired-alias reservation inside roster-write transactions.
  Detail, snapshot, and read-only `GET /v1/rooms/persistent/{key}/inspect`
  responses expose up to the oldest 256 aliases and `aliases_truncated` to report
  incomplete projections. Inspection returns room identity, local ownership,
  and aliases without transcript or workspace contents. This slice does not
  establish maintenance, context, attachments, summary, or workspace-bridge parity.
- 2026-10-05 — OAuth custody cluster (PR #23, personal tip
  `1bd1bc37636e0a4363f1f20aa1b72ee4c79b14cb`): operator-authenticated
  `/v1/auth/providers*` login/status/logout routes (`ocean-daemon/src/provider_auth.rs`),
  `ocean-oauth` token-free block status and atomic block removal. NOT ported:
  the personal `oauth_refresh.rs` rewrite (Kingmaker's version is the hardened
  evolution — full-block merge comparison, failure taxonomy, custody-timeout
  handling, no response bodies logged), the token-exchange response-body error
  hunk (contradicts the crate's fixed-classification doctrine), and the
  personal temp-path custody internals (the guard publisher already provides
  the same durability). Adapted for Kingmaker-only credential sources
  (`ClaudeCodeKeychain`, `ClaudeCodeCliAuthFile`).

## Reconciliation contracts

- Port selected public diffs onto current public main, preserving Kingmaker fixes.
- Keep private records, historical ledgers, research, deployment destinations, and
  mixed ancestry in the private companion repository.
- Read every affected component/crate devlog before editing its source.
- Reconcile in bounded units with relevant tests and builds; preserve root
  build-only CI instead of importing the personal CI matrices.
- Runtime logic, security, protocol, and architecture changes require fresh
  reviewer acknowledgement under the runtime component contract before landing.
- Keep the source-tip table current when reconciliation actually lands.
- Until reconciliation is validated and merged, do not claim the organization
  contains every newer personal-repository change.
