# ocean-agent — Session and Prompt Layer

## Purpose

This crate owns Ocean's agent session/history layer, project prompt loading and
admitted Room execution-context tools. Session load/save bugs here affect both
the TUI and `ocean-surface` because clients depend on the daemon remembering
transcripts by session id.

## Ownership

- **Scope:** `crates/ocean-agent/`
- **Parent contracts:** `../../AGENTS.md` and `../AGENTS.md`
- **Primary responsibilities:** session persistence, workspace binding, GC, and transcript projection in `src/session/mod.rs`; runtime/history shaping in `src/lib.rs`; system/surface prompt assembly, project instruction discovery, and prompt-memory context in `src/system_prompt.rs`
- `src/memory_tools.rs` owns the shared operator/Room memory factory;
  `src/room_history.rs` and `src/room_resources.rs` own opaque admitted tools and
  interfaces for daemon-owned authority. They do not own daemon admission,
  durable Room stores, HTTP routes or credentials.

## Local Contracts

- Turn-time OAuth refresh performs network work without auth-file custody, then
  fresh-reads under `ocean-providers::lock_auth_file` and merges only the exact
  original provider block/token identity. Removed or changed blocks/files are
  never resurrected; unrelated latest provider edits survive. Blocking custody
  runs off the async worker and retains existing refresh singleflight/cooldowns.
- The shared create-new publisher sets Unix0600 before bytes and distinguishes
  pre-rename failure from published bytes whose directory durability is
  unconfirmed. Never roll back a stale credential snapshot, hold custody across
  network/await, or log provider bodies/tokens; refresh diagnostics use fixed
  class/status only. External CLI writers are not enrolled by this interface.
- The ignored live catalog probe requires `OCEAN_LIVE_MODEL_PROBE=1`; optional
  `OCEAN_MODEL_PROBE_IDS` limits exact ids. `OCEAN_MODEL_PROBE_PROVIDER` tests
  an explicit auth route and requires that exact-id filter. It sends only a
  fixed no-tools prompt,
  bounds each call to 45 seconds, and prints fixed error classes and token counts.
  It never refreshes credentials or constructs sessions/stores. Configured
  credentials alone do not prove entitlement or completed inference.
- A failed, cancelled or timed-out turn reports the usage of the rounds it
  completed, read from the checkpointed assistant messages, with the last
  completed round's request as its context measurement. Those rounds were
  billed and saved. That reading is a floor: tool results saved after it are
  unmeasured. The agent marks it `TokenUsage.context_is_floor`, and the daemon
  labels a marked reading `provider_reported_last_completed_round`.
  Only a turn that fails before any round completes reports zero. Stop-hook
  continuations add their counters and replace the context measurement, which
  describes the latest provider request; a continuation that fails still adds
  the rounds it completed, and its reading stays marked as a floor even though
  the turn itself succeeded. A turn stopped at its turn limit is marked too:
  it ends on a tool round whose results were saved after the measured request.
  The daemon must take the label from that mark, never from the turn's `ok`.
- Preserve session compatibility unless a migration is documented.
- Every advertised catalog model must construct a runtime wire model with the
  same id and limits. Current Opus/Sonnet 5.5 constructors use 1M/128K;
  both Claude Code OAuth and explicit Anthropic routes share those constructors.
  Direct OpenAI API-key GPT-6/5.6 routes construct `openai-responses` models;
  same-route encrypted reasoning survives history shaping. Older OpenAI and
  compatibility routes retain Chat Completions and the existing thinking strip.
  Authored agent model validation checks production routing, not picker
  membership, so older pinned ids remain valid and keyless test routes fail.
- Session-config model pins update model/provider together under the same
  per-session lock as turn persistence and increment the persisted monotonic
  `config_revision`; explicit creation may atomically seed one already-resolved
  model/provider pair at revision one, while legacy session files deserialize
  that revision as zero.
  `SessionModelConfig::model_spec` reconstructs catalog provider/model routes
  for turn resolution so resumed API pins retain their selected auth; custom
  and non-catalog legacy ids keep bare-id behavior.
  Detail and bounded sync projections carry the same revision. Optional config reads must distinguish
  an absent session from unreadable/corrupt storage so daemon adapters map only
  genuine absence to 404.
