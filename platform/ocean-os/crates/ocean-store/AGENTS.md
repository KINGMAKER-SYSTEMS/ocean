# ocean-store — Durable Rooms + Federation Store

## Purpose

SQLite-backed durability for persistent rooms: rosters, transcripts, room
access projections, the outbox, normalized federation mentions, local execution
bindings and replay decisions, attachments, profiles, resource grants and
participant retirement. One database file (`rooms.db`), one owning crate.

## Ownership

- **Scope:** `crates/ocean-store/`
- **Parent contract:** `../AGENTS.md` — read it first
- **Owns:** room/roster/transcript persistence, `room_access` + `outbox`
  durability, local and mirrored room read cursors, federation credential
  custody, producer counters, confirmed ingest, trigger-claim journal;
  local owner roles, agent authorization, immutable decision replay, attachment
  metadata, profiles, resource grants/audits and participant aliases
- **Does not own:** HTTP projection (daemon), federation network client,
  agent sessions/memory, Longhouse titles

## Local Contracts

### Schema (private; consumers use APIs, never SQL)

- `rooms`, `participants`, `messages` — durable rooms, rosters and transcripts.
  `messages.attachment_id` is nullable; migration preserves existing rows.
- `room_agent_owners` records local attribution; `room_local_roles` separately
  records Local owner authority. Attribution is not execution admission.
- `room_agent_bindings` and `room_agent_decisions` store exact definition
  digests, capabilities, activation/status, canonical-TEXT generations and
  immutable authorization decisions.
- `room_attachments` stores immutable metadata only. Blob custody, server id
  validation and filesystem ordering belong to the daemon.
- `federated_event_mentions` stores validated member ids separately from trigger
  claims; `federated_event_mentions_known` distinguishes authoritative empty sets
  from legacy unknown metadata.
- `src/room_profile.rs` owns reference-only `room_profiles` and immutable
  `room_profile_decisions`; no credential values belong in profiles.
- `src/room_resources.rs` owns `room_resource_grants`, immutable
  `room_resource_decisions` and content-free `room_resource_audit`.
- `src/room_retirement.rs` owns `room_participant_aliases` and immutable
  `room_retirement_decisions`; aliases preserve frozen historical bindings.
- `room_access` — per-room access projection (`state`, `confirmed_sequence`
  as canonical decimal u64 TEXT, `member_projection` JSON).
- `outbox` — locally-authored unconfirmed events with full producer tuple
  (`client_event_id`, `source_id`, `source_sequence`) and stable `position`.
- `room_read_cursors` and `room_read_cursor_mirrors` — per-principal local and
  upstream-mirrored read positions as canonical decimal u64 TEXT. Mirror writes
  use `RoomReadCursorMirrorCas`: callers supply the previously observed mirror;
  mismatches return `Stale` without writing, including stale clears.
- `room_agent_runs` — team-platform P3 mutable work-card projections
- `room_agent_settings` (team-platform P4) keys per-room agent overrides by `(room_id, agent_id)` with a JSON `RoomAgentSettings` body that cascades with the room; writing empty settings deletes the row. `AwaitingReply` runs are parked, not open: `interrupt_open_room_agent_runs` leaves them parked, and `parked_room_agent_runs(key, thread_root_seq)` finds them for resume. Resumption is a two-step compare-and-swap under immediate transactions: `claim_parked_room_agent_run(run, answer_seq)` succeeds for exactly one answer while the run is parked and unclaimed; `settle_room_agent_run_answer(run, answer_seq, resumed)` then closes it `Done` (successor admitted) or releases the claim (still parked), and touches nothing claimed by another answer. At startup an outstanding claim settles `Done` only when a run triggered by that answer exists for the same room and agent; otherwise it is released.
  `{run_id, room_id, started_at, body JSON RoomAgentRun}` (cascade with the
  room). Not a transcript: `put_room_agent_run` upserts, `room_agent_runs`
  returns the newest N oldest-first, `interrupt_open_room_agent_runs` fails
  every non-terminal run at startup.
- `daemon_owner` — singleton team-platform P2 owner identity
  `{participant_id, display_name}`. `owner_identity(default)` mints it once;
  the id is derived from the first display name (`owner_participant_id`,
  canonical mention alphabet) and never changes. `set_owner_display_name`
  renames the owner row and the owner's Human roster rows in one transaction.
  Display data only — never an authentication principal.
