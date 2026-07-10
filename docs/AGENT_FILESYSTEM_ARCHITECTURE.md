# The Agent Filesystem — Typed Primitives across the Ocean Quad

> A design record of the "agent as a file system" direction: agents, memory,
> and workflows as **typed Rust primitives** with a deterministic low-level
> processing core, mapped onto the four sibling repos. Captures the ontology,
> the existing-crate grounding, the decisions made, and the build plan.
>
> **Status:** design record (2026-06-25). `BUILT` = verified in source today;
> `PROPOSED` = the gap this direction closes. Every `BUILT` claim cites a real
> crate/file so this stays honest as the code moves.

---

## 0. Thesis

An agent is a **folder** (filesystem-first), and its five constituent primitives
— **model, tools, skills, context, memory** — are **typed Rust contracts**, not
prose. The runtime reads them live (hot-reconfigurable, no rebuild), but their
*shape* is compile-checked and durable. Agents compose into **workflows** (typed
async DAGs, sqlite-backed, durable). All of it is fed by a **low-level
processing core** that parses session JSON deterministically (zero-LLM) and only
spends a cheap model on the residue.

The headline finding: **this pattern is ~60% already built in `ocean-os`.** This
doc exists to name what's there, name what's missing, and sequence the build.

---

## 1. The ontology (the spine)

```text
[model, tools, skills, context, memory]  ──compose──▶  AGENT
                                                          │
                                  ┌───────────────────────┴───────────────────────┐
                                  ▼                                                 ▼
                            ASSISTANT                                          COURIER
                      (interactive; state                                   (non-interactive;
                       determined live)                                       state determined
                                  │                                          by contract)
                                  ▼                                                 ▼
   REQUEST  (operator-invoked, interactive)              CONTRACT  (workflow-invoked, typed obligation)
                                  │                                                 │
                                  └──────────────────────┬──────────────────────────┘
                                                         ▼
                                                    WORKFLOW
                              (scheduled OR interactive; sqlite; typed; tokio;
                               no collisions / unresolved failures; retries + closeouts)
```

- **Agent** = the composition of the five primitives.
- **Assistant vs Courier** = *interactive-ness* + *how state is determined* (live
  vs contractual). Today this is a content-level TOML convention (`class`); the
  goal is a typed runtime distinction.
- **Request vs Contract** = *who invokes* (operator vs workflow) and *what shape*
  (free prompt vs typed input/output obligation).
- **Workflow** = a typed async DAG of agents, durable in sqlite.

---

## 2. The quad — where each piece lives

| Repo | Role | Language | Owns |
| --- | --- | --- | --- |
| **ocean-os** | Runtime + daemon (`:4780`) + **typed engine** | Rust | model, tools, local context, the agent loop, **the workflow executor**, **the memory engine**, the low-level parse core |
| **ocean-bedrock** | Shared knowledge layer — git-backed files + ledger | Node (>=22) | Shared-tier memory (`/docs /context /handoffs /sessions`), semantic search, graph, **workflow specs as data** |
| **ocean-agents** | Agent *instances* (this repo) | TOML + md + py | couriers (`file-courier`), assistants (`content-agent`, `bonzai`), surface profiles |
| **ocean-surface** | View only — PWA + GPUI + VSCode ext | Rust/TS | renders/steers; **no agent logic** |

> `~/ocean-os-repo` is a second clone of `ocean-os` (same GitHub remote
> `Risingtides-dev/ocean-os.git`), **not** a distinct repo.

**Separation principle:** the *execution* primitives (model, tools, the loop,
the typed engine) live in `ocean-os`; the *shared/knowledge* primitives (shared
memory, workflow specs-as-data) live in `ocean-bedrock`; *instances* live in
`ocean-agents`; *views* in `ocean-surface`. They meet at typed boundaries, never
by merging stores.

---

## 3. The five primitives — typed schema vs what exists

### 3.1 `model` — `BUILT` (well-typed)

`ocean-providers` owns it end to end:
`ProviderId`, `ModelSelection`, `ProviderConfig`, `ResolvedCredential`,
`SecretString` (Debug/Display redact), `ProviderReadiness`, `ProviderConfigError`,
`known_models()`, and cross-provider **failover** (OCEAN-275, `ENV_PROVIDER_FALLBACK`).
Providers routed: deepseek / openai / codex / anthropic / minimax / moonshot / google.
Per-turn/per-session model override already exists (`AgentTurnRequest.model_id`).

