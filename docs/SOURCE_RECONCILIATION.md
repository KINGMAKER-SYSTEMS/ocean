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
