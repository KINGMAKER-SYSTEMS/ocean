# ocean-daemon — HTTP Daemon

## Purpose

This crate owns the long-running Ocean HTTP service on `:4780`, including API routes, SSE event streaming, daemon runtime authority, and client-facing turn orchestration.

## Ownership

- **Scope:** `crates/ocean-daemon/`
- **Parent contracts:** `../../AGENTS.md` and `../AGENTS.md`
- **Primary responsibilities:** daemon API, health endpoints, turn/event routes, runtime wiring, permission boundary enforcement

## Local Contracts

- `provider_auth.rs` owns operator-only provider login/status/cancel/logout.
  Serialize start, cancel, and logout under an independent per-provider
  operation lease through cancel/bind/register or removal. Retain each attempt's
  OAuth publication fence even when an HTTP future drops; revoke and settle it
  before replacement/logout. Blocking removal retains its operation lease.
  Status runs custody/native reads off Tokio workers and projects the shared
  runtime resolver's token-free origin separately from stored OAuth facts.
  Login failure logs and responses contain only fixed classifications/provider
  identifiers; never log callback descriptions or arbitrary error chains.

- Daemon health is `GET /health`, not `/v1/health`.
- `/v1/models` preserves current/id/provider/label/readiness/provenance and
  additively exposes provider-owned `effort_levels` (an empty list is
  authoritative: that route sends no effort parameter); the route fixture verifies
  canonical metadata and ordering without performing inference. Legacy `models`
  stays intact; additive `routes` and `current.route` carry provider-qualified
  selections. Session creation/config accept both legacy ids and qualified
  catalog routes, persist the wire model/provider pair, and reconstruct the
  qualified route for resumed turns through `SessionModelConfig::model_spec`.
- Restart the daemon only by specific PID; do not use blind `pkill` sweeps.
- HTTP turn routes must resolve effective cwd from client cwd/project metadata and must never fall back to daemon process cwd.
- Do not bypass runtime permission gates from daemon route code.
- `host_guard.rs` checks every presented Host and absolute-form URI authority
  before CORS or dispatch. Preserve loopback, the actual bind IP (all IP literals
  for an unspecified bind), configured origin hosts, and `OCEAN_ALLOWED_HOSTS`;
  foreign names return fixed-shape 421 `host_not_allowed`. Startup rejects any
  malformed configured host and logs the effective allowlist once.
  Configured-origin IPv4/IPv6 hosts match parsed IP identity on any port,
  including an IP-preserving forwarder. Origin-derived grants never admit
  neighboring addresses, DNS aliases or forwarding headers as replacement
  authority; the existing bind/loopback rules still apply.
- `cross_site_write.rs` rejects Cookie-bearing writes and any present foreign,
  opaque, or unreadable Origin/Referer before dispatch with fixed 403 codes.
  `cors.rs` owns the shared `BrowserOrigins` trust set and pure authority
  normalization. Native callers and the authenticated Surface proxy retain
  origin-less, cookie-less writes; trusted loopback, Tauri, extension, and
  configured origins remain accepted. These guards do not authenticate callers
  or replace route/runtime permission checks; public proxy login remains owned
  by Surface. DNS-based daemon consumers must configure their upstream host.
- Global approval policy is exposed at `GET/POST /v1/settings/permissions` with
  manual, automatic (default), and skip-all modes; the legacy yolo endpoint
  remains a compatibility adapter and request-wire `yolo` remains inert.
- `call-voice` turns are always `HarnessProfile::Voice`, `yolo: false`, and
  `PromptControl::without_tools()`, regardless of global or persisted YOLO.
- `POST /v1/voice/stt` remains the credential-owning batch transcription seam
  for every first-party surface. Browser `application/octet-stream` retains the
  historical WebM multipart metadata; `audio/wav`/`audio/x-wav`/`audio/wave`
  selects bounded native WAV metadata for TUI dictation. Unknown content types
  fail soft to WebM for wire compatibility; no client receives the xAI key.
- `POST /v1/voice/realtime/client-secret` resolves only the dedicated Realtime
  voice credential (`OCEAN_OPENAI_REALTIME_API_KEY`,
  `OPENAI_REALTIME_API_KEY`, or auth-file `openai-realtime.api_key`). It must
  never inherit the agent `openai` API-key block or `openai-codex` OAuth.
- The daemon default Realtime model is `gpt-realtime-2.1`; preserve that exact
  ID in the upstream `session.model` body unless a surface explicitly supplies
  a compatible per-request override.
- Realtime `purpose: "planner"` is pre-session and propose-only: validate the
  registered project plus canonical live worktree before credential resolution,
  advertise only bounded read-only workspace inspection tools plus
  `propose_handoff`, and mutate only through the existing session/message/turn
  routes after an explicit Surface click.
- Realtime `purpose: "conversation"` may advertise bounded read-only
  `list_workspace` / `read_workspace_file` tools only when the daemon resolves
  the supplied session's persisted `workspace_root`/`cwd` back to a registered project
  or live linked worktree. Return that canonical root with the secret for frozen
  Surface fulfillment; never accept a browser/model-nominated conversation root,
  and keep project-less/session-less conversations on render + handoff only.