- Session model-pin provenance uses the existing monotonic `config_revision`:
  a positive revision proves an explicit config mutation, while revision zero
  (new inherited or legacy) retains the former model-difference fallback. The
  shared `SessionModelConfig` resolver is the sole authority for config
  projection and turn selection; global-model equality must not erase a pin.
- Permission-mode persistence atomically writes the authoritative three-state
  file and reports write failures. Load old booleans as automatic/skip-all; the
  legacy `yolo_pref` is a best-effort downgrade mirror, while current boolean
  reads derive from the authoritative mode so the two cannot disagree live.
- Project instruction discovery must respect the repo devlog chain: repo-root `AGENTS.md` is the root contract; `.ocean/AGENTS.md` is only a child doc for `.ocean/` runtime artifacts.
- `agentdir::resolve_snapshot` parses runtime fields only from the caller's
  immutable relative-path byte map and never reopens the live tree. The caller
  owns confined capture and hashes the same bytes it passes to the parser.
- `AgentRuntime::config_dir()` is the read-only daemon authority captured at construction; daemon-owned adapters must use it instead of re-reading process-global config environment during requests.
- Do not add new instruction sources without tests proving ancestor/nested cwd behavior.
- Project ownership resolution must compare canonical roots after the cheap exact
  lookup and when mapping a linked worktree's Git common directory back to its
  main checkout; path aliases such as macOS `/var` and `/private/var` are the same
  authority boundary, not project-less sessions.
- Turn persistence is incremental: save the accepted user message before provider execution, then save only at provider-valid round boundaries where every assistant tool call has its ordered tool result. Never persist an orphan tool-call batch.
- Spawned agent loops must remain owned by the parent turn future. Dropping the parent must abort the child; Tokio's default detached-on-`JoinHandle`-drop behavior is unsafe for side-effecting tools.
- Pre-stream provider failover must pin one session id and hold one per-session turn lock across the complete primary/fallback transaction, reusing the primary attempt's durable accepted-user row; never allow an intervening turn, append the operator prompt twice, or orphan an acceptance-only session.
- Observed primary or alternate provider 401/403 refusals suppress that provider
  as a fallback for 300 seconds in one runtime's clone-shared memory. Filter
  both selection-time and pre-stream fallback and their ready-label projection;
  keep explicit ready-primary attempts, single-alternate bounds and current
  model/catalog resolution. Successful service clears that provider's refusal;
  expiry restores eligibility. Availability, cancellation and other errors do
  not quarantine. New diagnostics carry provider/status/TTL only, no bodies or
  credentials; this memory neither probes nor edits credential files.
- Track-0 room prompt guidance is retired; prompt assembly must not infer a closed room role from agent-turn input.
- Desktop Surface guidance is exclusively `surface-tauri` and uses the shared
  Leptos component contract; do not add parallel desktop prompt families.
- Persisted history search reads only display-projected user/assistant transcript text; it must never inspect tool payloads/raw provider messages or invoke providers/embeddings. Preflight cumulative raw session-file size against the 64 MiB request budget, then enforce the same cumulative bound while reading so concurrent replacement/growth cannot bypass it.
- `PromptControl::without_tools()` is the fail-closed no-capabilities boundary. Empty or unmatched folder-agent allowlists intentionally remain fail-open and must never represent a no-tools posture.
- Room controls carry one exclusive memory mode: operator, disabled, or an
  opaque admitted Room handle. Only `AgentRuntime::admit_room_memory` issues the
  latter from typed admission evidence using the same factory as ordinary
  memory. Room tools remove ambient `retain`/`recall` and inject no operator
  facts; memory partition isolation is store-owned, while execution revocation
  remains daemon-owned.
- An opaque Room memory handle may carry `RoomMemoryAuthority`, attached by
  every daemon admission. Actual `retain`/`recall` invokes it before its mutex
  wait and again under the acquired memory guard immediately before put or each
  recall page, with fixed typed refusals. No Room guard spans the Memory wait;
  daemon admission mints scopes after releasing the Room guard. Observed
  revocation stops subsequent operations; authority validation and memory I/O are separate
  stores and do not cancel already-running synchronous operations. Ordinary
  operator tools and the partition-only constructor remain unchanged.