### 3.2 `tools` — `BUILT` (well-typed)

- `ocean-runtime`: `CapabilityRegistry` + `AgentTool`; built-ins read/write/edit/bash/ls/grep/glob/web_fetch/todo.
- `ocean-plugin`: `trait Plugin: Send + Sync` (`name`/`version`/`list_tools`/`invoke_tool`),
  `PluginManifest` (typed `plugin.toml`), `PluginTool` ("maps 1:1 onto runtime's `AgentTool`"),
  `SubprocessPlugin`, `PluginProvider`. **WIRED** — plugin tools reach the agent as `plugin__<name>__<tool>`.
- `ocean-mcp`: external MCP servers → runtime tools via the `CapabilityProvider` seam.
- `ocean-browser`: CDP Chrome tool suite.

The `Plugin` trait is the **reference for every other typed contract** in this doc:
"subprocess today, WASM tomorrow — same trait."

### 3.3 `skills` — `PARTIAL` (convention, not typed primitive)

Today: `skills/<name>/SKILL.md` + `skill.yaml` (`name` + `description` trigger).
`ocean-os/skills/` ships two (coworker-onboarding, software-factory). The
software-factory skill defines the **tier routing** this design leans on:
cheap-tier (sweeps/scans/discovery), mid-tier (implementation), expensive/opus (judgment).

`ocean-agent/src/agentdir.rs` already discovers `skills/*.md` by filename into an
`AgentDef`. The gap is a **typed `Skill` schema** (trigger semantics, declared
inputs, provenance) following the `AGENTS.md` standardization — authored in
`ocean-bedrock` (shared knowledge) and consumed as typed data by `ocean-os`.

### 3.4 `context` — `BUILT` (two halves)

- **Local turn context:** `cwd`, `workspace_root`, `project_id`, `client_type` on
  `AgentTurnRequest`; surface profiles composed per `client_type`.
- **Codified/handoff context:** `ocean-context` — `Handoff`/`Claim`/`Provenance`/`Anchor`,
  a **claim engine** with provenance, self-versioning history, deterministic
  `derive_confidence`, and `reverify()` (stamps baselines, birth-check, marks
  `Dead`/`Verified`/`Unresolvable`). **This is the trust/provenance layer memory reuses.**

### 3.5 `memory` — `GAP` (the load-bearing new primitive) → §4

The one primitive without a typed home. Sessions live in the daemon; rooms are
durable in `ocean-store`; handoffs are claims in `ocean-context` — but there is
no unified **`Memory`** primitive. This is what `ocean-memory` adds.

---

## 4. The Memory primitive (the heart)

### 4.1 Three scopes

```rust
pub enum MemoryScope {
    /// 1 per agent. Recursive learning — private accumulated experience.
    /// Versioned via MVCC inside sqlite (DECISION: no external git).
    Agent,
    /// Portable, 1 per operator (coworker) — the "dossier." Travels with them;
    /// mounted as a live query target when ported into a room.
    /// DECISION: Hybrid transport — local file canonical, bedrock holds an
    ///           encrypted/replicated copy for cross-machine rooms.
    Operator,
    /// Shared (ocean-bedrock). Git-backed files + ledger. Inferrable by all;
    /// never a raw SELECT into another operator's private rows.
    Shared,
}
```

### 4.2 Two access modes — **this is the security boundary**

```rust
pub enum MemoryAccess {
    /// Read summaries, handoffs, semantic search — *infer* about others.
    /// No raw SELECT. Default for Shared + for any operator db not granted to you.
    Infer,
    /// A portable Operator db ported into THIS room/session: mounted live.
    /// Direct SELECT. Granted only by ownership or by room convene (consent).
    Query,
}
```

| Memory | In your own chats | In a room with that operator |
|---|---|---|
| **your own** Operator db | `Query` | `Query` |
| **another** operator's db | `Infer` only | **`Query`** — elevated because they ported it in (consent) |
| **Shared** (bedrock) | `Infer` | `Infer` |

A room convene is therefore a **memory-mount event**: coworker B ports their
dossier in → every agent in that room gains `Query` for the session's lifetime.
"all operators port in to any ocean session's agent turn as queryable db."

### 4.3 A memory row == an attested claim (reuse `ocean-context`)

