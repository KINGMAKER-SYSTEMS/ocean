# Canonical Ocean repositories

`KINGMAKER-SYSTEMS/ocean` is the public shared development repository.
`KINGMAKER-SYSTEMS/ocean-private` holds private service code, production agents,
SOPs, company research, personal deployment material, and the complete original
mixed-repository history and PR discussions. All four maintainers have access
to both. Personal source repositories are preserved but are not the destination
for new shared Ocean changes.

The public repository starts with a clean publication snapshot based on private
main `81eb9fff5e438858e8ada8b7651c8e5609c838b4`, plus the lighter CI change.
It deliberately has no ancestry containing private material. Component licenses
and notices are retained. No runtime behavior changes are bundled into the split.

## Existing work

1. Clone the public repository into a fresh directory; do not reset a dirty checkout.
2. Preserve existing mixed-history branches in the private repository.
3. Port only the public-component diff onto the new public `main`; do not merge,
   subtree-pull, or push old mixed-history branches into the public repository.
4. Open new public-component PRs in `KINGMAKER-SYSTEMS/ocean` and private-component
   PRs in `KINGMAKER-SYSTEMS/ocean-private`.
5. Existing private PRs remain available for reference; their numbers and links
   belong to the private repository after the rename.

Public builds run on standard GitHub-hosted runners. Private production workflows
remain private and are not automatically enabled by this split.