- `POST /v1/ocean-buddy/events` is the deliberately narrow first Buddy ingress: it accepts only a typed mocked `attached` lifecycle event, carries attachment metadata but no image bytes, performs no camera/session/tool work, and returns a typed Watch result card. Watch approval remains in the Watch-to-iPhone adapter flow.
- Session behavior lives in `ocean-agent`; route changes must not create a separate session model.
- `extension_lifecycle.rs` is the metadata-only Stage A lifecycle authority: it owns deterministic source adaptation, synchronous registered-project classification, globally ordered non-blocking publication, bounded correlation, request-scoped exactly-once terminal authority, cleanup, and boot-local count/byte retention. `main.rs` wires only the nine ratified authoritative producers; `session_stopped` remains schema-only. This boundary must not gain process launch, transport, registry mutation, routes, persistence, Observatory coupling, or content-bearing fields.
- `extension_registry.rs` and its private `src/extension_registry/` child are the sole coherent extension registry authority. Shared-lock reads preserve the three A0 schema shapes, strict fourth-file service authority, descriptor/reparse-safe artifacts, digests and inspect/doctor behavior. The internal A3a writer owns local acquisition, mutations, journaled four-file publication/recovery, retention and exact grant confirmation; it is not bound into AppState, startup, routes, CLI or live supervision. Strict `stage-a-publication.json` is created only by complete journaled or atomic absent-root publication; marked missing companions fail incomplete, unmarked A0 absence remains the sole empty upgrade form. The `registry-portability-check` feature removes only daemon route coupling and excludes the Unix-only writer for the actual-source Windows reader; it never weakens reader or platform validation. The child contract owns transaction details and narrow fixtures. Independent A3a acceptance and later A3b/A4/A5 authority remain separate.
- Recursive package hashing keeps its 64 KiB read buffer on the heap. Preserve
  the frozen digest encoding, descriptor traversal, and byte/count/depth limits.
  The Unix small-stack regression isolates its worker in a bounded subprocess:
  depth 64 completes a full snapshot and depth 65 retains the existing refusal.
  This reader repair adds no later-stage registry or supervisor authority.
- Unix registry entry admission checks type with descriptor-relative
  `fstatat(AT_SYMLINK_NOFOLLOW)` before opening, admits only regular files or
  directories, adds `O_NOCTTY`, and verifies the opened device/inode against the
  captured identity before returning a handle. State JSON remains regular-only.
  The precheck and open are separate syscalls: concurrent substitutions can be
  opened before identity refusal; this does not prove zero device side effects.
  Issue #44 owns this two-file reader/devlog repair and synthetic type/replacement
  fixtures. That entry admission repair preserves digest/schema/lock/platform
  behavior and adds no registry writer, marker creation, journal or routes.
- `extension_service.rs` is the Stage A2a–A2b macOS/Linux supervisor: exact acknowledged native services, strict hello/ready stdio, immutable activation scope/epoch and replay floor, bounded replay/live data plus coalesced prioritized controls, and a fixed-size delivered-sequence ACK ledger. Ready/reset/replay attach shares one cancellation-aware deadline and drains legal child frames concurrently; event ACK eligibility begins only after a successful complete write, with an exact in-flight ACK buffered but non-authoritative until then. Caller-owned frame-prefix storage makes fragmented child frames lossless across `select!` cancellation. At the full live ACK window, event dequeue pauses while child frames receive a bounded prioritized drain opportunity; only a genuinely undrained window fails the connection. Heartbeat timeout begins only after a successful ping write. Restart/backoff/circuit history survives scope-only epoch changes and resets only on disable→enable, trusted digest change, stable-health reset, or daemon restart. It preserves A2a's descriptor/file-id-bound executable and assigned roots, `env_clear` plus explicit ordinary/`env:` secret bindings, typed post-ready failure causes, bounded structurally redacted stderr counters, descriptor-relative temp cleanup accounting and retry ownership, retained-leader generation-safe group cleanup, and one immediate bounded exceptional-authority retry in the managed production task. Exceptional cleanup aborts/bounds stderr collection before returning authority; the managed owner retains a still-failed process/temp root for later reconciliation or shutdown, and supervisor shutdown executes cleanup ownership instead of abort-detaching it. Project reconciliation is checked and must complete epoch/filter/reap work before success. `extension_service_unsupported.rs` reprojects common validated authority on project changes, remains non-probing `unsupported_platform`, and opens no secret, assigned root, or process. Neither supervisor owns registry mutation, routes, durable replay, package acquisition, or child-originated commands.
- Product turns and legacy/call turns (legacy requests pin a session id before
  admission) take the shared non-blocking session operation lease before
  `TurnStarted`, invalidation, or request registration and retain it through
  persistence plus terminal publication. The registry's exact terminal
  transition is authoritative for the agent-rail `TurnFinished`: a cancel race
  must publish `Cancelled` with no failure error even when the runtime result
  was res-derived as failed or completed, and an orphan guard that settles a
  cancelling task must emit the matching cancelled terminal frame rather than
  leaving the agent rail open. Repeated settlement of an already-terminal
  cancelled request must not refire the terminal callback or emit another frame.
  Durable-room turns wait on the same
  lane after their durable queued footprint and before request registration, so
  a committed trigger is never dropped. An ordinary product turn registers its
  request, mints terminal authority, and constructs/captures the orphan guard
  before lifecycle admission; abort-before-first-poll therefore still settles
  exactly one terminal fact. The configured turn permit pool is clamped below
  terminal-authority capacity, and authority-mint failure rejects admission
  rather than running a turn without exactly-once terminal bookkeeping. Explicit project identity must match the resolved
  workspace/session scope; mismatches fail before lifecycle publication. Every
  mutation path emits a scoped agent-rail lifecycle event or
  `ocean.session_changed` invalidation while leased, making synchronized
  snapshots replay-safe even if execution aborts.
- `POST /v1/sessions/{id}/compact` takes a turn permit from the shared limiter
  (429 at capacity), rejects a busy session lease immediately with 409, emits a
  session-scoped replay fence while holding the lease, and delegates model work
  to `AgentRuntime::compact_session_with_lease`. Successful/no-op responses
  include the lease-protected visible transcript snapshot plus that fence.
  `GET /v1/sessions/{id}/sync` is refresh-only, returns the same bounded public
  snapshot/fence, and rejects a busy lease with 409. Only genuine absence maps
  to 404; unreadable/internal errors are sanitized 500s and provider failures
  remain `200 ok:false`. No model compaction logic belongs in the daemon.
- `POST /v1/agent/sessions` accepts an optional catalog model and persists that
  model/provider atomically with the new session at `config_revision: 1` before
  returning or publishing `SessionCreated`; unknown model ids fail 400 without
  creating a session. This is the first-turn pin path, not a create→PATCH pair.