```rust
pub struct Memory {
    pub id: MemoryId,
    pub scope: MemoryScope,
    pub owner: PrincipalId,            // which agent / operator owns it
    pub kind: MemoryKind,              // Fact | Preference | Relationship | Event | Skill
    pub body: serde_json::Value,       // the queryable payload
    pub provenance: Provenance,        // reused from ocean-context::claim
    pub trust: TrustState,             // Verified | Stale | Dead — reverify() mutates on load
    pub anchors: Vec<Anchor>,          // git anchors; reverify flags drift
}
```

### 4.4 Storage — reuse the `ocean-store` template

`ocean-memory`'s `SqliteMemoryStore` mirrors `ocean-store`/`SqliteRoomStore`
(the battle-tested pattern): **sync** `rusqlite` (bundled), `&mut self` behind
`std::sync::Mutex` (guard dropped before any `.await`), **IMMEDIATE** transactions
for seq/write atomicity, monotonic seq, **soft-delete** for audit. MVCC (DECISION)
= a versions table + anchors inside sqlite, no external git per agent; `reverify`
still runs against ground truth on load.

### 4.5 Decisions (resolved 2026-06-25)

- **Agent-memory versioning → MVCC inside sqlite.** No external git per agent.
  Versions table + anchors; `ocean-context::reverify` still validates drift against
  current ground truth. Git remains a **bedrock/shared** concern only.
- **Operator-db transport → Hybrid.** Local file is canonical (attached to a turn
  like a credential); `ocean-bedrock` holds an **encrypted/replicated** copy so a
  portable dossier reaches rooms across machines.

---

## 5. The low-level processing core (daemon-side, cheap)

> The user insight: *"sessions are just JSON, so what stops a parser + a cheap
> model that analyzes chats at the daemon?"* Answer: nothing — and the parser
> half **already exists, zero-LLM.**

### 5.1 The pipeline

```
session JSON (daemon-owned; ocean-core types, SSE data lines)
   │
   ▼  ①  PROGRAMMATIC PARSE — deterministic, ZERO LLM   [ocean-context::extract]
   │     extract_claims(text, ctx) → Vec<Claim>          regex anchors/tickets/symbols
   │     "Faithful port of the validated Python prototype (51-claim corpus)"
   │     derives confidence from anchor richness; tags anchors/symbols/lines
   ▼  ②  RESIDUE → CHEAP MODEL — only what ① couldn't structure   [ocean-providers]
   │     routed to cheap-tier (the software-factory tier table); tiny judgment pass
   ▼  ③  WRITE — typed Memory rows, MVCC sqlite, IMMEDIATE tx   [ocean-memory]
   │     provenance + anchors attached; confidence from ①
   ▼  ④  TRIGGER — low-level, background   [ocean-hooks]
        HookEvent::Stop today → extend to HookEvent::MemoryIngest (or a new event)
        NOT a foreground agentic turn — recursive learning is a background process
```

### 5.2 Why this is the beautiful part

- **Deterministic-first.** The parser (①) is regex/tree-sitter, reproducible, free,
  and the thing that stamps the trustable anchors/symbols. The model (②) only
  touches freeform residue — so the "analyze chats" cost is tiny and bounded.
- **Low-level.** It runs in the daemon as a hook, not as an expensive reasoning
  turn. Learning accrues continuously without taxing foreground work.
- **Already grounded.** ① = `ocean-context/src/extract.rs`; ② = `ocean-providers`
  cheap-tier routing; ③ = `ocean-memory` (§4); ④ = `ocean-hooks`. The only net-new
  glue is the **ingest bridge** that wires ①→②→③ and fires on ④.

### 5.3 `ocean-context::extract` — the existing parser (verified)

`extract.rs`: *"Pass-1 claim extraction: deterministically pull anchored claims
out of prose HANDOFF.md docs. **Zero LLM.**"* Ships `extract_claims(text, &ExtractCtx)`,
`pair_symbols` (proximity-based symbol↔anchor pairing, not blind zip),
ticket/symbol/anchor regexes. Output is typed `Claim`s ready to become `Memory` rows.

---

## 6. Workflows — spec-as-data, engine-in-Rust

### 6.1 The reconciliation (forced by what exists)

`ocean-bedrock/workflows/` already ships typed specs: `.workflow.json` +
`.workflow.ts` (`ocean-bedrock-week-one`, `ocean-triage-scheduler`, `agents/sonar…navigator-p0`)
and a `runs/` ledger. **This does not conflict with "workflows = typed Rust + sqlite."**
It separates **spec from engine**:

