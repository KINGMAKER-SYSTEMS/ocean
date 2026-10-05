# Internal Extension Registry Transactions

## Purpose

Own the Stage A3a internal local registry engine beneath the sole registry
schema, digest and descriptor authority in `../extension_registry.rs`.

## Ownership

- `transaction.rs` owns local quarantine, typed mutations, four-file publication,
  journal recovery, retention and exact grant preview/confirmation, including
  its synthetic in-file fixtures.
- Parent contracts are `../../AGENTS.md` and the root-to-crate devlog chain.
- No HTTP/CLI, AppState, startup composition, live supervisor binding, Git
  acquisition, package execution or new dependency belongs here.

## Local Contracts

- The accepted Stage A manifest sections 5, 6, 12, 13.1, 14, 17, 18, 19.4 and
  21 govern this engine. Source input is the bounded earlier A3a `7804b39f`;
  later A3b/A4/A5 behavior is excluded.
- Four acquisition permits are process-wide across writer instances and config
  roots. Canonical aliases share cleanup custody; idle root accounting is
  removed. Local sweep admission waits at most 250 ms before a typed busy
  refusal. Acquisition/copy/hash never holds `.state.lock`.
- Persistent `<config>/.extensions-acquisition.lock` supplies cross-process
  custody: acquire a shared lock before any bootstrap/quarantine directory and
  retain it through the acquisition lease; both orphan sweeps require exclusive
  custody. Verify the no-follow descriptor's owner, single link, exact 0600 mode,
  empty regular-file shape and named inode. Never unlink or rewrite it. An
  in-use or unprovable lock refuses destructive recovery; local live leases
  preserve the existing skip-sweep behavior while journal recovery proceeds.
- Existing schema-1 shapes, limits and `sha256-tree-v1` encoding stay unchanged.
  All publication/recovery holds the existing 250 ms exclusive file lock and
  uses descriptor-relative, no-follow custody. The first state rename commits;
  recovery then rolls forward verified bytes, never guesses an old revision.
- A marker is published only with a complete journaled generation or one atomic
  absent-root publication. Strict journals bind operation, checked next revision,
  exact lowercase hashes and staged names. Missing/corrupt evidence remains a
  typed recovery error; known commit identity/revision is retained, unknown
  facts are not represented as an effective old revision.
- Every directory sync error, including unsupported-barrier errors, fails
  closed. Before commit retain the old proven generation; after a proven commit
  report the known next revision, never success without all barriers. Journal
  retirement failure attempts to restore committed proof; further restoration
  failure remains an error, never a successful durability claim. Atomic root
  publication retains its complete committed root on a failed parent barrier;
  recovery must prove that barrier too.
- Install grants no trust/enablement; update revokes all package trust and
  service grants. Update/remove require fully disabled state and opaque trusted
  stopped-package input. Implement every section 12.4 retention transition;
  delayed removal cleanup never deletes a subsequent install's state.
- Confirmation binds id, digest, current revision, canonical added/removed grants
  and the exact native authority notice. References only, never secret values.
- Writer support is Unix-only: reviewed no-replace rename primitives on macOS
  and Linux, fail-closed elsewhere. Preserve the actual-source Windows reader
  and unsupported supervisor behavior. This engine alone does not accept A3a,
  authorize later stages, or release the live migration hold.

## Work Guidance

- Issue #52's exact base is main51 `d1220f53fccbef0dc5902ec1732e93db526ea439`.
  Scope is this file, `transaction.rs`, the parent reader and Daemon contract,
  plus the crates ownership cell. Keep public/API and dependency fanout absent.
- Preserve all existing reader fixtures and the 22 original A3a fixture groups.
  Add process-wide/canonical-alias capacity, strict journal and truthful
  committed-recovery assertions using disposable files and no-execution canaries.
- Separate-process fixtures hold real copied acquisition bytes across both
  sweep attempts, then crash the owner and prove orphan cleanup. Synthetic
  descriptor/mode replacement, deadline and pre/post-commit directory-sync
  failures must preserve custody and truthful outcomes without package execution.
- Root owns Cargo, independent review, reconciliation, PR and release proof.
  Synthetic auth/config overrides preserve real HOME; do not inspect owner state.

## Verification

- `cargo test -p ocean-daemon extension_registry::transaction::tests::a3a_ --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon extension_registry:: --locked -- --nocapture --test-threads=1`
- Daemon all-target Clippy, test linking, workspace test compilation and the
  existing actual-source Windows reader check remain parent-owned checks.

## Child devlog Index

- None.