- Session-config RPC v1 is `GET/PATCH /v1/agent/sessions/{id}/config`;
  PATCH accepts strict model-only JSON (malformed or extra-key bodies return
  exact `400 {"ok":false,"error":"invalid_request"}`), persists the catalog
  model/provider pair plus a monotonic legacy-default-zero `config_revision`,
  returns that revision from GET/PATCH and synchronized snapshots, and emits it
  on exactly one session-scoped `SessionConfigChanged`.
  Model metadata does not mutate transcript history, so this route must not also
  emit the generic `ocean.session_changed` sync invalidation. Permission state
  is read-only and `permission_mode.env_override` is a boolean
  presence flag. Only
  absent sessions map to 404; corrupt/internal reads and writes return sanitized
  500s. `model_source` comes from ocean-agent's persisted `config_revision`
  authority; model/global comparison is used only for revision-zero inherited
  or legacy records. Turn selection is explicit model > resolved role > named-agent model >
  session pin > global; an explicitly named unresolved role stops at global,
  and `TurnStarted` announces the model passed to execution before any separately
  announced provider reroute.
- `GET /v1/agent/history/search` is a bounded adapter over ocean-agent's persisted display-transcript search (default 20, clamp 1..50); it performs no provider or embedding calls, allows at most two concurrent scans, and returns a capacity response before deserialization when raw session files exceed the 64 MiB request budget.
- Subagent roles, dispatch, lifecycle, and orchestration are extension-owned. Do not add daemon-native `task`/`spawn_worker`/fleet machinery. The daemon may expose generic permission-gated turn, cancellation, capability-provider, and extension event/tool seams; current `/v1/subagents/spec` and folder-agent subagent metadata remain compatibility surfaces until a separately approved extension migration.
- Slack Socket Mode, API/credential access, reconnects, replies, files, and real Canvas delivery are `ocean-slack` extension concerns. Private `slack_canvas_fulfillment.rs` is only the temporary typed host ingress/readback, runtime lookup, scoped-event, and lifecycle-enforcement compatibility seam; do not grow it into a second Slack transport authority.
- `component_interaction.rs` is a leaf HTTP fulfillment adapter over the runtime-owned `COMPONENT_WAIT_REGISTRY`: preserve exact key scoping, remove-before-send semantics, poison/error responses, and runtime ownership of wait registration, timeout, and ordinary cleanup.
- `model_roles.rs` owns once-at-startup fail-open `[roles]` loading and pure turn/advisor alias precedence only. Keep `AppState`, warning call sites, provider routing/readiness, persisted model selection, and advisor execution in their existing owners; do not trim or validate role strings during extraction.
- Post-turn advisor execution is best-effort and isolated from the completed main turn: preserve activation/alias precedence, use only the dedicated fixed two-permit `AppState` limiter with immediate fail-open saturation, hold its owned permit across the provider call, and keep the call under a fixed 30-second timeout. Advisor Extension payloads retain `note`/`severity`/`model` and carry the authoritative originating `turn_id`; logs and Prometheus labels must never contain prompt, response, or note content, and metrics labels remain fixed-cardinality outcomes only.
- `request_control.rs` owns the private request/permission registry records, status-only snapshots, registration/handle mechanics, waiter cancellation, and bounded status transitions. Keep `AppState`, permission policy/orchestration, decision-token verification, HTTP/event mapping, GC scheduling, active-turn projection, and shutdown draining in composition; preserve sender/handle ownership and drop registry locks before signaling or awaiting.
- `recall_registry.rs` owns only the private in-memory `title_id -> RecallVote` store, first-cast tally construction, distinct-voter casting, poison recovery, and named-tally removal. Keep UUID/live-title validation, persisted title and daemon-held Revoker authority, carried-outcome execution, HTTP mapping, and successful-only cleanup ordering in composition. Preserve the existing memory-only, unbounded retention of abandoned tallies during behavior-neutral extraction.
- `persistent_rooms.rs` owns the shared room-store adapters, durable-room lifecycle/paging and Local/federated message, invite/redeem and agent-registration adapters. Keep AppState, startup, route composition, call persistence/retries and LiveKit authorization in composition. Local triggers persist the caller row and publish wakes before generation-bound admission, durable allow audit, convene footprint and spawn; confirmed federated claims revalidate current roster/credential and private binding before at-most-once dispatch. Replies use exact-generation local writes or the outbox, with daemon-minted generation-scoped session attribution. Preserve one concrete store, poison recovery, no store guard across await/event/spawn, ordinary three-state permission authority and closed-room audit replay. Local HTTP authors must be exact Human roster members; non-roster, whitespace variants and client-claimed Agent/System kinds fail closed without write. The message wire rejects caller `session_id`; admitted replies thread under the resolved root, with stale-parent degradation to top-level.
- `room_federation.rs` owns the restart-safe outbound Bedrock room client and AppState-owned `FederationSupervisor`: strict origin-only client construction, header-bearer SSE receive/reconnect, roster/presence projection, durable Pending sender scans, stable producer/control admission, owner bootstrap, idempotent redemption/self-join recovery, safe-agent registration, status/revoke policy, and post-commit wakes/dispatch hints. SQL remains `ocean-store`; HTTP handlers and local agent execution remain `persistent_rooms.rs`. Keep exactly one task tree per room, serialize stop/join before the next epoch, start existing credentials before the bounded-concurrency all-row recovery worker, select sender Notify + bounded periodic durable scan + cancellation, and never hold `RoomStoreHandle` across network I/O or `.await`. POST 201 never mutates transcript/outbox; ordered SSE is the only ingest/removal rail. Presence follows the authenticated SSE lease, not merely the access-state label. Bearers, registration keys, and invite codes (except intentional invite success) never enter logs/errors/debug output or surface projections. Successful local invite redemption additively returns the daemon-authoritative `room_key` beside the existing access projection so clients never trust an editable deep-link path as room authority.
- Federated read-cursor GET/PATCH and epoch refreshes take one per-room operation lock before credential/generation/mirror snapshots. Upstream SSE cursor values are only coalesced invalidation hints; authenticated GET supplies the current nullable value. Refresh is an epoch-owned, cancelled/joined worker, including during authenticated snapshot catch-up; denial returns to the room owner rather than calling a self-joining public revoke path. Epoch revocation revalidates generation/credential under the stable gate and store guard before closing or mutating authority. Revalidate credential identity, generation and stable/epoch admission before mirror commit; keep store guards out of network awaits. PATCH accepts explicit higher `changed:false` no-ops and empty-room `clamped:true, changed:false` null responses while rejecting unflagged mismatches.
- Durable room failure logs carry only a fixed error code and correlation IDs; never copy response/stderr, prompt, tool, body or credential text into that durable stream.
- Persistent-room artifact HTTP create/amend classify the roster author kind and perform the write under one store guard. Agent/System authors remain daemon-only; unreadable roster/store state fails closed.
- Persistent-room Agent joins reject unresolved folder definitions. Executable turns capture one immutable package/profile snapshot and validate persisted generation/owner authority before request visibility or convene footprint; a valid data-only definition remains resolved. Room execution requires a usable canonical cwd and admits only its scoped capabilities. Ordinary named-agent routing stays fail-open; underlying permission-mode, YOLO and decision-token gates are preserved within the new Room admission boundary.
- Persistent-room detail and snapshot responses expose only the bounded public participant-alias projection `{from,to,retired_at}` (including `[]` when absent) and `aliases_truncated` so clients can distinguish a complete list. `GET /v1/rooms/persistent/{key}/inspect` is a read-only, bounded identity view of room id/name, local owner member id, and aliases with the same completeness flag; it excludes transcript, workspace, and store internals. Store membership writes remain the authority that permanently refuses a retired `from_id`.
- Persistent-room mention tokens use the canonical participant-id alphabet:
  alphanumeric, `-`, `_`, and `.`. Keep parser, roster lookup, folder-agent
  resolution, Surface completion, and federated outbox mention ids aligned.