- **Spec = portable data.** Authored anywhere; bedrock is a fine home. A workflow
  spec is JSON/TOML describing phases, steps, model roster, boundaries.
- **Engine = typed Rust, in `ocean-os`.** Interprets the spec with **durable
  execution**: sqlite run-state, tokio, idempotent retries, closeouts.
- Bedrock's TS runner becomes a **client** of the Rust engine (or retires).

### 6.2 Contract primitive

```rust
#[async_trait]
pub trait Workflow: Send + Sync {
    fn spec(&self) -> &WorkflowSpec;                 // the parsed spec (data)
    fn steps(&self) -> &[Step];                      // typed DAG
    async fn run(&self, ctx: WorkflowCtx) -> Result<WorkflowOutput>;
}

/// A Contract = a typed input/output obligation a workflow step honors.
/// Replaces free-form "prompt in, text out" for workflow-invoked steps.
pub struct Contract {
    pub input_schema:  Value,   // JSON Schema — what this step requires
    pub output_schema: Value,   // JSON Schema — what it guarantees
    pub retry: RetryPolicy,     // idempotent; provenance lets you detect "already applied"
    pub closeout: CloseoutPolicy, // terminal-failure handling
}
```

### 6.3 Durability guarantees (the user's "no collisions / unresolved failures")

Inherited from the `ocean-store` pattern: every step's memory/state mutation runs
in a single **IMMEDIATE** sqlite transaction — a partial write is never observable,
a seq never reuses. Retries are **idempotent** (the `Memory` provenance/sig-hash
detects "already applied"). Closeouts are explicit terminal states (soft-close for
audit, like `RoomStore::close`). Workflow run-state lives in sqlite; the spec stays
data — so a crashed daemon resumes mid-workflow from the last committed step.

---

## 7. Agent classification + invocation (typed)

```rust
pub trait Agent: Send + Sync {
    fn manifest(&self) -> &AgentManifest;          // typed identity (== agent.toml today)
    fn primitives(&self) -> &AgentPrimitives;       // model/tools/skills/context/memory bindings
    async fn turn(&self, req: TurnRequest) -> Result<TurnOutcome>;
}

pub trait Assistant: Agent { /* interactive; state determined live */ }
pub trait Courier:    Agent { /* non-interactive; state determined by Contract */ }
```

- **Request** (interactive, operator) → `AgentTurnRequest` (`BUILT`, `POST /v1/agent/turns`).
- **Contract** (workflow-invoked) → the typed obligation above (`PROPOSED`).

This **already exists in embryonic form**: `ocean-agent/src/agentdir.rs` resolves
`agents/<name>/agent.toml` into an `AgentDef` (model/description/tools/permissions
+ instructions + skills + tools + subagents). The typed `Agent`/`Assistant`/`Courier`
traits lift that folder-convention into runtime contracts.

---

## 8. What exists vs what to build

| Capability | Status | Where |
|---|---|---|
| Agent-as-folder (filesystem-first, hot-read) | `BUILT` | `ocean-agent/agentdir.rs` |
| Typed `model` (routing, failover, readiness) | `BUILT` | `ocean-providers` |
| Typed `tools` (Plugin trait, MCP, browser) | `BUILT` | `ocean-plugin`/`ocean-runtime`/`ocean-mcp` |
| Deterministic claim parse (zero-LLM) | `BUILT` | `ocean-context/extract.rs` |
| Claim trust/provenance/reverify | `BUILT` | `ocean-context` |
| sqlite durable-store template | `BUILT` | `ocean-store` |
| Shared knowledge layer (inferrable) | `BUILT` | `ocean-bedrock` |
| Rooms convene / auto-wake | `BUILT` | `ocean-agent/rooms.rs` + daemon |
| Lifecycle hook primitive | `BUILT` (Stop only) | `ocean-hooks` |
| Workflow specs as data | `BUILT` | `ocean-bedrock/workflows/` |
| Typed wire vocab (turns/events) | `BUILT` | `ocean-agent-sdk` |
| **`Memory` primitive** (3 scopes, 2 access, MVCC) | `PROPOSED` | **new `ocean-memory`** |
| **Ingest bridge** (extract→cheap model→sqlite, on hook) | `PROPOSED` | daemon + `ocean-memory` |
| **Portable-db mount** (`Infer`→`Query` room elevation) | `PROPOSED` | `ocean-agent/rooms.rs` |
| **Typed `Agent`/`Assistant`/`Courier` traits** | `PROPOSED` | `ocean-agent` (above `AgentDef`) |
| **Typed `Workflow` engine + `Contract`** | `PROPOSED` | new crate in `ocean-os` |
| **`MemoryIngest` lifecycle event** | `PROPOSED` | `ocean-hooks` |
| Typed `Skill` schema (AGENTS.md-std) | `PROPOSED` | bedrock-authored, os-consumed |

