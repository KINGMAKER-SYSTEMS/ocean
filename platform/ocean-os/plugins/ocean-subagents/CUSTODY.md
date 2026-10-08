# Subagent state custody — proposal

Status: **PROPOSED; implementation and adoption are not authorized**. This is an
implementation and acceptance manifest for giving the subagent plugin's shared
state one owner and for recovering dispatches whose outcome is unknown. It
describes the plugin as it is on `main` and implements nothing. Source work
needs an accepted review of this manifest. It also depends on a host-side
prerequisite (generic plugin quiescence and retirement of the old launch path,
below) that is outside this plugin and not yet built. Reviewing or publishing
this draft does not discharge it.

## Current facts and retained contracts

`JsonStore` loads `runs.json` once at startup and every later write replaces
the whole file with that process's cached map. The atomic rename, the store
lock and the per-request write fence (`JsonStore.update(run_id,
expect_request=...)`) all work inside one process; none of them excludes
another process. So with two plugin processes on one state root, A and B both
load run R1; A accepts and records R2; B's next write (a refresh of R1, say)
replaces the file with its cache, and R2 is gone from state while the daemon
still runs it. The daemon can host more than one plugin process (the global
process plus named-agent ones), so this is reachable. The first acceptance row
below requires reproducing it over real stdio before any fix.

Keep everything the plugin guarantees today:

- The eight tool schemas and the newline JSON-RPC ABI, the fixed
  non-recursive worker profile and its fail-closed resolution check, ordinary
  daemon-owned sessions, normal tool permissions and private per-turn decision
  tokens bound to the exact run/request/session/tool tuple.
- Strict cancellation: `cancelling` only on `ok` exactly `true`, the same
  request id and `state: cancelling`; `ok:false` settles from daemon truth; any
  other answer is an error that writes nothing and carries no response body.
- Request-fenced lifecycle writes: every write after daemon I/O names the
  request it observed, so a late observation, acknowledgement or watchdog
  error for a replaced turn writes nothing. A watchdog cancels only the request
  it was armed for, and never asks again for a run already `cancelling`.
- Admission: `spawn` and `send` share one lock from the capacity check through
  publishing the accepted turn; at most four active runs; queued, running,
  waiting-for-permission and cancelling hold slots; a failed or malformed
  observation keeps its slot. A refused admission starts no turn and leaves a
  finished target unchanged. An accepted follow-up replaces its predecessor
  only by publication, with a fresh request and token and preserved turn
  history.
- Loss settlement: the daemon's request registry is volatile, so an exact
  known request that is absent settles as `lost` from session truth after a
  short grace (immediately on an explicit cancel refusal), keeping what the
  child wrote in that turn. This releases its slot; it is the only way a run
  survives a daemon restart without holding a slot forever.
- Retention: the newest 200 finished runs are kept and older finished runs are
  pruned; active runs are never pruned. A pruned run's child session stays in
  the daemon.
- `--check` reads state only and never starts watchdogs or calls the daemon.

No provider, Room trust, credential, core fleet or scheduler authority is added.

## One lifetime owner

Choose exclusive process-lifetime ownership rather than short locks around
cached writes. At most one cooperating plugin process may load, refresh, decide,
dispatch, cancel or write for a canonical per-user state root. Other processes
refuse startup promptly; they do not proxy work to the owner or start another
worker. This trades simultaneous plugin availability for a single
authoritative writer.

Resolve the configured root to an absolute canonical directory, including
symlink aliases, before loading the snapshot. Pin an open directory descriptor
and its device/inode identity. Use directory-relative opens and replacements;
before each mutation revalidate that the configured/canonical directory and
lock name still identify the pinned objects. Root replacement, a removed or
replaced lock, or an identity mismatch stops admissions and writes without
choosing another root. Canonical and symlink aliases of one directory share
custody; distinct directories stay independent. Nothing deletes or renames a
root automatically.