- `GET /v1/rooms/persistent/{key}/events` is the open, non-call persistent-room merged SSE tail: it bootstraps full `room_access` and JS-safe decimal-string `room_read_cursor` projections without `event.id`, preserves id-bearing `room_message` replay via `Last-Event-ID` (numeric or 400; wins over `after_seq`), then tails three post-commit wake buses (`RoomAccessWakeBus`, `RoomReadCursorWakeBus`, `RoomWakeBus`). Wake hints are payload-free; relevant and lagged hints reread SQLite. Cursor tails use the daemon Local principal or the credential-owned federated human principal, suppress unsupported/transient projections rather than emitting false clears, and deduplicate the unified `{room_id,read_seq}` wire body. Message gap recovery pages ascending; access dedup compares the full projection. Unknown/closed rooms return 404, `call:` rooms return the typed unsupported rejection, and the stream uses the shared 3-second keepalive plus shutdown wrapper. Every non-call production transcript writer must publish only after its allocating transaction commits. Tail tasks must select downstream `tx.closed()` so disconnected clients release state and wake receivers without a new hint.
- `POST /v1/rooms/persistent/{key}/outbox/retry` is the strict retry adapter: accepts `{ "client_event_id": "<id>" }`, returns 202 on durable requeue, 403 revoked, 404 for an unknown room or item, 409 pending/local, 400 for malformed or non-object body, or sanitized store 500 on internal error. No network or provider calls; the adapter owns only HTTP validation, store lookup, and wake publication.
- `longhouse_preparation.rs` owns only the state-free prepare/inspect/workflow HTTP request/projection adapters. Preserve exact Axum extractor/method/default envelopes, PR #292 exact-token evidence and redaction, cwd roots/cache choice, and all three `spawn_blocking` fail-open lanes. Keep route composition, librarian query/fetch, compatibility subagent spec, and all governance/title/escrow/recall state in `main.rs`; ranking/cache algorithms remain in `ocean-longhouse`. The deferred cached skill-path symlink-retarget finding must be resolved separately before any librarian extraction.
- `longhouse_turn_preparation.rs` owns only the fresh default-on opt-out gate, deterministic advisory rendering/application, fixed 250 ms deadline, and cached read-only `TurnPrep` selection inside one blocking closure. Keep all three call sites in `main.rs` with exact caller-cwd, request/permit/acknowledgement, event/runtime, raw-versus-guided prompt, and browser-layer order. Helper-owned warnings remain fixed-field, while delegated loader path logs, unsanitized advisory names/descriptions, and uncancelled timed-out work behind the process-wide cache lock remain documented separate risks rather than extraction scope.
- `longhouse_topics.rs` owns only the detached scripted demo producer and read-only topic list/detail HTTP adapters over the existing `AppState::{agent_events,longhouse}` handles. Preserve the immediate acknowledgement, exact 17-event/delay/content/ID/tally sequence, projection-before-publication with no lock across publish/await, demo skip-on-poison/live-publication asymmetry, list/detail poison recovery, and exact UUID/error/envelope behavior. Keep `longhouse_routes()`, `AppState`, startup's one registry shared with runtime extensions, HTTP/SSE composition, real convene/model selection, and every title/escrow/revoker/recall/breach/board control path in `main.rs`; do not add a helper, registry, service seam, or broader governance authority here.
- `longhouse_governance_control.rs` owns only the exact 13-definition claim/revoke/recall/breach/board HTTP adapter boundary. Revoke/recall/breach/board have no caller-authentication extractor: Host/browser-write guards and CORS are the current deployment posture, not authentication. Recall deduplicates caller-supplied voter UUIDs and omitted/zero threshold clamps to one; accepted live-title breach reports accrue persisted strikes, while a post-close report returns 200 with zero strikes rather than 409; board state is an in-memory projection whose poisoned second lock can skip mutation while still publishing and returning success. Preserve these characterized behaviors without endorsing them; keep `AppState`, route composition, Revoker/recall construction, title storage, real convene/title grant-bind, provider execution, and raw-token delivery in composition.
- Agent SSE replay is globally bounded by both 2,048 events and 32 MiB of serialized event payload. Oldest envelopes evict until both limits hold; an individually oversized event remains live but is not replay-retained. Empty, non-UTF-8, malformed, foreign-session, unknown, or evicted `Last-Event-ID`, and live broadcast lag, emit the existing `event:error` frame with a typed `AgentReplayGap` body and `reset_required:true`; never silently attach live-only after an unavailable anchor. Gap bounds are filtered to the requested session and remain diagnostic opaque UUIDs. Preserve full live delivery and first-party error-frame compatibility.
- A failed admitted agent turn must retain a fixed failure code in daemon
  logs with turn, request and session correlation. The one-shot SSE error frame
  and an `ok=false` summary alone are insufficient postmortem evidence. Never log
  prompt, response, tool output, credentials, or authorization material.