---

## 9. Build sequence

There are two build tracks:

- **Runtime foundation** in `ocean-os`: typed primitives, stores, dispatch, and
  engines.
- **Agent package work** in `ocean-agents`: the filesystem artifacts the runtime
  consumes -- manifests, profiles, SOPs, tool grants, contracts, and fixtures.

This repo should not grow a second runtime. Its end-state is the pure-content
shape described in [`PYTHON_TO_RUST_MIGRATION.md`](./PYTHON_TO_RUST_MIGRATION.md):
executable bridge/router/harness code moves to Rust only after each replacement is
verified.

### 9.1 Runtime foundation (`ocean-os`)

1. **`ocean-memory`** (foundation — everything depends on it): `Memory` schema,
   `MemoryScope`/`MemoryAccess`/`MemoryKind`, `SqliteMemoryStore` (Agent+Operator
   scope, MVCC versions table), reuse `ocean-context` provenance/trust. Fixtures:
   parse the existing handoff corpus into memory rows.
2. **Ingest bridge:** `ocean-context::extract` → cheap-model residue → sqlite
   `Memory` rows; fire on `ocean-hooks` (extend `HookEvent`).
3. **Portable-db mount:** `mount_portable_memory()` on rooms → `Infer`→`Query`
   elevation; hybrid transport (local canonical + bedrock encrypted replica).
4. **Typed agent traits:** `Agent`/`Assistant`/`Courier` over the existing
   `AgentDef`/`AgentConfig` — classify by interactive-ness + state model.
5. **Workflow engine:** typed `Workflow`/`Step`/`Contract`, sqlite run-state,
   idempotent retries, closeouts; consumes bedrock specs as data.

Each step is independently shippable and lands in `ocean-os` without disturbing a
running assistant.

### 9.2 Agent package build list (`ocean-agents`)

The codified source of truth for the package-side build list is
[`ocean-agents-builds.toml`](./ocean-agents-builds.toml). Update that register
first; this table is the human-readable mirror.

**Reality check:** `ocean-agents` currently has **zero primitive-native builds**.
The useful things in this repo today are filesystem conventions, authored
profiles/manifests/SOPs, and transitional Python harnesses. The real typed
primitive implementation and enforcement happen in `ocean-os`.