Use standard-library `fcntl.flock(LOCK_EX | LOCK_NB)` on a permanent
`.custody.lock` in that directory. Open it with no-follow and close-on-exec
flags; require a regular, singly linked, current-user-owned private file in a
current-user-owned private directory. The lock file holds no stale-owner lease
or PID expiry; never unlink, rename, truncate or recreate it to break
contention. One nonblocking attempt bounds contention handling: failure emits
one controlled startup diagnostic and a nonzero exit, with zero daemon calls,
snapshot reads, admission writes or watchdog starts. Filesystem setup and open
errors also fail closed; there is no unlocked fallback store.

Initial support is same-host Unix roots with proven kernel locking and atomic
replace plus directory fsync. Unsupported platforms or filesystem primitives,
network or shared-host roots and malformed artifacts refuse before daemon I/O.
Platform and filesystem qualification is an implementation gate, not an
assumption from `os.name`; no Windows or cross-host custody claim follows.

Acquire custody before `JsonStore._load`, manager creation or watchdog start.
Keep the descriptor non-inheritable and alive through the last possible mutator
and actual process exit; do not release it on a tool-call timeout, or when
`serve` returns while watchdogs remain. PID reuse, elapsed time, caller
cancellation, parent disappearance and transport timeout never transfer
ownership.

## Shutdown and succession

Replace the sleeping watchdog threads with cancellable waits and keep their
handles. EOF marks the owner closing, refuses new work and drains owned
observation, HTTP and publication tasks under bounded I/O deadlines. EOF is not
seen during a synchronous held call until that call returns, and cancelling the
host's waiter neither cancels nor completes it. An unproven drain keeps the
owner and the lock and must not announce relinquishment. Kernel release on
actual process death lets a successor in, and the successor reloads disk before
anything else. No inheriting descendant may keep or bypass custody.

`--check` acquires and validates custody and state with no watchdogs and no
daemon I/O. Serving startup first reconciles journals, then re-arms watchdogs
only for exact known active request ids from durable state; it never sends
replacement turns. For an exact known request, daemon truth keeps today's
meaning: a terminal state settles it, an absent request settles as `lost` from
session truth after the grace, and an unavailable or malformed read changes
nothing and keeps its slot. A dispatch whose request id was never learned has
no exact request to observe; it stays an occupied, unresolved hold (below) and
is never settled by absence.

## Durable state and publication

Propose schema 2 in `runs.json`: keep every existing run field and add an
`operations` map. Each dispatch operation stores unique operation and attempt
UUIDs, an owner-generation UUID, its kind, the reserved run id, the
predecessor request id for a send, the exact task/role/cwd/model/thinking
level/session/timeout inputs, the private decision token, a payload digest,
timestamps and a phase. These UUIDs express local operation ownership; they are
not daemon request ids or a way to correlate with the daemon. Phases are
`prepared`, `attempting`, `accepted_unpublished`, `published`,
`registered_refusal_pending`, `registered_refusal_terminal`, `rejected` and
`unresolved`; every unfinished phase holds a slot.

A completed send predecessor stays unchanged until accepted publication. A run
has at most one outstanding operation. Each nonterminal known request or
unresolved dispatch counts once, including a send reservation on a completed
run; an accepted operation linked to its published run is not counted twice.
A capacity refusal creates no operation, changes no completed target and makes
zero turn POSTs. Intents and tokens stay mode 0600 and private; tool errors
expose only controlled reasons and non-secret operation ids.

### Storage budgets and reserved publication space

These are proposed schema-2 limits; schema 1 enforces only the field limits
noted. `KiB` is 1,024 bytes and `MiB` 1,048,576 bytes. Size records with one
canonical compact JSON encoding (UTF-8, `ensure_ascii=False`, sorted keys, one
trailing newline); escaped control characters count at their encoded size.
Both decoded-field and encoded-record limits apply, and no field or history is
silently shortened to fit. Refuse duplicate keys, non-finite numbers, invalid
UTF-8, nesting deeper than 16 or more than 200,000 structural JSON values in a
snapshot before materializing it.