- Build provenance must follow normal branch commits and linked worktrees: `build.rs` watches Git `HEAD`, its resolved symbolic branch ref, and `packed-refs`; `/health` and `/ready` must report the exact main-built revision after deployment.
- `AgentEvent::TurnCheckpoint` is an internal persistence signal consumed by `ocean-agent`; daemon bridges must filter it rather than exposing transcript deltas on SSE.
- The Track-0 projection routes (`GET /v1/rooms`, detail, snapshot, events) are retired. Preserve `/v1/rooms/persistent/*` and `/v1/rooms/{room_id}/livekit-token`; these are separate durable-collaboration and media contracts.
- The explicit method/path set in `app_router`, `banner_routes()`, and the operator-guide HTTP quick reference must remain identical. Preserve Axum's default 404/405 fallback and the global layer order: HTTP tracing outside Host allowlist outside CORS outside browser-write guard outside route dispatch.
- `github.rs` owns exactly five public, read-only `GET /v1/repo/github/{project_id}/*` projections: pulls, one pull, full-head-SHA checks, reviews, and commits. Resolve only the registered workspace-root `origin`; accept only exact GitHub remote forms; never send credentials or `Authorization`; and add no aggregate or write route. Preserve route-owned sanitized extractor errors, two-phase byte/field/vector bounds, Link-only pagination, full-SHA Moka singleflight caches (256 entries/60s), pinned GitHub headers, and bounded kill-on-drop git stdout handling.
- `POST /v1/longhouse/inspect` is a read-only projection of the exact ordinary preparation ranking: preserve the shared request/cwd roots/cache/cap/exact-token scorer/tie-break path, path-redacted compact response, raw-prompt/session/cwd/body non-echo (only contributing prompt terms and the additive `exact_name_phrase` flag are returned), and separation from turn execution, capabilities, models, and automatic prompt injection.
- `harness_profile.rs` owns only the effective per-turn profile gates currently applied to `PromptControl`: hashline edits and artifact spill. LSP/memory remain globally registered, and stream rules, rich context, and minimization remain unavailable rather than logged-only profile claims. Preserve the unknown/missing → CLI fallback; `acp-zed` resolves explicitly with the same effective gates as its former fallback. New external surface classifications require a separate cross-repository policy decision.
- `observatory_auth.rs` owns the typed Axum auth-state/extractor seam: accept `Authorization: Bearer` or the `Authorization-Observer` compatibility cookie, never query credentials, and map every credential failure to 401. Startup loads the secure secret, atomically mints the mode-0600 boot-bound `.ocean/observatory-token`, mounts typed extension state, and rotates the token file every ten minutes; no public token-creation route exists, the daemon never emits `Set-Cookie`, and the signing secret is never distributed. Observer tokens are stateless bearers replayable within scope until expiry or daemon restart; their nonce provides issuance uniqueness, not one-time consumption. Any compatibility-cookie issuance and its `Secure`/`HttpOnly`/`SameSite=Strict`/scoped-Path attributes belong to the authenticated Ocean Surface proxy.
- `observatory_adapter.rs` owns the one-way bridge from the runtime `AgentTurnEvent` stream to redacted Observatory facts: content-bearing variants (text/thinking deltas, tool chunks, component/canvas/extension payloads) return `None`, tool args/output bodies and free-text errors/titles/paths are stripped by construction, and reroute/error reasons are classified to fixed codes. Startup runs the restart interruption sweep (nonterminal executions close as canceled), emits `DaemonStarted`, and spawns the append pump off `AgentEventBus::subscribe_with_full_replay`; graceful shutdown appends `DaemonStopping`. Pump lag is `warn!`-loud because the pump is the durability path.
- `observatory.rs` owns the read-only Observatory data routes (`GET /v1/observatory/snapshot|events|replay`) behind the `ObservatoryAuth` extractor: snapshot/replay answer 410 only against the durable retention-boundary watermark (a natural log start at cursor 1 is not a crossing), with replay boundary validation and page reads in one store lock (after is exclusive, so equality at the boundary resumes cleanly); successful snapshot cursor headers come from the returned projection watermark, the SSE tail always replays from the durable store before live attach and emits explicit `reset`/`error`/`stream.gap` frames instead of silently skipping, and every response carries the manifest §7.4 no-store/X-Observatory headers. The store is optional at runtime: open failure degrades routes to explicit 503, never daemon startup. V1 projection gaps (empty session/turn/request ids, empty attention shelf) stay empty until Task 6 wires real daemon facts.


### Room request-authority implementation manifest (#33)

- Base: merged canonical source; selective inputs are immutable running
  `0abb558179af3ff848d3bd19ac67aff246299ce1`. This stage restores local
  generation-bound execution, not full #22 parity or deployment acceptance.
- `room_operator.rs` owns the header-only, owner-only local operator key and
  cookie/origin refusals. `room_agent_authority.rs` owns package preview,
  Local bootstrap, binding reads/authorize/reauthorize/status mutations,
  immutable admission, fixed-code audits and per-call/lifetime authority checks.
  Missing key/header is 503; invalid authority is 403. Roster, browser trust,
  network location and permission mode never substitute for this principal.
- Capture parses and hashes one immutable, complete UTF-8 relative-path byte
  map through no-follow descriptors. New local ceilings (not original behavior)
  are 10,000 directory/file entries, depth64, 8MiB per file, and 32MiB combined
  content/map-key bytes; checked arithmetic and limit-plus-one reads refuse
  overflow/growth. Reject symlinks/special files and unsupported platforms.
  Valid accepted maps keep the original definition digest bytes. No new global
  capture scheduler, dependency or semaphore is part of this stage.
  Existing `turn_limiter` supplies a new local capture/execution bound: acquire
  before capture without awaiting locks, refuse busy before admission, then
  carry one separate owned permit through queued/executing terminal settlement.
  Admission/callback clones never own capacity; previews release on return.
- `request_control.rs` adds private immutable Room/member/generation/admission/
  decision/digest/session metadata and checked registration. The request write
  lane precedes the store guard: final validation, durable allow audit and
  request visibility linearize there; audit failure leaves no queued request.
  Ordinary registrations retain `None` and existing decision-token custody.
