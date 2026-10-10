# Ocean Extension A3b Contract Amendment

**Date:** 2026-10-03

**Status:** PROPOSED — neither decision is operator-ratified.

**Decision record:** Pending. Record explicit operator rulings for both decisions in §5 or a linked canonical public decision record before implementation.

**Parent:** [accepted Stage A implementation manifest](2026-07-27-ocean-extension-stage-a-implementation-manifest.md), especially §§10.1, 12.3, 14–18 and 21.

**Evidence base:** The public A3a registry writer in `crates/ocean-daemon/src/extension_registry/transaction.rs` preserves independently optional commit and revision facts. Source presence does not establish slice acceptance.

## 1. Authority and concrete conflicts

The parent authorizes complete A3b after A3a. Its accepted text remains unchanged.
This proposal resolves two conflicts before binding the writer to HTTP, CLI and
the live supervisor; it authorizes no source implementation by itself.

1. Parent §15 requires boolean `committed` and numeric `state_revision` in
   mutation errors. A3a can independently lack proof of either fact: a corrupt
   journal cannot prove a commit, and an unreadable namespace can leave the
   revision unknown even when this attempt is proven pre-commit. The writer's
   optional facts must not be converted into a guessed boolean or revision.
2. Parent §§10.1 and 14 require revision reconciliation and grant-change service
   restart/stop. Sections 16, 19.4 and 21 also forbid package execution during
   trust. Existing enabled scopes can survive a grant change; a confirmed grant
   can therefore make a service newly effective without a new enable mutation.

Both decisions require explicit operator rulings recorded in the canonical public decision record before
A3b source begins. Review acknowledgement or merging this proposal does not
ratify either choice. Silence grants no authority. No startup-only substitute
or inferred disable-before-trust restriction completes the accepted A3b slice.

## 2. Decision 1 — preserve independently proven outcome facts

**Recommended amendment to §15:** `committed` is `boolean | null` and
`state_revision` is `nonnegative integer | null` in mutation error envelopes.
Null means that particular fact is unproven; it does not mean false or zero.
Keep every independently proven fact, including an actual revision of zero when
that is the proven empty/A0 namespace revision. Committed Stage A publications
still require the parent's nonzero revision.

- Prove facts from the writer's commit point and validated journal/generation.
  Never substitute caller `expected_state_revision`, false, zero, a stale cache,
  or an unrelated later reread for missing evidence.
- Preserve the writer-returned operation UUID and the facts proven for it.
  Failure while recovering a prior journal retains that journal's UUID, commit
  and revision; never relabel it as the current request's commit. A successfully
  recovered prior generation reconciles independently. CLI claims refer to the
  reported operation, which can be prior; a recovery error does not establish
  that the current request was admitted. No durable operation ledger, replay
  token or new route is introduced. A lost/unvalidated response leaves the id
  unknown unless independently proved; never invent one or recover it by guess
  from cached status.
- Unknown evidence is never a successful mutation response. Keep an already
  proven closed error code and its fixed safe message. Otherwise use proposed
  fallback `registry_outcome_unknown`, with fixed message
  `extension registry outcome is unproven`.
- **Proposed HTTP choice:** preserve an established pre-commit HTTP status,
  closed code and fixed message when `committed:false`, even if the revision is
  unproven. Unknown commitment, or proven commitment without a proven revision,
  returns `503`; the unknown fallback also uses `503`. These new terms require
  review and operator acceptance. Proven coherent/committed-recovery responses
  keep their accepted classifications. Null revision alone must not relabel a
  known input/platform/capacity refusal as a transport-availability failure.

Examples of proposed HTTP `503` errors with no established closed cause,
preserving the admitted operation id:

```json
{"ok":false,"mutation":{"operation_id":"<actual-uuid>","committed":false,"state_revision":null},"error":{"code":"registry_outcome_unknown","message":"extension registry outcome is unproven"}}
```

```json
{"ok":false,"mutation":{"operation_id":"<actual-uuid>","committed":null,"state_revision":9},"error":{"code":"registry_outcome_unknown","message":"extension registry outcome is unproven"}}
```

`(null,null)` and `(true,null)` use `503`; proven `true` must remain true.
An established `invalid_source` refusal, for example, retains HTTP `400` and its
fixed reason with `(false,null)`; its CLI exit is still 5 because the revision is
unproven. Neither raw journal bytes, environment values, paths, response bodies,
stderr nor exception text belongs in the public error.