| Build | Status | Owns | Acceptance |
|---|---|---|---|
| **A1. Typed package manifests** | `FILESYSTEM_CONVENTION` | `couriers/*/courier.toml`, `assistants/*/*.toml` | Every assistant/courier declares `class`, `mode`, surfaces/routes, `cwd`, profile path, tool grants, and workflow-contract hooks in a schema the Rust runtime can parse without Python discovery. |
| **A2. Surface profile catalog** | `FILESYSTEM_CONVENTION` | `assistants/_shared/`, `assistants/_base/<SURFACE>/`, generated `assistants/<SURFACE>/system.md` | Every authored surface has source files plus a non-empty generated profile; `make assistants-check` proves no drift. New surfaces are added as data, not daemon rebuilds. |
| **A3. Named assistant packages** | `FILESYSTEM_CONVENTION` | `assistants/bonzai/`, `assistants/content-agent/` | `bonzai` remains a complete git-hygiene specialist; `content-agent` graduates from scaffold to live Slack assistant with generate/status/gallery/canvas flows grounded in its manifest, tools, and SOPs. |
| **A4. Courier package catalog** | `FILESYSTEM_CONVENTION` | `couriers/<courier>/courier.toml`, courier protocol docs, route contracts | `file-courier` stays the reference; each new courier is a folder with a manifest, destination-confirmation rules, input/output contract, and no router edit. |
| **A5. Slack bridge validation pack** | `TRANSITIONAL_HARNESS` | `assistants/bridge/`, `couriers/transport/slack.py` until `ocean-slack` lands | The current Python bridge remains the live parity harness: inbound mention/DM -> daemon turn -> in-thread reply, plus canvas-op fulfillment once the daemon emits the SSE event. It is retired only after the Rust path passes the same tests live. |
| **A6. Tool/SOP/skill declarations** | `FILESYSTEM_CONVENTION` | `assistants/*/tools.toml`, `assistants/*/sops/`, future `skills/<name>/` | Tool grants and SOPs are explicit filesystem data. When the typed `Skill` schema lands, each reusable SOP becomes a declared skill with trigger semantics, inputs, and provenance. |
| **A7. Memory mount declarations** | `PROPOSED` | future manifest stanzas, not local sqlite stores | Agent packages declare the memory scopes they expect (`Agent`, `Operator`, `Shared`) and the access mode (`Infer`/`Query`) they are allowed to request. Actual storage remains `ocean-memory`. |
| **A8. Workflow contract fixtures** | `PROPOSED` | per-agent/per-courier JSON Schema fixtures or manifest stanzas | Workflow-invoked steps can call a courier or assistant through a typed `Contract` instead of a free prompt; fixtures are testable without a live workflow run. |
| **A9. Verification gates** | `TRANSITIONAL_HARNESS` | `Makefile`, bridge tests, router list checks, future manifest linter | Existing checks stay green (`make assistants-check`, bridge unit tests). Add a manifest/schema linter before deleting Python discovery. |
| **A10. Pure-content cutover** | `PROPOSED` | deletion gates tracked by `PYTHON_TO_RUST_MIGRATION.md` | After Rust replacements are live, remove Python bridge/router/harness code and leave this repo as manifests, profiles, skills, SOPs, and test fixtures only. |
| **A11. Bedrock context mounts** | `PROPOSED` | `assistants/*/context.toml`, future `[context]` manifest stanzas | Every agent declares the bedrock namespaces it mounts (`/context`, `/handoffs`, `/docs`, prompt-library, ledger); the runtime resolves the mounts at turn time and injects the results as first-class context. Knowledge access is a manifest fact, not prompt prose. |
| **A12. Automation-agent retyping** | `PROPOSED` | `couriers/*` migration, `agent.toml` class + contract stanzas | The courier framing is retired: `/ship`/`/say`/`/resolve` become tool grants; each automation agent declares a typed `Contract` invoked by workflows through the daemon. No new couriers after this lands. |
| **A13. Knowledge-parity gate** | `PROPOSED` | per-agent knowledge fixture suites, future Makefile gate | Each deployed agent must answer a company-operations fixture suite using ONLY its declared bedrock mounts + accumulated memory (answers absent from `system.md`). No agent registers as deployed until its suite passes. |

> **Knowledge-first repositioning (decided 2026-07-10).** `ocean-agents` is the
> deployment layer for Rising Tides' agents: interactive **assistants** and
> workflow-invoked **automation agents**. The differentiator is baked-in company
> knowledge — bedrock mounts resolved by the runtime at turn time (A11) plus
> continuously accumulated typed memory (R1/R2) — never hand-maintained prompt
> prose. `content-agent` (prompt library + canon SOPs + bedrock publishing) is
> the reference package pattern. North star: staff interacting with any deployed
> agent find it already knows the company's operational underpinnings.

Dependency order from the package side:

1. Lock A1 (manifest shape) enough that both couriers and assistants describe the
   same five primitives.
2. Keep A2 green while adding or changing surfaces; profile drift is a runtime
   bug because the daemon loads the generated file.
3. Finish A3/A4 as real packages before pushing them into workflow automation.
4. Use A5 as the parity harness while Rust absorbs Slack I/O and dispatch.
5. Layer A6/A7/A8 once the runtime has typed `Skill`, `Memory`, and `Contract`
   readers.
6. Do A10 last; no Python deletion before a verified Rust replacement.
7. A11 (bedrock mounts) can start as manifest data now and go live when the
   runtime grows a `ContextMount` reader; A12 waits on typed `Contract` (R5);
   A13 gates every deployment once A11 mounts resolve live.

---

## 10. Decisions log

- **Agent-memory versioning → MVCC inside sqlite** (no per-agent git repo, no
  project-repo branch). Versions table + anchors; reverify still validates drift.
- **Operator-db transport → Hybrid** (local file canonical; bedrock encrypted
  replica for cross-machine rooms).
- **Spec vs engine split:** workflow/memory *specs* are portable data (bedrock);
  the *typed engine* is Rust (ocean-os).
- **Prose stays for judgment; types own the spine.** The system stays
  hot-reconfigurable — types are the durable contract, prose is the filler.