- P2-A federation tables: `federation_instance` (singleton instance id),
  `room_federation` (bearer credential — PRIVATE), `room_member_bindings`
  (member→agent binding, `registration_key` PRIVATE, agent name unique per
  room), `producer_counters` (next source_sequence per room+member),
  `federated_events` (dedup + monotonic order index),
  `processed_room_triggers` (at-most-once trigger-claim journal), and
  `pending_redemptions` (v1.2 amendment table 7: pre-room `{redemption_id,
  bearer, invite_code}` custody, `invite_code` UNIQUE — bearer AND invite
  code are PRIVATE).

### Load-bearing invariants

- **u64 as canonical decimal TEXT.** Counters, cursors, and sequences are
  stored via `write_u64_text` and re-read only through
  `parse_canonical_u64_text`; noncanonical text fails closed. Never compare
  or `MAX()` these columns in SQL — lexicographic order is not numeric order.
- **Atomicity.** Every multi-row federation mutation runs in one IMMEDIATE
  transaction: `allocate_outbox_pending` (counter advance + outbox insert)
  and `ingest_confirmed_event` (dedup check, monotonic check, transcript
  append, dedup-index insert, full-tuple outbox removal, cursor advance,
  trigger claims) commit all-or-nothing.
- **Outbox removal requires the full producer tuple.** A confirmed event
  deletes an outbox row only when `client_event_id`, `source_id`, and
  `source_sequence` all match — never `client_event_id` alone.
- **Confirmed ingest is fail-closed.** Dedup cross-checks BOTH persisted
  copies: the `federated_events` index tuple must equal the parsed transcript
  `FederatedMessageMeta`, and that meta must equal the incoming event — every
  field, plus exact persisted payload and normalized mention-set equality,
  never raw JSON bytes or a column subset. Full equality yields
  `IngestOutcome::Duplicate`; an authenticated exact replay may fill legacy
  unknown mention metadata without replaying a turn or advancing the cursor.
  Any divergence (including index vs
  transcript), a missing/unreadable indexed transcript row, a
  `global_sequence` at or below the ordering baseline, or a missing access
  row ⇒ error and full rollback. The ordering baseline is
  max(last indexed sequence, persisted `room_access` cursor), so a
  bootstrap/recovery cursor set ahead of the local index rejects stale
  sequences and the cursor can never regress. Sequence gaps are accepted.
- **Trigger claims are at-most-once per (room, ledger event, target).**
  Claims commit inside the ingest transaction, only for locally-bound
  targets, and never for agent-authored rows.
- **Producer counters never reuse a sequence.** Allocation is transactional
  across connections; exhaustion at `u64::MAX` fails closed.
- **Credential custody.** `bearer_token`, `registration_key`, and pending
  redemption secrets (bearer + invite code) are never serialized into
  projections, transcripts, logs, or error messages. `RoomCredential` and
  `PendingRedemption` have redacting `Debug` and no `Serialize`. `open()`
  enforces owner-only (0600) mode on the DB and its sidecar files BEFORE any
  DB work and again after create/migration (Unix); filesystem errors fail
  closed except `NotFound`.
- **Pending redemptions never fork.** `get_or_insert_pending_redemption` is
  an atomic get-or-insert keyed by `invite_code`: an existing code returns
  the STORED triple and discards caller-supplied values. Promote takes exact
  inputs `(redemption_id, room, bearer, local_human_member_id)` and is
  all-or-nothing: install credential + delete pending in one transaction;
  exact replay after response loss is an idempotent no-op; every other state
  is corruption with no partial write.
- **Bindings are write-once per member.** A retried registration with the
  identical `(room, member, agent, key)` tuple is an idempotent no-op; the
  same member with a different agent or key fails closed — rebinding
  requires an explicit unbind. Registration-key derivation is frozen for
  P2-C (freeze v1.2 §3); this crate stores the column opaquely.
- **Local agent ownership follows current roster kinds.** Retained
  `room_agent_owners` bindings apply only to a current Agent; `owner_present`
  requires a current Human. Reusing a departed id for another kind must not
  inherit attribution or project a live owner. Artifact `on_behalf_of` remains
  a creation-time snapshot and historical bindings remain intact.
- **`update_room_access_safe` is the runtime refresh path**: it never touches
  the outbox and its cursor only advances. `replace_room_access` is
  destructive test seeding only.
- **Caller projection follows credential custody.** Non-Local `room_access`
  reads `caller_member_id` and P2 speaker field `local_member_id` from one
  `room_federation.local_human_member_id` snapshot alongside access state; no
  credential means both are absent, and Local always means absent. Projection
  replacement never installs caller identity or credentials.
- **Mirrored cursor writes are compare-and-swap.** `set_room_read_cursor_mirror`
  evaluates the expected prior mirror and write under one IMMEDIATE transaction.
  `Applied` returns the durable projection; `Stale` never mutates the row. Callers
  must handle newer concurrent `Some` values monotonically while reserving a
  clear for an expectation that still matches.