| Material | Hard byte/count budget |
| --- | --- |
| Caller inputs | One JSON-RPC line and one encoded dispatch body: 512 KiB each. Keep today's limits: task/message 64,000 UTF-8 bytes, role and model 200 each, cwd 4,096. Private token at most 96 ASCII bytes, UUIDs exactly the canonical 36 bytes, timestamps at most 64 bytes. |
| Lifecycle text | Output at most 24,019 UTF-8 bytes (today's 24,000-byte cap plus the truncation marker); error at most 2,000 characters, so at most 8,000 UTF-8 bytes, as today. These maxima are included in lifecycle headroom. |
| Runs and history | Today's retention bound of 200 finished runs, plus active runs and runs held by an unfinished operation; each encoded run at most 1 MiB. At most 256 turn-history entries per run and 16,384 overall, each at most 1 KiB. An accepted turn must fit before dispatch; a refused admission preserves every existing entry. |
| Operations | At most 512 retained operations, each at most 512 KiB including inputs, reservation and disposition fields. Finished and rejected operations count until pruned with their run (below). |
| Disposition receipts | At most 512 immutable receipts, each at most 8 KiB. Store bounded disposition codes and digests plus exact ids and token, never raw response bodies. Accepted-202 and registered-503 receipts share this budget. |
| Snapshot and legacy backup | `runs.json` and the single byte-exact `schema1.backup`: at most 8 MiB each. No rotating backups or numbered archives. |
| Control artifacts | `adoption.json` at most 64 KiB, referencing external quiescence evidence rather than embedding logs; `.custody.lock` at most 4 KiB. |
| Staging | At most one snapshot-or-backup staging file of 8 MiB, one receipt staging file of 8 KiB and one adoption staging file of 64 KiB. No accumulation of random temporary files. |
| Startup inventory and reads | Root: at most eight recognized entries; receipt directory: at most 512 final receipts plus one stage. Stop each iterator at limit + 1 without collecting an unbounded list. At most 32 MiB of root file content read at startup; each file is size-checked before reading and read with a limit + 1 check. |
| HTTP reads | Dispatch response at most 16 KiB; any other daemon response at most 2 MiB. An oversized response is controlled observation or dispatch uncertainty, never terminal proof. |
| Whole state root | At most 32 MiB of logical file bytes including live data, receipts, control artifacts, backup and every staging file. Allocated blocks and metadata overhead are separate; this is not a free-disk guarantee. |

The recognized root layout is `.custody.lock`, `runs.json`,
`runs.json.staging`, `schema1.backup`, `schema1.backup.staging`,
`adoption.json`, `adoption.json.staging` and `dispatch-receipts/`. Snapshot and
backup stages cannot coexist. A receipt stage is `<operation-id>.json.staging`
and there is at most one. Create stages exclusively; a surviving stage is
validated within bounds and reconciled before another write, never overwritten
or deleted merely because startup found it. A complete staged receipt may
recover only its exactly matching operation, attempt, token, digest and
disposition. An unproven stage, unexpected entries, links, counts or sizes fail
closed with files intact. Reads after restart enforce the same limits before
watchdogs or daemon I/O. Legacy data over any import budget stays untouched and
adoption waits for a separately reviewed lossless plan; there is no automatic
truncation, split root or archive escape.

Retention stays the only deletion. Schema 2 applies today's pruning to the
candidate snapshot before every headroom check: the oldest finished runs beyond
the retention bound go first, and when headroom is short, further finished runs
oldest first, each together with its finished operations and their receipts.
Pruning never removes an active run, a run or operation with an unfinished or
unresolved phase, a `registered_refusal_pending` operation or its receipt.
Rejected operations with no run are pruned oldest first under the same rule.
There is no TTL retirement, archive or environment quota override.

Reserve logical headroom durably in each operation before POST. Let `S` be the
encoded candidate snapshot after pruning. Every unfinished dispatch reserves an
additional `G = 512 KiB + 1 MiB + 4 KiB` for its operation growth, a possible
accepted run and structural/disposition overhead, plus one 8-KiB receipt and
the prospective run and history count increments. Each known active run
reserves `L = 256 KiB` for the largest escaped output, error and timestamp
growth. The reservation is conservative: bytes already present may be counted
again, and it never promises a smaller result. Require
`S + sum(G) + sum(L) <= 8 MiB` for the post-intent candidate, including the new
reservation, and enforce every individual record and count cap. Every current
or prospective active run must also fit its encoded base plus `L` within the
1-MiB run cap; the projected 1-MiB accepted-run reservation includes that
lifecycle allowance. Validate this on import and before POST, not after
learning accepted ids. A snapshot change cannot spend another operation's or
active run's headroom. Reserved counters are revalidated from operation
identities on every reload.

Also reserve the peak publication footprint before POST: two full 8-MiB
snapshots (live and replacement), one 8-MiB backup, one 8-KiB receipt for each
existing or reserved receipt, one 8-KiB receipt stage, two 64-KiB adoption
artifacts (live and stage) and one 4-KiB lock must fit the 32-MiB root cap.
With 512 receipt slots that peak is below 29 MiB; unknown or stale artifacts
are not excluded from accounting. Admission reserves a future receipt and
count slot even before the response is known. A send reserves one new history
entry without changing its completed predecessor; a spawn reserves one new run.

On accepted publication, keep the active run's `L` before releasing the
operation's `G` and prospective counts; its final receipt and retained
operation stay charged until pruned. A registered-503 operation keeps `G` until
an exact terminal observation is durably committed. A proven pre-registration
refusal releases only after its rejection is durably published. Unknown or
publication-failed attempts keep every reservation. Once terminal truth is
durable, release future-growth headroom; the record, token, receipt and turn
history stay until retention prunes them. The accepted-publication candidate
must satisfy the same equation with its new active `L` and every other
reservation; no reservation disappears before the replacing obligation is
durably recorded.

If any budget, count or headroom check still fails after pruning, admission
makes no POST and no completed-target mutation, so saturation refuses new work
rather than dropping obligations. Four active slots bound concurrency, not
retained history; retention bounds history. Existing lifecycle writes use their
reserved headroom; if one still exceeds a cap, or physical storage or fsync
fails, keep obligations and the old committed bytes, retain uncertain staged or
receipt evidence, stop admissions and follow the publication-failure rules
below. A logical reservation does not allocate or guarantee free disk, atomic
write success or eventual repair.

Every store mutation builds a separate candidate snapshot. Publish with
temporary-file fsync, atomic replace and directory fsync; only confirmed
publication replaces the in-memory committed snapshot. A failed directory fsync
after rename is uncertain publication, not permission to restore an older
cached map. Reload and reconcile under custody; never silently overwrite
uncertain disk truth. Keep the store lock short with no daemon I/O under it.
Admission custody spans capacity inspection, durable intent, one POST and
accepted publication; request-fenced lifecycle writes stay independent of it,
as today.

Before POST, fsync a complete operation intent and then its `attempting` phase.
Failure to prove those writes durable starts no POST. A successor holds any
surviving unfinished intent conservatively, including a crash just before the
POST; it cannot infer that nothing was sent. No intent expires by TTL or
watchdog. A caller's JSON-RPC id is not an idempotency key or replay authority.

After a valid accepted response, fsync a separate immutable disposition receipt
at `dispatch-receipts/<operation-id>.json` before publishing run and operation
changes in `runs.json`. The receipt binds operation and attempt, payload
digest, predecessor, the exact returned request and session ids and the private
token, with `kind: accepted_202`. The registered-503 path below uses the same
journal with a distinct kind. Validate every receipt/state relationship;
malformed, orphaned, conflicting or duplicate-id records fail closed without
replacing records. Once durable, the receipt permits idempotent **local
publication recovery**, never a second POST. Publish the accepted run, turn
history and operation disposition atomically, and arm its exact watchdog only
after durable publication. Keep receipts as custody evidence; unresolved or
publication-failed operations are never retired automatically.
Send publication matches the predecessor's request identity, not its observed
status, output, error or timestamps, so ordinary observations of the old
request cannot discard an accepted new turn. An unexpected identity conflict
keeps the known receipt and recovery obligation, stops admissions and reports
controlled uncertainty; it never returns an unrelated current run as if nothing
had been dispatched.

If run publication fails after a durable receipt, keep the occupied operation,
return a controlled publication error, stop new admissions and recover the same
ids from that receipt. If the receipt itself cannot become durable, keep the
known ids, token and recovery obligation in owner memory while the owner lives,
keep the durable pre-dispatch reservation occupied and refuse further
dispatches. Bounded persistence repair may write those same ids; it must not
redispatch or claim a durable acknowledgement. If the owner dies before any
write persists the ids, the successor keeps an explicit unresolved hold; ids
that were never written are not claimed to be preserved. A stdout or output
failure after publication never rolls back acceptance or changes the outcome.

## Exact transport classification

One operation has one POST attempt, with no redirects or automatic transport
retries. Use a bounded response size and a total deadline with retained I/O
ownership; expiry does not imply owner exit. Local validation before transport
proves "unsent" only while the implementation can prove no transport call
happened. `URLError`, connection reset and timeout exceptions and cancelled
waiters give no trustworthy transmitted-byte count and keep uncertainty.

Acceptance requires the daemon's HTTP 202 envelope: `ok` exactly `true`,
canonical UUID turn and session ids, `status: running`, the matching event-id
prefix and no error. A send must also match the requested session. Persist the
exact ids; never accept truthy values, contradictory ids or truncated data.
(Today `start_turn` accepts any truthy `ok` with string ids.)

Only complete, source-shaped pre-registration refusals release a reservation:
the daemon's 429 capacity refusal, 400 cwd/binding/project refusals and 409
session-busy refusal, each with `ok` exactly `false`, failed status and
recognized refusal fields. These branches mint UUIDs even though no request is
registered; never publish them as accepted request custody. The capacity and
early-cwd forms have an empty event prefix; the later binding, project and busy
forms carry the matching turn prefix. Validate those branch-specific shapes and
a send's session identity, and pin each recognized error form to its producing
branch in fixture tests; unrecognized 400 forms stay unresolved. A status code
alone or an arbitrary `ok: false` body is not enough. Release the reservation
only after the rejection is durably published; if that write fails it stays
occupied.

The daemon's 503 branch (lifecycle bookkeeping unavailable) happens **after
request registration**: it marks that request errored and starts no provider
work, but it is not an unsent response. Preserve its known request identity
only from the complete envelope (strict `false` and failed status, canonical
ids, matching prefix and session, recognized bookkeeping refusal); anything
else keeps an unresolved occupied hold. For that complete registered refusal,
fsync an immutable `kind: registered_refusal_503` receipt in the same journal,
binding operation and attempt, ids and token, digest, predecessor and a bounded
refusal code. Then publish those ids and the receipt reference in its private
operation as `registered_refusal_pending`: no accepted child run, no turn
history append, no change to a completed send predecessor or its permission
token. The operation stays occupied until an exact-id terminal observation and
`registered_refusal_terminal` are durably committed. A missing or unavailable
read keeps occupancy; the response's failed status alone does not stand in for
durable request lifecycle truth.

Startup validates a durable registered-refusal receipt and reapplies it only to
its matching private operation, then reconciles that exact request without a
POST. It never turns the receipt into accepted-202 publication, starts a child
watchdog or guesses another request from the send's session. If main
publication fails, the receipt keeps the known ids, token, disposition and
occupied reservation for a successor. If receipt persistence also fails, owner
memory keeps the same facts and obligation, the durable intent stays occupied,
admissions stop and no durable acknowledgement is reported. Death before any
write persists the ids leaves the successor an explicit unresolved hold.
Receipt-stage recovery, directory-fsync ambiguity and all-write failure follow
the same bounded rules as accepted-202, without altering the predecessor or
replaying the dispatch. Other HTTP errors, mismatched or malformed envelopes,
absent or truncated replies and post-send exceptions keep uncertainty. Never
copy arbitrary response bodies into JSON-RPC diagnostics.

The daemon's API mints request ids and exposes no caller admission key or
decision-token correlation in request status; searching by session, time or
prompt is not an exact seam. This manifest therefore chooses durable unresolved
**non-replay holds** for a lost acknowledgement, not guessed rediscovery.
Status, wait, restart, permissions and cancellation cannot manufacture ids for
an unknown operation. Known requests still refresh and cancel through the
existing exact permission and strict cancellation contracts. An unavailable
read never authorizes a new attempt. An unresolved hold has no automatic clear
or reset command; exact recovery or release needs a separately reviewed proof
seam, not a manual assertion.

## Migration prerequisite and rollback

The host today has no enforceable launch fence across all plugin producers, no
retained registry of child identities and exits, and no receipt that a child
actually exited. The generic plugin request timeout drops a waiter; transport
close can be delayed by held I/O, after which requests kill without waiting for
exit. Named-agent plugin processes can overlap the daemon-global process, and
the installer overwrites executables without a drain. None of these, nor an
empty activity snapshot, an environment adoption flag, acquiring the new lock
or an operator assertion, excludes an old schema-1 writer that already cached
state. **Extension-only automatic adoption is held.**

Before any implementation or adoption under this manifest, the host must gain
an independently reviewed and proven generic quiescence seam and retirement of
the old launch path, outside this plugin's source. It must stop every daemon
and named-agent producer from launching the old executable, keep real
identities and handles for every existing child, and prove that every old
mutator and pending I/O has exited. Missing or lost child records fail closed;
they are not reconstructed from an argv or PID guess. No core subagent or fleet
service and no new scheduler is required or authorized.

Under that fence: drain existing dispatches and prove no unresolved legacy
execution remains, using real generic execution and exit evidence. One empty
request list or a zero-turn preflight is not enough without fenced intake and
every writer gone. Keep a mode-0600 byte-exact schema-1 backup and digest;
validate and import every known run, request, token and turn history into
schema 2; publish and fsync a verified adoption record tied to the root
identity, executable revision and retained quiescence proof. Only then release
cooperating new producers. The record indexes evidence; reading a JSON
assertion is not proof of quiescence. Fresh or empty roots are no exception
while uncooperative legacy producers can address them. Schema-1 rows already
erased, and lost acknowledgements, cannot be reconstructed by migration;
preserve and report that gap, never invent ids or claim recovered history.

Migration failure keeps intake fenced and evidence intact. Never downgrade
schema 2 in place, delete reservations or receipts, or let a legacy binary load
an older backup while a new owner can write. Rollback uses a reviewed
custody-aware binary compatible with schema 2, under the same launch fence and
real exit proof. If none exists, keep the plugin disabled and preserve state
for forward repair. Returning to an unlocked schema-1 writer is not an
executable rollback. This proposal grants no migration authority and makes no
safe-deployment claim.

## Bounded implementation and acceptance

After an accepted review and the migration prerequisite, plugin source work is
bounded to `ocean-subagents.py`, `test_ocean_subagents.py`, `test_wire.py` and
this plugin's documentation. Standard library only. No tool schema or worker
profile, provider, Room, core protocol, installer or production change is part
of this plugin slice. Host lifecycle work needs its own ownership, contract,
review and proof before adoption.

| Required outcome | Adversarial acceptance proof |
| --- | --- |
| Actual lost-state defect | Against the unchanged plugin, reproduce the two-process erasure over real stdio with a loopback daemon: a seed process accepts R1 and exits; A and B load R1; A accepts and records R2; B refreshes R1 and erases R2 while the daemon still reports R2 running. |
| Exclusive acquisition | Two real processes race before snapshot load; exactly one owns, the loser exits within bounds with zero GET/POST, watchdog or state mutation. Repeat for canonical, relative or symlink alias and distinct-root cases. |
| Artifact and platform refusal | Bad owner, mode, type or link, replaced root or lock, malformed schema, intent or receipt, and unsupported primitives fail closed; a live lock is never unlinked or recreated and there is no fallback. |
| Lifetime succession | Hold an RPC, cancel the caller, expire the request timeout, send EOF, drop the host handle and restart the daemon; no contender acquires until the real mutators and child have exited. Abruptly kill only a disposable owned child; the successor reloads disk. |
| Existing lifecycle safety | Sequential owners preserve unrelated runs, exact accepted follow-ups, tokens and history, permission tuples, strict cancellation (acceptance, refusal settling from daemon truth, malformed answers), late-observer fences, `lost` settlement of exact known requests, and retention pruning that never touches active or obligated records; re-arm only exact known active watchdogs with zero replacement POSTs. |
| Shared admission | Final-slot spawn/send and same-run send races across real processes; active, waiting, cancelling, failed-observation and unresolved operations each hold one slot. A refusal changes no completed target and makes no POST. |
| Dispatch durability | Crash or fail before intent, after intent and before POST, during a held accepted POST, before and after the acknowledgement receipt, and at every run replace or fsync point. Assert one attempt, no invented ids, an unchanged predecessor and durable unresolved holds. |
| Acceptance publication failure | A valid acknowledgement plus failed main publication recovers exact ids and token from the receipt; failed receipt persistence keeps the owner-memory obligation and reservation. Death with no durable ids yields an unresolved hold, not recovery or replay. |
| Bounded storage | Test each byte and count cap at the limit and limit + 1, escaped-input expansion, final run, history, operation and receipt slots, peak accounting with every reservation, and pruning order. Sequential dispatches stay bounded through retention; with only unprunable records left, admission refuses with zero POST and no predecessor change. Oversized schema-1 import, excess startup entries or read bytes, stale stages and restart stay bounded and preserve original bytes. |
| Quota and write failure | Fill the logical quota, and separately inject physical ENOSPC, replace or directory-fsync failure before POST, after intent, after each receipt kind and during run or lifecycle publication. Reserved active, unresolved and known-id obligations, tokens and full history survive; no quota reclamation beyond retention, no implicit retry and no promised free disk. |
| Registered refusal recovery | A complete registered-503 with receipt, main, stage or fsync failures and owner death recovers only the exact private operation ids, token and disposition, or an unresolved hold if never persisted. Exact terminal, missing and error reads control occupancy; no accepted run, history append, predecessor change, child watchdog or replay. |
| Transport truth | Exercise the exact 202, pre-registration refusals and post-registration 503; wrong session, ids or prefix, truthy `ok`, HTTP proxies and redirects, malformed or truncated bodies, timeout or reset, and stdout failure. Assert controlled errors, occupied uncertainty and zero implicit retry. |
| Migration and rollback | Enforce the producer launch fence and real identity and exit evidence with no legacy mutators before import; simulate a cached legacy writer, held RPC, lost handle, migration crash and unsafe downgrade. Any missing proof keeps adoption fenced. Backup bytes, tokens, history and receipt holds survive; rollback cannot admit two writers. |
| Release | Owning unit tests, real stdio, syntax and installer parsing, `cargo xtask docs-check`, independent review of the exact change, then the required Build Ocean and Build Surface checks on a reconciled head. No merge bypass or live claim; installation needs clean merged-main provenance and proof of the running outcome. |

Every fixture keeps the real `HOME`, uses absolute disposable auth, config and
state paths and synthetic loopback daemons, creates no real auth files and
keeps its proof. Every owned process, server and thread handle must be finished
at closeout. The real migration and deployment seam, historical loss and exact
recovery of unknown accepted requests remain explicit acceptance gaps. A lock,
a schema or an uncertainty reservation does not by itself establish global
restart-safe fan-out or exactly-once execution.
