# ocean-memory — Scoped Durable Memory

## Purpose

Own typed provenance-bearing SQLite memory, operator and admitted Room
partitions, schema compatibility, and ingest adapters.

## Ownership

- Scope: `crates/ocean-memory/`; parent contracts: `../../AGENTS.md` and
  `../AGENTS.md`.
- `src/lib.rs` owns memory vocabulary, scoped store authority and migration.
- `src/ingest.rs` owns existing evidence-to-memory ingestion.
- `src/bin/ocean-memory-rollback.rs` owns the explicit offline legacy rollback
  command; it is not a daemon startup or installation hook.

## Local Contracts

- Legacy `MemoryStore` APIs operate only in `operator:v1`. Operator IDs remain
  globally unique across owners; Room logical IDs are namespaced by partition
  and owner. Every read, list, count, search, write and delete retains that scope.
- A Room partition is minted only from typed `RoomMemoryAdmission` evidence,
  with exact UTF-8 byte-length key encoding and the fixed shared Room owner.
  Opaque scopes expose no caller-selected partition or Room setter. Disabled
  scope refuses all operations without mutation.
- Migration preserves operator rows, existing Room rows and monotonic sequence
  state across reopen. Ambiguous operator IDs fail closed before rebuilding;
  no error may silently merge or discard memories.
- A binary-only downgrade to a pre-partition memory implementation is unsafe:
  its unscoped queries can expose Room rows. `prepare_legacy_rollback` archives
  non-operator rows and restores the old single-ID schema transactionally;
  upgrading rehydrates the archive. The command requires `--offline-confirm`,
  an absolute existing regular database file, and every database user stopped.
- This library stage supplies execution-context interfaces only. Daemon
  admission and route activation are dependent work; the production Rooms
  migration hold remains in place.

## Work Guidance

- Preserve ordinary operator-memory behavior and existing ingest vocabulary.
- Use isolated temporary databases for migration and rollback fixtures. Never
  run fixture commands or the offline utility against an operator database.
- Keep cross-room, owner, disabled-scope, reopen and rollback fixtures with the
  persistence implementation; schema changes require their regression proof.

## Verification

- `cargo test -p ocean-memory --locked`
- `cargo clippy -p ocean-memory --all-targets --locked -- -D warnings`
- `cargo check --workspace --tests --locked` for dependent consumers.

## Child devlog Index

No child boundaries defined within `ocean-memory/` at this time.