### Production Rooms persistence invariants

- **Migration boundary.** These APIs and original fixtures are the first
  persistence stage toward running source
  `0abb558179af3ff848d3bd19ac67aff246299ce1`. The daemon's admission, profile,
  resource, attachment and retirement interfaces remain a dependent stage.
  This foundation is not complete parity or a deployable runtime replacement.
- **Durability.** File stores verify WAL, explicitly choose synchronous NORMAL,
  set a five-second busy timeout and foreign keys before migration, and preserve
  owner-only DB/sidecar modes. NORMAL preserves atomicity but may lose the latest
  committed transaction on power loss; it does not promise FULL durability.
- **Closed-room guard.** Content and authority writes, including exact decision
  retries, check openness inside their transaction. Secret revocation and
  content-free late resource audit remain available; cuts apply only to closed
  rooms and preserve room/access/authority history.
- **Admission generations.** Authorization/status mutation, immutable decision
  and audit commit together. Exact `(room, agent, generation, session)` checks
  gate authorized history, reply/failure writes and outbox allocation in the same
  transaction. A revoked binding is terminal; stale or absent bindings refuse.
- **Decision namespace.** Agent, profile, resource and retirement decisions
  share one room-wide namespace. Exact content replay is idempotent; conflicting
  replay fails closed. Agent authorization and status writes check all four
  ledgers under their write transaction before mutation, preserving exact Agent
  ledger replay. Generation integers migrate to canonical decimal TEXT.
- **Local bootstrap.** Establishing a Local owner, package-derived Agent and
  attribution is one idempotent transaction. It creates no execution binding or
  decision. Owner eligibility requires a current Human.
- **Attachment ordering.** Immutable metadata and its System marker commit
  together; uploader/remover is roster-checked in that transaction. Daemon bytes
  must be fsynced before metadata commit and unlinked only after deletion/cut
  commits. Stored ids never authorize paths on their own.
- **Bounded history.** Forward and newest-tail paging are distinct, including
  separate closed-room audit APIs. `RoomRecord.transcript_has_more` carries the
  limit-plus-one sentinel; a full page is not proof of another page. Mutable
  room-list ordering cursors encode the captured timestamp and id boundary.
- **Profiles and grants.** Profiles contain references only. One live grant per
  canonical root is enforced by a partial unique index; effective expiry is
  revoked, revocation is terminal, and grant generations/replay are exact.
  Status changes and expired-root retirement parse canonical u64 TEXT and
  increment in Rust with checked arithmetic, including values above SQLite's
  signed range. Exhaustion rolls back the decision, grant/status, audit and
  room touch; exact replay and valid no-op status decisions require no increment.
  Resource audit stores path digests rather than path text.
- **Retirement.** Human-to-Human merge moves the Local owner and current Agent
  attribution, removes the placeholder, and records alias + decision + minimal
  audit atomically. HTTP eligibility remains daemon policy; alias chains are
  bounded to eight links. A chain ending at that exact boundary resolves;
  a further link or any repeated id (including a self-cycle) fails closed
  without returning partial authority.
- A retired alias `from_id` permanently reserves that room-scoped participant
  identity. Every roster-creating path, including ordinary, owned-Agent, and
  bootstrap joins, checks the alias ledger inside its IMMEDIATE transaction;
  a racing join is either removed by the later retirement or refused after it.
  Alias list reads return the oldest 256 ordered rows plus `has_more` for
  bounded daemon projections; the underlying durable ledger remains complete.
- `inspect_room_identity` returns only room id, name, and closed state without
  hydrating transcript rows; daemon inspect responses combine that bounded
  metadata with the bounded public alias list.
- **Policy and markers.** The hand-written trigger codec preserves both build
  and CI flags. Omitting a policy mutation leaves persisted flags intact. Store
  marker names use core `bounded_prose`; structured audit payloads remain raw in
  storage and require daemon read-boundary projection before display or prompts.

## Work Guidance

- Add new durable state to this crate; do not let the daemon or a network
  client own SQL against `rooms.db`.
- New u64-valued columns must use the canonical-decimal TEXT helpers and get
  reopen + fail-closed tests.
- Keep new error variants coordinated with
  `ocean-daemon/src/persistent_rooms.rs::room_store_error_response`
  (exhaustive match).

## Verification

- `cargo test -p ocean-store`
- `cargo clippy -p ocean-store --all-targets -- -D warnings`
- Workspace impact: `cargo check --workspace` (daemon matches
  `RoomStoreError` exhaustively).

## Child devlog Index

- (none)