- `persistent_rooms.rs` admits Local mention/thread, explicit and confirmed
  federated turns before convene footprint or context, uses generation-bound
  sessions, bounded projected history, exact-generation reply/failure writes,
  cancellation-aware lifetime validation and exact-Room close cancellation.
  Existing `room_create` canonicalizes supplied directory roots before persistence
  through original private `canonical_submitted_workspace_root` and typed
  `invalid_workspace_root_response`; omitted/null roots stay unbound.
  Registry settlement owns the single terminal result; cancelled work cannot
  publish reply/failure content. New capability scheduling rechecks authority,
  including memory. Admission, issued callbacks and request registration share
  one private cancellation token; execution alone owns the capacity permit.
  Refusal/abort cancels that token, and normal/error terminal output settlement
  cancels it without rewriting the settled registry fact, so cached capability
  clones cannot schedule later I/O. An already-running memory operation is not
  atomically revoked. Preserve existing session leases and permission gates.
- `room_profile.rs` is only a read-only required-slot status adapter against
  runtime-captured config authority. `room_resources.rs` is only cwd/catalog
  selection plus scoped list/read authority and content-free audit callbacks.
  Return the stored canonical grant root unchanged to #24 descriptor I/O;
  catalog/handle issuance alone is not an in-flight operation proof. No profile
  or grant mutation/preview routes are opened.
- Approved consumer fanout: Agent `memory_tools.rs` adds a private optional
  operation callback on the opaque admitted handle, with fixed typed refusal;
  Agent `lib.rs` only reexports that interface and Agent `AGENTS.md` records it.
  Every daemon-created Room memory handle attaches the callback. Actual
  `retain`/`recall` execution checks before and after its mutex wait, then
  immediately before put or each recall page; no Room guard spans that wait;
  observed revocation refuses subsequent operations without put/scan. This
  operation-start admission does not make Room-store validation plus memory I/O
  atomic and cannot cancel an already-running synchronous operation. Preserve
  constructor/admission APIs and ordinary operator tools unchanged.
- `room_federation.rs` adds authorized reply allocation under its existing
  stable admission gate, one exact-generation store transaction and post-commit
  outbox/audit wakes. The accepted opt-in build-failure hint remains at-most-once
  with `Unknown` activation refused; CI remains marker-only. An allowlisted
  workspace row with a validated exact Room scope/sequence but invalid typed
  payload advances the durable cursor without transcript/trigger facts, so a
  poison marker cannot block subsequent rows. Store/transport faults, malformed
  envelopes and scope/id/sequence confusion keep their existing recovery paths.
  Preserve #14/#15 cursor credential, generation and cancellation rails.
- `main.rs` adds only modules/operator state/startup, the binding/explicit/close
  registered routes, matching banner/operator reference, required AppState and
  request fixtures, and meaningful original Room fixtures. No ordinary provider,
  session, prompt or authentication implementation is replaced.
- Verification: original operator/package/admission, registration linearization
  and audit rollback, activation/owner-kind/cross-Room/digest/slot refusal,
  generation-scoped history/resources/output, cancellation/close/permission,
  normal mention/thread/explicit/federated execution and exact terminal fixtures;
  new synthetic capture-bound and per-call memory-revocation checks. Root runs
  locked focused/full daemon tests, denied-warning Clippy and workspace test
  compilation remotely; independent exact-source review is required.
- Deferred: Room metrics/sampler, attachments/context, profile/resource writes,
  retirement/summary/maintenance/workspace bridge, client onboarding/UI, extension
  scheduling, Unknown build execution and CI orchestration. Keep #22/#48 holds;
  automated CI remains only Build Ocean and Build Surface.

### Reviewed package consent manifest (ocean-private #55)

- Ported from ocean-private #59 onto the public canonical source. Targets are only `src/room_agent_authority.rs`, registered-route fixtures in
  `src/main.rs`, and this contract. This additive consent boundary follows #33;
  it does not complete client onboarding or lift #22/#48 deployment holds.
- Strict authorize/reauthorize bodies accept optional
  `expected_definition_digest`; omission and JSON null preserve legacy callers.
  Both routes require a JSON object through a map-only decoder that passes the
  original map directly to the strict DTO deserializer. Reject positional arrays
  and all other non-objects without collapsing duplicate keys into a Value.
  A supplied string must exactly match the single immutable package capture
  used for decision hashing and binding persistence. Never trim, reopen, or
  independently recapture to check it. Refuse mismatches with fixed, content-free
  `409 definition_digest_mismatch` before decision consumption, binding/audit
  mutation, or request cancellation; retain existing operator and owner policy.
- Registered production-route fixtures preview then modify disposable package
  bytes, proving stale authorize/reauthorize leave bindings, generations,
  decisions, transcript, requests and cancellation unchanged. Fresh preview
  succeeds; exact replay, conflicting decisions, legacy omission/null and strict
  malformed bodies retain their existing behavior. Future reviewed-consent
  clients must send the digest rather than falling back to legacy submission.
