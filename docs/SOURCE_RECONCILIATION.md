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

Bounded personal-source slices landed on Kingmaker main, newest first. Each
entry names the personal source tip the slice was ported from and what was
deliberately left behind.

- 2026-10-06 — Minimizer M2 output economy (PR #49, personal tip
  `1bd1bc37636e0a4363f1f20aa1b72ee4c79b14cb`): the first CONFLICT-CLASS
  reconciliation — `agent_loop.rs` three-way merged (Kingmaker's
  token-aggregation and `Option<ThinkingLevel>` fixes preserved verbatim; the
  single textual conflict resolved to Kingmaker's types), `output_economy.rs`
  provider-only tool-result projections (PinBudget-bounded, exact provider
  ordinals, sealed from emission) with the m2a/m2b characterization suites,
  the `execute_for_run`/argv-mode capability seam, `ArtifactLease`/`PinBudget`,
  the default-off `SessionContext::command_output_minimization` gate (M2c
  profile enablement remains separately reviewed), and the ocean-minimizer
  dependency. NOT ported: personal's +1234-line metrics expansion beyond
  what later units need.

- 2026-10-06 — Rooms S0 participant retirement (PR #25, personal tip
  `1bd1bc37636e0a4363f1f20aa1b72ee4c79b14cb`): the daemon operator route
  `POST /v1/rooms/persistent/{key}/participants/{id}/retire` plus its governing
  spec. The store retirement core already existed on Kingmaker main from the
  publication snapshot. PR #25 also hardened that store implementation with
  transaction-bound retired-alias reservation and bounded alias projection and
  inspection. The remaining Rooms files (`room_maintenance`, `room_context`,
  `room_attachments`, `room_summary`, `room_workspace_proxy`, `room_inspect`)
  still await their units on top of the persistent_rooms rework.
- 2026-10-06 — Observatory durability cluster (PR #31, personal tip
  `1bd1bc37636e0a4363f1f20aa1b72ee4c79b14cb`): versioned idempotent
  `observatory.db` schema migrations (v1 baseline → v2 §4.1 rebuilds), the v2
  store with backfilled correlation/producer/recorded_at, §7.3 envelope
  replay, admission-wiring and observer-token gates, and the daemon half
  (first production caller for the G3 retention loop, checkpointing, extracted
  durability pump, summary-token rotation with failure metrics). NOT ported:
  personal's +1234-line metrics.rs expansion beyond the rotation-failure
  counter. Kingmaker had not touched these files since the split.
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