| Validated mutation result | CLI exit | Required interpretation |
| --- | --- | --- |
| Proven HTTP `200` completion | `0` | Applied commit and required reconciliation complete; trust preview retains its separate unapplied shape. |
| Coherent committed HTTP `202` | `3` | Print actual operation/revision plus pending or blocked reconciliation/reap; never retry. |
| HTTP `500`, committed `true`, proven revision, `registry_recovery_required` | `4` | Commit is proven; coherent recovery failed. Direct recovery, never mutation replay. |
| Either `committed` or `state_revision` unproven, including `(false,null)` | **`5` (proposed)** | Print each known fact and `unknown` for each unproven fact. Do not claim rollback, success or the old revision. |
| Mutation response lost, invalid or inconsistent with its HTTP status | **`5` (proposed)** | Outcome, revision and operation id remain unknown unless independently proved; do not print invented reconciliation/reap facts. |
| Fully proven ordinary pre-commit refusal | Existing failure exit | Preserve the parent's refusal and facts; no silent conflict retry. |

Parse and validate mutation envelopes before generic HTTP error handling, so
known committed `500` responses retain exit 4. Print committed, operation id,
revision, reconciliation and reap, with `unknown` for absent/unproven facts.
Never claim that a cached status or inspect result proves the original operation.
Exit 5 forbids automatic mutation retry/replay. Before another explicit mutation,
establish a coherent current generation through independently validated reads
or journal-proven recovery when required. A known pre-commit input/platform
refusal does not require recovery merely because its revision is unknown; a new
explicit corrected request may follow a validated coherent read. That read
never retroactively proves the old request's outcome. Transport availability
alone proves no recovery. Phase 1 read response/error schemas remain unchanged.

## 3. Decision 2 — choose explicit live-trust execution semantics

### A — recommended: postcommit reconciliation under retained enablement

Amend §§16, 19.4 and 21 to distinguish trust preview/publication from the
supervisor consequence of a separately committed confirmed grant:

- Acquisition, validation, preview and registry writer publication never launch
  or probe package content. Preview remains read-only and no-execution.
- After a coherent confirmed trust commit, the existing supervisor may start,
  restart or stop individually acknowledged services under package scopes that
  were already enabled and retained. This includes starting a previously
  ungranted service that becomes effective. Trust never creates, widens or
  implicitly reenables package enablement; fresh or fully disabled trust stays
  inert until a separate enable mutation.
- Preserve exact installed digest, current revision, canonical capability and
  per-service grant rows, explicit native acknowledgement, complete binding
  checks, the exact frozen native-authority notice, and its confirmation hash.
  Before preview and confirmed-apply submission, the CLI displays this fixed
  presentation supplement outside the JSON payload and frozen notice/hash:
  "Applying confirmed trust may start newly granted services, restart changed
  services, or stop revoked services within already enabled package scopes.
  Trust does not create or widen enablement; disabled packages remain inert
  until separately enabled." Preserve machine-readable stdout; this supplement
  uses the human-facing diagnostic stream. Static proved selected-scope facts
  may explain authority transitions; unknown other scopes, project snapshots,
  bindings and runtime custody remain conditional. Existing public data does
  not prove an exhaustive action plan. No new preview field, probe or notice/
  hash change is required; actual effects belong to owned reconciliation/status.
- Route completion tracks this postcommit consequence through the existing
  common envelope: `200` only when required reconciliation/reap is complete;
  committed `202` exposes pending/blocked work. A HTTP response does not detach
  or hide the owning reconciliation task.

### B — alternative: explicit disable and reap before confirmed apply

If selected instead, every scope must be disabled and every package service
fully reaped before confirmed trust applies. Otherwise refuse pre-commit with
`extension_active`; never implicitly disable. Preview remains read-only.
The operator then confirms trust and separately enables. This is a **new
precondition**, requires explicit selection, and is not inferred from the
existing authorization. It intentionally prohibits A's live grant transition.

### Safety common to either choice

Remove old delivery eligibility and replace its activation epoch immediately
when a grant/scope transition takes effect, before cleanup; changed authority
cannot use an old cursor or stale status generation. Retain generation-safe
old process-group custody through cleanup; replacement may start only after
the old group is proven gone and its leader is reaped. Committed `202` retains
cleanup ownership, fences removed delivery and keeps update/remove blocked.
A remaining effective scope may keep the shared service alive after partial
disable while removed scope delivery is already fenced. These are metadata
and process lifecycle guarantees: Stage A does not atomically revoke native OS
filesystem/network effects, contain a hostile child, or undo its past effects.