- History and contributed-folder handles are non-Serde, with fixed Room, Agent
  and binding generation supplied to daemon authority on every call. Model
  arguments cannot select those identities. Folder operations also revalidate
  grant generation and bounded read/list budgets; they never write or execute.
  Backend callbacks own current generation decisions; handles alone do not
  prove revocation through an entire in-flight operation.
- Contributed-folder I/O captures the stored canonical grant root through
  no-follow component handles. Point-in-time `confine` output is a projection,
  not filesystem authority: actual list metadata and regular-file reads use
  those captured descriptors, refuse root/parent/leaf symlink substitutions and
  special files, and fail closed on non-Unix platforms. In-root aliases remain
  compatible only after resolution followed by confined descriptor traversal.
  UTF-8 chunks reject invalid bytes and defer only incomplete trailing sequences
  before EOF; folder text never uses replacement characters for invalid data.
  Read chunk budgets are 4–524,288 bytes (default 65,536): schema and execution
  reject unsupported budgets with `invalid_chunk_budget`, without clamping or
  resolving/accessing contributed-folder files; refusal auditing remains.
  Execution checks the JSON budget before unsigned narrowing: negative,
  fractional, oversized or nonnumeric values use that same typed refusal and
  audit. Omitted or null budgets retain the default.
  Supported continuation offsets advance across complete scalars; EOF
  returns no continuation. Listings require complete UTF-8 entry names and
  refuse invalid bytes with `filename_not_utf8` or names over 255 characters
  with `entry_name_too_long`; never return replacement/truncated names or a
  partial listing on those failures.
- Apply the immutable capability intersection after all ambient tool providers
  are assembled. Reserve `room_history`, `room_list` and `room_read` against
  ambient providers, then append only admitted tools. `without_tools()` wins
  over every handle. Ordinary controls retain operator memory and existing
  prompt bytes unless an explicit Room context is attached.
- These library interfaces do not activate daemon routes or complete the
  production Rooms migration; dependent admission/client parity remains held.
- `PromptControl` receives exactly two effective harness-profile booleans from the daemon: `hashline_edits` and `artifact_spill`. Direct/legacy callers default both off; do not add declarative profile fields here until production runtime composition actually consumes them.
- History shaping preserves stored thinking only when the selected route is exact
  `kimi`/`kimi-k3` (Moonshot requires same-model `reasoning_content` replay) or
  `openai-codex` or current OpenAI GPT-6/5.6 Responses routes (the shared
  Responses encoder replays its own marker-signed encrypted
  reasoning items and MUST receive them back — stripping them degenerates
  gpt-5.x into malformed tool calls across tool rounds). Kimi K2.x and other
  OpenAI-compatible routes retain the existing thinking-strip boundary;
  provider encoders still drop cross-provider thinking.
- Public `SessionTranscriptEntry.text` and persisted history search project
  visible `Content::Text` only. Provider `Thinking` remains in raw persisted
  messages for compatible same-provider replay and never enters display/search
  text.
- The shared session mutation mutex exposes an opaque operation lease for
  daemon admission. Interactive product/legacy/call turns and config/message/
  compact/sync routes acquire it non-blockingly before lifecycle or mutation;
  durable room turns wait on the same lane while the daemon preserves their
  committed trigger ownership and execution permit, before its final registry
  admission/context/footprint, so acknowledged triggers are never dropped. Every leased turn retains the
  lane through persistence and terminal/invalidation publication. Plain runtime
  wrappers remain compatibility callers that acquire the same lane.
- `SessionSyncSnapshot` is projected directly from persisted messages without
  constructing `SessionDetail`: user/assistant visible Text only, fixed image
  placeholder with no metadata, at most 512 rows and 1 MiB text, with explicit
  front-row/text truncation counts. Tool rows, raw messages, tool context,
  provider thinking, image bytes, and MIME metadata never enter this response.
