# Generic Plugin Process Ownership and Quiescence Manifest

**Status: PROPOSED — design review required.** This document defines a candidate
generic subprocess lifecycle contract for issue88. It records a design for
review; it does not authorize source changes, daemon installation, plugin-state
adoption, or a claim that any implementation is complete. Issue88, issue85 and
PR94 are ocean-private tracker numbers (`KINGMAKER-SYSTEMS/ocean-private`
#88, #85 and #94); this monorepo has no items of its own under those numbers.

## Problem and current evidence

Issue88 requires generic ownership, launch fencing, and observed quiescence for
every supported subprocess-plugin path. The owner must retain responsibility
through cancellation, shutdown, timeout, restart, and failure. EOF, a closed
pipe, `start_kill`, a dropped future, a request timeout, a daemon label, or a
PID read by itself is not proof that a child or descendant has exited.

On the frozen private main `81eb9fff5e438858e8ada8b7651c8e5609c838b4`, the
content this monorepo was published from,
`ocean-agent` starts global plugins during capability-registry construction
(`crates/ocean-agent/src/lib.rs::discover_plugin_providers`) and starts
folder-agent subprocess capabilities per turn in
`build_agent_capability_providers`. The per-turn launch occurs before
tool-name de-duplication. Both paths call `SubprocessPlugin` directly.

`SubprocessPlugin` keeps request channels and pending response senders, while
`spawn_io_task` drops the transport into a detached Tokio task. A request timeout
removes its waiter without terminating or joining that task. The I/O loop may
await a transport write inside `select!`; closing the transport calls
`start_kill`, ignores its result, and does not wait for the child. `kill_on_drop`
covers the direct child drop behavior, not an observed wait or descendant
quiescence. The subagent installer copies plugin files over their destinations
without fencing already-loaded interpreters. These behaviors are not an issue88
quiescence rail.

Relevant anchors in the reviewed snapshot:

- Global discovery and launch: `crates/ocean-agent/src/lib.rs`,
  `discover_plugin_providers`.
- Per-turn launch and later de-duplication: `crates/ocean-agent/src/lib.rs`,
  `build_agent_capability_providers` and its caller.
- Child/task state, request timeout, detached I/O loop:
  `crates/ocean-plugin/src/subprocess.rs`.
- Child process and best-effort close:
  `crates/ocean-plugin/src/transport.rs`.
- Copy-in-place plugin install:
  `plugins/ocean-subagents/install.sh`.
- Existing native extension supervision covers registered `ServiceActivation`
  processes only. It does not currently enroll ordinary `SubprocessPlugin`
  launches or prove old plugin interpreters are gone.

Recheck these symbols against the implementation head before source work. Line
numbers are intentionally omitted because this is a proposal, not a patch recipe.

## Ownership boundary

The daemon remains the authority for whether a plugin capability is exposed and
whether each tool call is permitted. A generic lifecycle owner controls process
creation, transport/task lifetime, cancellation, shutdown, exit observation,
and launch fencing. It does not decide which agents to create, how they are
scheduled, how many subagents are admitted, which credentials they receive, or
how durable subagent jobs and request identifiers are stored. Those policies
and durable state remain extension-owned.

The process owner must outlive any one capability-registry build, session turn,
request waiter, or caller future. The operating-system adapter must also define
how that owner remains available across daemon restart and what happens if the
owner itself is lost. A process-local task or `JoinHandle` alone cannot meet the
abrupt-daemon-death requirement. The selected platform mechanism must be
reviewed separately for macOS, Linux, and Windows. If a platform cannot prove
the required child and descendant state, it returns an unknown/quarantined
result and blocks the dependent adoption gate; it must not report success.

## Proposed interface and invariants

Names below describe responsibilities, not accepted public Rust API names.
Implementation may refine types under a separately reviewed source change.

### Launch and ownership

1. Every production subprocess launch enters one `PluginLifecycleCoordinator`
   before `Command::spawn`. There is no production constructor that can bypass
   enrollment. Test-only in-process `Transport` fixtures remain isolated from
   production launch authority.
2. The coordinator issues an opaque launch lease bound to a daemon generation,
   plugin identity, launch origin, and an unpredictable launch identity before
   spawn. It commits the OS process identity and owner record before the child
   can receive a protocol request. A failed or partial spawn is still settled by
   the coordinator.
3. The retained owner record contains the OS process handle needed to wait,
   task and transport owners, pipe state, lifecycle state, and the exact launch
   lease. Numeric PID is diagnostic only; PID reuse must not satisfy identity
   comparison.
4. Registry discovery, per-turn named capabilities, other direct plugin
   constructors, and queued launch requests share the same registry and launch
   fence. A provider later discarded by tool-name de-duplication is terminated
   and observed through this same owner. It is not an unregistered child.
5. The owner—not a request future—retains the I/O task and its `JoinHandle`.
   Caller/request cancellation ends that caller's wait only; it does not stop a
   shared plugin instance. Explicit instance, host, or generation shutdown asks
   the owner to stop the exact launch and awaits a bounded receipt. Dropping a
   request, response waiter, provider or runtime handle cannot silently detach
   child cleanup.

### Fencing and shutdown

The coordinator exposes an exclusive quiescence operation. Its cutover first
atomically closes the launch gate, invocation-admission gate, and every enrolled
transport's physical-write gate for the covered generation. It rejects or
queues new launches and invocations under a fixed bounded policy, fences queued
invocations that have not begun a physical write, and prevents previously
authorized but not yet started writes from starting. No launch or physical
write can begin between this cutover and the later inventory. Only after those
gates are closed does the coordinator snapshot the enrolled owners. Writes that
physically began before cutover are tracked as in-flight; the owner must settle
or terminate them and account for any unknown effect before reporting
quiescence. The owner proves no transport can issue another write. A request
accepted by the plugin whose reply is lost remains an unknown effect; it is
never replayed. The snapshot states its generation and participant set; it
cannot imply coverage of paths or processes that were not enrolled.

Shutdown is a monotonic state machine:

`Starting -> Running -> Stopping -> ExitedAndReaped`

Any uncertain start, write, cancellation, timeout, signal, wait, descendant
check, or owner loss may instead transition to `Unknown` or `Quarantined`.
Neither state can be converted to `ExitedAndReaped` by elapsed time, pipe EOF,
request status, or a later PID lookup. Recovery requires fresh evidence against
the retained launch identity.

For a known live owner, the adapter negotiates graceful shutdown only if the
plugin protocol explicitly supports it, closes stdin, and waits for a bounded
grace period. It then uses the platform's reviewed forced-termination mechanism and
retains the OS wait handle until the direct child has been observed and reaped.
It separately proves the owned descendant set is empty. Every stage reports a
typed result; errors preserve the owner record and prohibit reuse or a false
quiescence receipt.

A write already delivered to a plugin may have side effects even when its reply
is lost. The host records that result as unknown and never automatically replays
the tool request while recovering process ownership. This manifest does not
introduce an idempotency key or claim exactly-once execution.

### Quiescence receipt

The owner may issue a sanitized `QuiescenceReceipt` only after it has fenced
launches and observed every covered member reach a terminal state. The receipt
binds:

- coordinator and daemon generation;
- the exact enrolled participant/launch identities covered by the fence;
- each direct child's observed exit status and wait/reap result;
- completion of owner tasks and pipe cleanup;
- the platform-specific evidence that each owned descendant set is empty;
- any unknown or quarantined members, which force the overall result to
  `incomplete`.

The receipt excludes environment values, arguments containing secrets, plugin
payloads, tool inputs, and provider output. It is evidence for a named covered
set at a point in time; it is not a permanent assertion that new launches can
never occur. A later operation must use the same coordinator's fence or a newer
generation.

### Restart and independent supervision

The daemon may reconnect only to the same surviving owner generation and exact
launch identities. If the owner or its process handles cannot be recovered, the
operation remains unknown; rebuilding a cache or adopting a new state file does
not imply that former children exited. The host must keep launch fenced for the
affected participant set until it obtains a verified recovery receipt or an
operator-approved containment decision that does not claim quiescence.

The implementation manifest must choose and validate an owner that can survive
the daemon failure modes required by issue88. This proposal does not assume that
Tokio `kill_on_drop`, daemon shutdown hooks, a detached Tokio task, a Unix
process group, Windows job assignment, an environment flag, or a filesystem lock
alone provides that guarantee. Each OS adapter must document the exact evidence
it uses after owner restart and abrupt owner death. Unsupported or ambiguous
mechanisms fail closed.

## Legacy launch retirement and adoption

No new owner can retroactively enroll a process started by old code. Migration
therefore has two distinct epochs:

1. **Legacy retirement:** stop accepting new legacy launches at every known
   entry point; discover and account for already-running and untracked legacy
   plugin processes, queued launches and copied installer artifacts; terminate
   and observe them with platform-specific evidence. A bare daemon exit, absent
   plugin label, EOF, lock acquisition, sleep, PID list or operator assertion is
   insufficient. Missing evidence leaves the host quarantined.
2. **Owned generation:** start the new coordinator behind the closed launch
   fence, recover only exact durable owner records, reconcile the covered
   participant set, then open launches for the new generation. Old and new
   writers must never overlap against shared lifecycle state.

Install, update, uninstall and rollback must use the same fence and owner. A
rollback selects the previous compatible artifact under the current coordinator
and proves it loaded; it must not re-enable an untracked legacy writer or
resurrect a second state owner. If safe rollback cannot be demonstrated, leave
launches disabled and retain the recovery record for an explicit operator
decision.

This is a design dependency for issue85. Issue85's full custody/schema
implementation and adoption stay held until the complete issue88 launch,
termination, descendant, restart and legacy-retirement acceptance passes.

## Proposed implementation sequence

These stages order the complete issue88 work; finishing an earlier stage does
not close issue88 or authorize issue85.

| Stage | Outcome | Required evidence before the next stage |
| --- | --- | --- |
| D0 — reviewed contract | Independently review this manifest and reconcile its interfaces with the extension architecture and issue85 custody proposal. | Exact reviewed document; settled ownership, failure and adoption semantics. |
| D1 — launch enrollment | Implement the coordinator/owner seam and route every global, per-turn, named/direct and dedup-discarded plugin process through it. | Source audit shows no bypass; real-spawn tests prove closed-gate and startup-failure behavior. |
| D2 — retained lifecycle | Implement cancellation-safe transport/task ownership, timeout behavior, graceful/forced termination, wait/reap and descendant results. | Real child/descendant tests prove held I/O, delivered request, EOF, timeout, cancellation, shutdown, failure cleanup and no orphan owner. |
| D3 — restart recovery | Implement independently supervised ownership and exact-generation reattachment or fail-closed quarantine. | Kill/restart tests prove owner interruption, same-identity recovery, unknown-effect non-replay, and one terminal receipt. |
| D4 — legacy migration | Implement launch fencing, old/untracked writer retirement, clean adoption and rollback. | Cross-platform real census/recovery evidence proves no old writer or duplicate owner before issue85 state adoption. |

Each stage requires a fresh exact-source review and the owning project checks.
Cross-stage work remains one ordered dependency chain unless a reviewer proves
the stages independent. None of these stages authorizes a fleet scheduler,
provider credentials, permission bypass, Room trust changes, or production
installation.

## Acceptance matrix

| Issue88 requirement | Required proof |
| --- | --- |
| Global discovery, per-turn/named/direct launch, startup failure, tool-list timeout, and de-dup-discarded providers are all enrolled. | Source call-site inventory plus real-spawn fixtures for each path; no constructor bypass. |
| Child identity, task, transport and cleanup remain owned through last-handle drop, delivered/queued RPC, cancellation, daemon stop and errors. | Receipts bind exact process identity and actual wait/reap; tests retain owners after timeout, future drop and failed writes. |
| Launches and writes are fenced before quiescence inventory and remain closed until recovery/rollback resolves. | Contended launch/write/quiescence tests prove no launch or write can begin after cutover, pre-cutover in-flight writes settle before receipt, and participant coverage is exact. |
| EOF, `start_kill`, timeout, daemon death, PID absence and sleep cannot fabricate exited evidence. | Negative fixtures leave owners unknown/quarantined until real platform evidence arrives. |
| Descendants and abrupt host failure are covered on every supported platform. | Real platform fixtures prove descendant absence after forced stop and after owner death; unsupported cases fail closed. |
| Legacy processes, untracked launchers and queued paths are retired before adoption. | Upgrade fixture starts old versions, holds each legacy launch path, then proves a complete census and prevents old/new overlap through rollback. |
| Unknown tool effects are not replayed and extension state is not discarded after a known accepted response. | Delivered-request/lost-reply and publication-failure fixtures retain the uncertainty obligation through restart. |
| Existing plugin ABI, permission gates, unrelated plugins and users remain compatible. | Existing ABI/permission tests plus multi-plugin and independent-root regression tests. |
| No core named-subagent, task/fleet scheduler or provider authority is introduced. | Exact source/API diff review and manifest/permission audit. |
| Production release is protected and verifiable. | Independent exact-head review, both required hosted builds, clean merged-main provenance, installer compatibility and live revision/health evidence. |

Tests must use actual subprocesses and inspect handles, process state and wait
results. Mocks may verify parsing and state transitions, but cannot certify
quiescence. Tests must cover held stdin/stdout, stdout EOF while a process remains
alive, queued and delivered RPCs, caller cancellation while another caller uses
the same instance, duplicate providers, launch/invocation-fence contention,
startup failure, cancel/timeout, graceful and forced shutdown, abrupt owner
death, descendants, restart, stale identities, independent state roots and
rollback. No sleep-only, lock-only, mocked-kill or synthetic receipt is
acceptance evidence.

## Rollout and delivery gate

This proposal may be independently reviewed and published as documentation.
Source work begins only after the exact lifecycle/interface and migration design
is accepted under issue88 and a separate implementation change is reviewed.
Issue85 source/schema/adoption remains blocked until all issue88 acceptance is
proven. Every code PR requires the current protected `Build Ocean` and
`Build Surface` checks. The current hosted-runner billing/spending denial is a
separate delivery blocker; local tests or a reviewed manifest do not replace
those required checks. Production installation requires clean merged-main
content, the existing quiet-intake boundary, rollback safety, and exact running
revision/readiness evidence.

## Explicit non-claims

- This is not implemented behavior or operator ratification.
- This does not mean a close request, EOF, PID lookup, daemon exit, or parent
  process wait proves descendants are gone.
- This does not solve provider-side exactly-once effects or create a scheduler.
- This does not close issue88, issue85, or PR94, or authorize plugin-state
  migration/adoption.
- The bounded pre-body `RefusedBytes` V3 is a separate design-only transport
  proposal; it does not satisfy lifecycle, restart or legacy acceptance.