## 4. Complete A3b implementation and acceptance after both rulings

Keep the entire §18 A3b boundary: exact local HTTP/CLI mutations, common outcome
envelopes, cached reads, startup recovery, and revision-serialized supervisor
reconciliation/reap over A3a. Retain source ownership in §6 and every unaffected
parent invariant. The implementation must prove the following together:

- Perform validated journal recovery before extension reader/service admission
  at startup. Corrupt or mixed generations fail closed without blocking
  ordinary daemon availability. Reconcile only coherent proven generations.
- Serialize registry changes, project snapshots and retries with revision and
  activation-generation checks; stale work may finish cleanup but cannot
  overwrite newer status. Coherent empty activation sets retain their revision.
- Recovering a prior journal can advance the namespace even when the current
  attempt then fails pre-commit or conflicts. Reconcile the independently proven
  recovered generation without assigning its commit to the current operation.
- Reserve four acquisition permits outside both publication and supervisor
  actor locks. A 60-second held local acquisition must leave coherent readers,
  ordinary daemon work and the other three acquisitions available.
- Hold actor-owned stopped-service custody through update/remove publication,
  including starting/backoff services, retained cleanup failures and temporary
  roots. No PID-null or empty-cache snapshot constitutes stopped proof. Recheck
  disabled scopes and publication preconditions under the registry lock.
- Keep one bounded daemon-owned operation alive through acquisition, stop
  fencing, publication, reconciliation and terminal cleanup. Request timeout,
  disconnect or worker-observation timeout is not task cancellation or terminal
  proof; never drop/detach the sole owner. Shutdown retains cleanup responsibility.
- Report retention cleanup and pending reap truthfully. Preserve descriptor/
  no-follow anchoring, expected-revision conflicts, strict request DTOs, relative
  CLI path canonicalization, percent-encoded/revalidated ids, fixed-safe errors,
  A0 schemas and old read fixtures. Keep Host/CORS/cross-site guards; they are
  not caller authentication or authority to borrow another endpoint's token.
- Keep Windows reads and cached unsupported status usable; unsupported mutations
  fail before acquisition/spawn. Do not add a portable writer or invent process
  cleanup proof. Ordinary sessions, hooks, plugins and Observatory stay intact.

Required composed fixtures cover first-rename crash recovery; corrupt/missing
staged bytes and unknown facts; known pre-commit/conflict and prior recovery;
actual CLI exits 0/3/4/5; lost/malformed responses without retry; stale concurrent
publication/status; held acquisition and readers; fresh disabled trust canaries;
separate enable; selected live-trust start/restart/revoke behavior or B's explicit
refusal; immediate epoch/delivery fencing with committed-202 held cleanup;
active update/remove refusal and publication while stopped custody is held;
retention transitions, unsupported Windows, and ordinary daemon availability.
Canaries remain untouched in every no-execution phase; A's explicitly confirmed
postcommit supervisor execution is observed separately, not passed as a canary.
A disclosure fixture preserves the frozen preview/hash and JSON stdout, proves
the fixed supplement appears before apply submission, and checks actual effects
only through the revision-bound supervisor owner/status.

Run the owning registry/service/route/CLI checks, meaningful security fixtures,
format/workspace test compilation and documentation checks, then obtain fresh
exact-head independent review before landing the full slice. Automatic root CI
remains only Build Ocean / Build Surface; this proposal adds no CI matrix.

## 5. Ratification and closeout

Record explicit rulings for decisions 1 and 2, including any changed HTTP/error
terms. Only then update the parent's conflicting §§10.1, 14–16, 19.4 and 21
consistently and authorize A3b source under its existing gates. Until that
record exists, this document remains PROPOSED even if its PR is merged.

This documentation-only change is verified by Markdown targets,
`cargo xtask docs-check`, `git diff --check`, independent exact review and
byte-unchanged accepted-manifest evidence; none proves runtime implementation.
It authorizes no A4 Git acquisition, A5 integrated closeout or Stage A acceptance,
Crew Stages B–E, private knowledge/provider/integration work, activation,
installation, scheduling or account changes, and does not release migration
holds or the parent's §22 acceptance boundary.