- `compact_session` is owned here: one-shot no-tools model call, atomically
  replaces session transcript with summary + protected recent window. The
  session lock must be held for the entire load-call-save cycle. Only the
  current-runtime model is used; session-historical model is ignored. The
  protected window keeps at most 20 messages and at most 20% of the context
  window (always the newest message) and never begins on an orphan tool
  result. A fully-protected transcript is an `ok:true` no-op with no model
  call. Provider readiness fails closed before the call; the call is bounded
  by the 300-second turn budget; every failure path (not-ready, provider
  error, timeout, empty summary) leaves the stored transcript untouched, and
  corrupt storage is an `Err`, never a wipe.

## Work Guidance

- Issue #29's bounded restore owns only `src/lib.rs` and this contract: private
  refusal memory and filtering over the current provider candidates, primary/
  alternate outcome recording, and synthetic scripted dispatch/clock fixtures.
  No provider/catalog API, session schema, dependency, route or metric changes.
  Root owns actual Cargo, review, reconciliation and hosted builds; source
  implementation does not lift #22 migration or #48 client acceptance holds.
- Filename validation uses the private Unix `validated_entry_name` seam for
  complete CStr UTF-8 and 255-character checks in the descriptor listing.
  Preserve refusal codes, check order,
  raw-name `statat`, ceilings, descriptors and all read behavior. Verify that
  same production function with raw invalid bytes on every Unix; real invalid
  filename listings remain required where creation succeeds. Account only for
  explicit filesystem `EILSEQ` rejection during synthetic creation; other
  creation/listing errors fail, and valid Unicode listing/read tests always run.
- Keep prompt-loading behavior deterministic and easy for cold agents to reason about.
- `src/system_prompt.rs` is one intact cohesion boundary. Prompt wording and literal bytes are behavior; do not mix wording changes with structural extraction.
- `src/session/mod.rs` is the intact persistence boundary. Do not split it or change schema, atomic-save order, duplicate healing, or resume behavior without a separately approved design and compatibility tests.
- Avoid client-specific assumptions; daemon, TUI, and surface clients share this session layer.
- Refresh the recorded `cwd` on every bind; update `workspace_root` and git metadata when the caller moves into a different workspace.
- When changing prompt text, include tests for client-type differences when relevant.
- The TUI fallback/profile guidance must advertise its supported terminal component
  projections and distinguish them from unsupported arbitrary web/HTML layouts;
  never restore a blanket `component_render` ban while the TUI consumes those events.
- Keep the base prompt compact and tool-agnostic: runtime tool schemas describe mechanics; the prompt governs selection, batching, and verification.
- Memory guidance must not encourage unconditional recall. Call `recall` only when prior conversations, preferences, or decisions are needed and not already injected.

## Verification

- Explicit live diagnostic: `OCEAN_LIVE_MODEL_PROBE=1 cargo test -p ocean-agent live_catalog_models_complete_tool_free_prompt --locked -- --ignored --nocapture` (contacts configured providers).

- `cargo test -p ocean-agent every_catalog_model_constructs --locked`
- `cargo test -p ocean-agent every_auth_route_constructs --locked`
- `cargo test -p ocean-agent session_model_spec --locked`
- `cargo test -p ocean-agent production_model_validation --locked`

- `cargo test -p ocean-agent provider_refusal --locked` — synthetic primary and
  alternate 401/403 sequences, later availability failures, exact expiry,
  successful recovery and runtime isolation without sleeps or provider calls.
- `cargo test -p ocean-agent oauth_refresh --locked -- --test-threads=1`
- `cargo test -p ocean-agent system_prompt`
- `cargo test -p ocean-agent session`
- `cargo test -p ocean-agent project_prompt_loads_ocean_agents_md_from_ancestor`
- `cargo test -p ocean-agent room_history --locked`
- `cargo test -p ocean-agent room_resources --locked -- --nocapture` — includes
  raw-name validator proof on every Unix and explicit `EILSEQ` accounting where
  the filesystem refuses synthetic invalid filename creation.
- `cargo test -p ocean-agent memory_tools --locked`
- `cargo test -p ocean-agent agentdir --locked`
- `cargo test -p ocean-agent without_tools_suppresses_combined --locked`
- `cargo test -p ocean-agent`
- `cargo check --workspace`

## Child devlog Index

No child boundaries defined within `ocean-agent/` at this time.