- Object-only repair (the second ocean-private #59 revision): the same three
  targets remain owned. Shape-correct positional-array fixtures must refuse with fixed
  `400 invalid_request`, preserving binding/generation/decision/transcript and
  a live request's cancellation state. Equivalent object controls must succeed;
  retain duplicate/unknown rejection and all same-capture/replay invariants.
- Root runs locked focused/full daemon tests, all-target denied-warning Clippy
  and workspace test compilation with both auth paths and config/XDG pinned to
  disposable absolute paths, unchanged HOME. Independent exact-head review is
  required; no store/schema, client, dependency, route or permission expansion.

### Sovereign federated own-agent consent manifest (ocean-private #62)

- Base: the reviewed package-consent section above (ocean-private #59);
  ported here from ocean-private #66. Exact targets are
  `src/room_agent_authority.rs`, registered production-route fixtures in
  `src/main.rs`, test-only federation execution fixtures in
  `src/persistent_rooms.rs`, and this contract. The accepted Surface
  [post-G3 program](../../../../apps/ocean-surface/docs/specs/2026-08-13-ocean-rooms-post-g3-product-depth.md)
  separates R5 sovereign agent custody from R6 Room administration.
- `room_owner_proof` proves node-local own-agent identity, not shared Room
  administration: in Live/Recovering federation, the stored credential must
  name an exact current User roster row with Owner or Member role. Local
  ownership/bootstrap stays unchanged and Local-only. The exact target must
  remain an Agent owned by that human with the captured local package binding.
  Missing/revoked credentials, missing/reclassified rows, Connecting/Revoked
  access and foreign ownership fail closed. Preserve existing refusal codes.
- The only three direct calls are Local/federated `target_proof` and binding
  list identity projection. Preserve its transitive preview/consent/status,
  admission, request registration/lifetime/cancellation, scoped memory/resources/
  tool authority and federated reply consumers. No role promotion, Room
  administration, store/schema, protocol, client, operator, ordinary permission,
  provider, dependency or deployment change belongs here.
- Registered route fixtures use two independent disposable node identities:
  creator Owner and invited Member with distinct owned agents. Prove own
  preview/digest consent, reauthorization, invocation/admission, status/revoke
  and generation cancellation; foreign node/agent/key/credential/roster/kind/
  package/digest/access, replay and cross-Room authority remain refused with
  appropriate no-mutation assertions. Keep existing Local/operator/permission
  and digest fixtures. Parameterize the actual fake-provider federation
  execution proof for both Owner and invited Member; make no external provider
  calls.
- Root owns the sole locked remote Cargo lane: relevant complete Room groups,
  full daemon tests, all-target denied-warning Clippy, locked workspace-test
  compilation and docs-check. Both auth and config/XDG paths are absolute
  disposable paths; HOME stays unchanged. Require independent exact-head
  review and both actual hosted builds. Native broker/served clients, actual
  two-human/two-machine R1-R7 and ocean-private #22/#48 deployment acceptance
  remain separate; the ocean-private #58 billing and original-repository
  migration holds remain intact.

### Rooms persistence migration boundary

- Preserve the accepted store/core foundation and #14–19 repairs. The scoped
  admission stage above does not establish full production parity or lift the
  live-daemon migration hold; profile/resource writes, attachments, retirement
  and subsequent accepted stages remain separate.
- Create accepts the original `on_build_failure` marker hint path; its `Unknown`
  activation remains refused. Create refuses `on_ci_failure` before persistence
  because CI is marker-only. Legacy true flags round-trip, and omitted-policy
  store updates preserve them. No metadata-update HTTP route is introduced.
- Federation confirmed-message ingestion passes only the already validated
  `mention_member_ids` wire array to the normalized store rail. Never infer
  mentions from prose or replay legacy unknown metadata without exact confirmed
  event equality. Preserve the cursor operation/admission/generation guards.

## Work Guidance

- Keep HTTP/SSE contracts stable for both `ocean-tui` and `ocean-surface`.
- Caller-submitted and resumed turns execute in the caller's cwd; never pin them to the daemon launch cwd or the first session cwd. Admitted Room turns require a usable canonical workspace or authorized default folder; unbound turns fail closed with `workspace_unavailable`. Startup still rejects repository cwd.
- Build from up-to-date `main` before daemon restarts when doing operator work.
- Prefer narrow route tests for API behavior and workspace checks before merge.
- Guard room fixtures use registered production routes and assert committed roster/transcript effects for successful native/trusted writes and denied foreign writes.

### Accepted supervision recovery manifest (#43)

- Base: canonical main `26a88ddbd16b233ebb7dfde7699c9a6e1da3e3b4`.
  Targets are only `src/extension_service.rs` and this contract. Stage A
  sections 10, 11.2 and 18 authorize the existing A2a/A2b cleanup/reconciliation
  boundary; original `0abb5581` is source evidence, not later-stage authority.
- Key retained exact process/temp descriptors by service identity; finish every
  obsolete cleanup, withhold only unproven replacements, and acknowledge the
  complete pass through the existing project-snapshot Result. Retain an
  authority-less failed stop as a blocked key rather than inventing recovery.
  Preserve same-digest restart history while cleanup delays a scope replacement.
- Cleanup proof belongs to the exact managed owner and follows its returned
  descriptors. A successful retry remains authoritative after its retained row
  is consumed; joining that owner's earlier false result must not invent an
  authority-less failure or poison a proven replacement key.
- Impose a new boot-local ceiling of 1,024 distinct owned service keys, aligned
  with the reader's service-grant count. Count the union of managed, retained
  and authority-less blocked keys; refuse only new admissions at capacity,
  never cleanup/retry or an already-owned key. Retry incomplete startup/project
  passes indefinitely with 500 ms exponential delay capped at 30 seconds, resetting on success;
  cancellation interrupts waits without abandoning native cleanup ownership.
- Repair read-only connection-temp directories only through captured no-follow
  descriptors. Keep public command/start/status APIs, unsupported-platform
  behavior, transport, lifecycle, registry and startup composition unchanged.
  Do not import registry writers/journals/markers/revision commands,
  `PackageGeneration`, `ActivityLedger`, routes, CLI or later-stage machinery.
- Fixtures use coherent synthetic registry/artifact state and real disposable
  A/B/C children to prove continued stops, unrelated starts, incomplete ACK,
  retained cleanup/recovery and circuit history. Add startup/backoff/cancellation,
  authority-less failure, owner ceiling and descriptor-bound read-only-tree
  proofs; preserve existing lifecycle/transport/group-generation fixtures.
- Root owns locked remote supervisor/project tests, daemon all-target Clippy,
  test linking and workspace test compilation with both disposable auth paths,
  config/XDG overrides and unchanged HOME. Source review remains independent;
  parent #28 and live migration #22 remain open.

### Internal A3a implementation manifest (#52)

- Base: canonical main51 `d1220f53fccbef0dc5902ec1732e93db526ea439`,
  with accepted Phase 1 `77630bdaccb2829c45b0f137e3750d2ec089d212`
  verified in its ancestry. Exact paths are `src/extension_registry.rs`,
  `src/extension_registry/transaction.rs`, its new child contract, this
  contract and the parent crates ownership cell. Stage A sections 5, 6, 12,
  13.1, 14, 17, 18, 19.4 and 21 authorize this complete internal A3a slice.
- Reuse one schema/validation authority by extracting the current lock-owned
  reader without changing semantics. Preserve every old reader, marker, heap,
  entry-admission, digest and Windows fixture. Select only earlier A3a
  `7804b39f` transaction source; exclude later original Git acquisition,
  activity-ledger/reconciliation binding and A5 composition.
- Implement typed local acquisition/install/update/trust/enable/disable/remove,
  expected revisions, strict journals, complete publication and recovery,
  all section 12.4 retention and exact native authority confirmation. Four
  acquisition permits are globally process-wide; canonical-root accounting
  disappears when idle. Persistent descriptor-verified config-parent shared
  acquisition/exclusive sweep custody protects live bootstrap/quarantine bytes
  across processes; local sweep waits and OS custody waits refuse after 250 ms.
  Unsupported directory-sync barriers are failures, with known commit evidence
  retained and no successful durable acknowledgement without all barriers.
  Source acquisition stays outside the 250 ms exclusive publication lock.
- Opaque trusted stopped-package input and supplied registered-project ids are
  internal preconditions only. No AppState, startup, supervisor, route/CLI,
  Git, new dependency, crate, credential or database change belongs here.
- Preserve all 22 original synthetic A3a fixture groups and add global/alias
  capacity, journal hash/checked-revision and truthful committed-recovery
  assertions. Fail closed with unknown facts when proof is corrupt; never
  invent an effective old revision. No package code may execute.
  Include real separate-process acquisition/sweep/crash recovery and synthetic
  lock-shape, bounded-wait and pre/post-commit durability-failure fixtures.
- Root owns locked remote checks, independent review, exact integration and
  both hosted build jobs. This source does not claim A3a acceptance, later
  stage authority, production activation or release of migration hold #22.

## Verification

- `cargo test -p ocean-daemon provider_auth:: --locked -- --test-threads=1`

- `cargo test -p ocean-daemon bus::tests::`
- `cargo test -p ocean-daemon fulfillment -- --nocapture`
- `cargo test -p ocean-daemon cors::tests:: -- --nocapture`
- `cargo test -p ocean-daemon host_guard:: --locked -- --test-threads=1`
- `cargo test -p ocean-daemon cross_site_write:: --locked -- --test-threads=1`
- `cargo test -p ocean-daemon startup::tests::allowed_hosts_ --locked -- --test-threads=1`
- `cargo test -p ocean-daemon component_event_ -- --nocapture`
- `cargo test -p ocean-daemon event_adapter::tests:: -- --nocapture`
- `cargo test -p ocean-daemon extension_ -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon extension_registry::transaction::tests::a3a_ --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon extension_registry:: --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon registry_entry_ --locked -- --test-threads=1`
- `cargo test -p ocean-daemon extension_service:: --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon --locked --no-run`
- `cargo zigbuild --manifest-path crates/ocean-daemon/tests/windows-portability/Cargo.toml --features registry-portability-check --target x86_64-pc-windows-gnu`
- `cargo test -p ocean-daemon fs_ -- --nocapture`
- `cargo test -p ocean-daemon metrics::tests:: -- --nocapture`
- `cargo test -p ocean-daemon model_catalog_ -- --nocapture`
- `cargo test -p ocean-daemon model_roles_ -- --nocapture`
- `cargo test -p ocean-daemon role_resolution_ -- --nocapture`
- `cargo test -p ocean-daemon session_config_ -- --nocapture`
- `cargo test -p ocean-daemon unresolved_role_executes_global_and_turn_started_matches_it_despite_session_pin -- --nocapture`
- `cargo test -p ocean-daemon project -- --nocapture`
- `cargo test -p ocean-daemon recall_registry -- --nocapture`
- `cargo test -p ocean-daemon recall_route -- --nocapture`
- `cargo test -p ocean-daemon request_ -- --nocapture`
- `cargo test -p ocean-daemon permission_ -- --nocapture`
- `cargo test -p ocean-daemon persistent_room_http_ -- --nocapture`
- `cargo test -p ocean-daemon room_read_cursor_ --locked -- --nocapture`
- `cargo test -p ocean-daemon room_ -- --nocapture`
- `cargo test -p ocean-daemon room_reviewed_digest_ --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon room_sovereign_ --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon p2c_ --locked -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon at_mention_queues_turn_and_posts_reply_back -- --nocapture`
- `cargo test -p ocean-daemon closed_persistent_room_preserves_audit_http_asymmetry -- --nocapture`
- `cargo test -p ocean-daemon workspace_policy::tests:: -- --nocapture`
- `cargo test -p ocean-daemon yolo_settings_ -- --nocapture`
- `cargo test -p ocean-daemon longhouse_inspect -- --nocapture`
- `cargo test -p ocean-daemon longhouse_preparation_ -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon longhouse_turn_preparation_ -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon longhouse_topic_projection_ -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon longhouse_governance_control_ -- --nocapture --test-threads=1`
- `cargo test -p ocean-daemon harness_profile -- --nocapture`
- `cargo test -p ocean-daemon router_contract -- --nocapture`
- `cargo test -p ocean-daemon github::tests -- --test-threads=1`
- `cargo test -p ocean-daemon observatory_auth -- --nocapture`
- `cargo test -p ocean-daemon observatory:: -- --nocapture`
- `cargo test -p ocean-daemon persistent_rooms -- --test-threads=1`
- `cargo test -p ocean-daemon room_create_holds_unwired_flags --locked -- --test-threads=1`
- `cargo test -p ocean-daemon`
- `cargo check --workspace`
- Manual daemon health check when route/startup behavior changes: `curl http://127.0.0.1:4780/health`

## Child devlog Index

- `src/extension_registry/` — internal A3a local registry transactions, recovery, retention and in-file synthetic fixtures → `src/extension_registry/AGENTS.md`
- `tests/windows-portability/` — isolated source-inclusion cross-build for the actual Windows registry reader and unsupported supervisor → `tests/windows-portability/AGENTS.md`

- `tests/fixtures/bedrock-room-events/{room-event-actions.json,vendored-from.json}`
  retain exact immutable0abb fixture bytes and historical producer revision
  `1c2e2993` / `2026-08-31` provenance. These pin the workspace-producer action
  subset, not the full Room stream or a fresh upstream export. No vendor script
  or additional producer source is imported.
