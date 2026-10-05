# ocean-providers — Model and Credential Routing

## Purpose

Own Ocean's provider/model catalog, credential resolution, readiness and shared
auth-file write custody. Provider wire encoding remains in `ocean-protocol`.

## Ownership

- Scope: `crates/ocean-providers/`; parent: `../AGENTS.md`.
- `src/lib.rs` owns routing/catalog and exports the shared custody interface.
- `src/auth_file.rs` owns cooperating Ocean writers' lease and publication.

## Local Contracts

- Preserve forward model/catalog routes and feature credential isolation.
- Public Codex routes include GPT-6.1 Sol, GPT-6 Sol, GPT-6 Luna and GPT-6
  Astra. Claude Opus aliases track 5.5 (1M/128K); explicit 5/4.x ids remain
  routable for pinned sessions. Fable aliases retain Fable 5.1.
- Every public catalog id resolves back to the same wire id and provider.
  Credential readiness remains separate from account entitlement and live inference.
- OAuth login/API-key storage and Agent refresh acquire `lock_auth_file` before
  fresh read/merge/publication. Never hold custody across network I/O or await.
- The process mutex and exclusive sibling file lock share a five-second
  contention deadline. Open/lock/timeout errors refuse the write; there is no
  unlocked fallback. Dropping custody never removes the shared lock file.
- A guard binds its configured auth path and publishes unique create-new sibling
  files, Unix0600 at creation, file sync then atomic replacement. Only owned
  temporary files receive bounded best-effort cleanup.
- Pre-rename failure preserves the previous file. Post-rename Unix directory-sync
  failure means published bytes with durability unconfirmed; do not restore a
  stale snapshot. Directory fsync remains unsupported off Unix.
- Errors carry fixed classes/kinds, never credential bytes or provider bodies.
- This coordinates cooperating Ocean writers; external CLI writers are not
  automatically enrolled. Tests use disposable synthetic files only.

## Work Guidance

- Keep custody separate from catalog/model changes and provider login routes.

## Verification

- `cargo test -p ocean-providers auth_file --locked -- --test-threads=1`
- `cargo test -p ocean-providers --locked`
- Credential consumer changes also require OAuth store and Agent refresh tests.

## Child devlog Index

- No child boundaries.
