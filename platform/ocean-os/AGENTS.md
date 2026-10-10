# Ocean OS — Devlog Root

## Purpose

This is the root devlog contract for the `ocean-os` repository. Every agent entering this repo — Claude, Codex, Pi, ocean-native, or any other harness — reads this file first.

## Ownership

- **Repo:** `KINGMAKER-SYSTEMS/ocean`, component `platform/ocean-os/`
- **Runtime:** Rust workspace, daemon on `:4780`, TUI binary `ocean`
- **Connected Ocean repos:** route cross-repo work through `docs/OCEAN_PROJECT_MAP.md`; do not infer ownership from proximity.

## Local Contracts

- Read this file before editing anything in this repo.
- Walk from repo root to each target path and read every `AGENTS.md` along the route.
- Use the nearest `AGENTS.md` as the local contract; parent docs set repo-wide rules.
- No child doc may weaken this root contract.
- Cross-repo routing map: `docs/OCEAN_PROJECT_MAP.md`.
- Canonical workspace package/entry/test index: `crates/AGENTS.md`. Do not maintain a second crate inventory here.
- After any meaningful change, do a devlog pass: update the nearest owning `AGENTS.md`, refresh affected child indexes, remove stale text, and append a root `events.md` entry with `worktree:`.
- Project code is `MIT OR Apache-2.0`, copyright © 2026 Rising Tides. Preserve `NOTICE.md` and local donor notices; Ocean names, logos, and distinctive brand assets remain outside the open-source grants under `TRADEMARKS.md`.
- Rust workspace packages are private distribution inputs, not crates.io releases: `[workspace.package] publish = false`, and every member must inherit it or set `publish = false` explicitly. `cargo xtask docs-check` enforces this for new members.

## Workspace Routing

Core execution flow:

```text
clients (TUI / CLI / ACP / surface)
  -> ocean-daemon (HTTP/SSE authority)
  -> ocean-agent (sessions, prompts, capability assembly)
  -> ocean-runtime (agent loop, permissions, tools)
  -> ocean-protocol + ocean-providers (wire encoding + model/auth routing)
```

Use `crates/AGENTS.md` for all current workspace packages, ownership exclusions, entry points, local contracts, and narrow validation commands.

## Work Guidance

- Current architecture and operations live in `docs/ARCHITECTURE.md` and `docs/OPERATIONS.md`; open work lives in `ROADMAP.md`.
- The active extension architecture and staged migration program is governed by `docs/specs/2026-07-14-ocean-extensions-architecture-and-migration-manifest.md`; Phases 0–1 are accepted. The operator-accepted Phase 6 orchestration-transport ratification is `docs/specs/2026-07-18-ocean-crew-orchestration-and-durable-workflow-manifest.md`; the exact Stage A implementation contract at `docs/specs/2026-07-27-ocean-extension-stage-a-implementation-manifest.md` authorizes only its ordered A1 → A2a → A2b → A3a → A3b → A4 → A5 sequence. A1, A2a, and A2b are accepted. A2b supplies authoritative metadata-only lifecycle publication, synchronous project snapshots and activation epochs, bounded replay/live delivery with fixed ACK state, checked project reconciliation, health/restart/circuit policy, and retained generation-safe cleanup without registry mutation, routes, CLI, or A3+ behavior. The A3a registry writer, including the durable first-Stage-A-publication marker, is present in this monorepo; it does not bind HTTP/CLI mutations or live supervisor reconciliation, and its slice acceptance remains a separate gate. Complete A3b follows A3a and is subject to the two concrete wire-evidence/live-trust contract decisions in [docs/specs/2026-10-03-ocean-extension-a3b-contract-amendment.md](docs/specs/2026-10-03-ocean-extension-a3b-contract-amendment.md). Its proposed amendment does not change the operator-ratified manifest or authorize implementation before those decisions. Crew Stages B–E each require their own implementation manifest and keep extension-provided Longhouse facades over an extension-owned Crew engine, not core.
- The Ocean Observatory program is governed by `docs/specs/2026-07-17-ocean-observatory-architecture.md` with Gate 0 accepted in `docs/specs/2026-07-17-observatory-gate0-decisions.md` (operator ruling: 90s pixel-game visual parity on truthful real events with a durable event store; security invariants unchanged). The operator accepted the Gate 1 implementation manifest at `docs/specs/2026-07-17-observatory-gate1-implementation-manifest.md` on 2026-07-17; tasks 2–9 are authorized under its strict dependency order and stop conditions. Tasks 2–8 are landed; the Task 9 independent review is retained at `docs/specs/2026-07-20-observatory-gate1-task9-independent-review.md` with gating repairs G1–G5 that must land and pass delta review before production Ocean Floor renderer work. Do not build from the Claude Design export code or the raw global agent firehose.
- The Ocean Browser program is governed by `docs/specs/2026-07-19-ocean-webkit-browser-program.md` (operator ruling 2026-07-19: a custom WebKit engine with earned Chrome DevTools protocol parity, built outside the Cargo graph in a dedicated repository). The legacy Chromium backend is quarantined behind the default-off `legacy-chromium` feature; the supervised daemon keeps interim browsing via `ops/install-ocean-daemon.sh`. Acceptance gates are fixed in the manifest; do not ship partial-fidelity network capture as parity.
- The active behavior-neutral daemon refactor is governed by `docs/DAEMON_REFACTOR_MISSION.md` and the supporting code-health plan under `docs/specs/`.
- Optimize for cold-agent discoverability: ownership, entry point, critical invariant, and narrow validation must remain findable from the root, `docs/README.md`, and `crates/AGENTS.md`.
- Behavior-neutral extraction requires a written extraction manifest and must not bundle redesign, protocol changes, renames, or opportunistic fixes.
- Subagent definitions, dispatch, lifecycle, and orchestration policy are extension-owned. Do not add a core daemon/runtime `task`, `spawn_worker`, fleet scheduler, or named-subagent runtime; core may expose only generic permission-gated execution, cancellation, capability-provider, and extension event/tool seams. Existing core subagent-shaped metadata/spec routes are compatibility surfaces pending a separately approved extension migration.
- Once the operator explicitly authorizes an ongoing program, continue through safe approved checkpoints without repeated approval prompts. Close each bounded change with verification, review, commit, upstream reconciliation, and a clean tree; pause only for a concrete blocker or required design decision.
- A feature is not delivered while it exists only in a stash, detached worktree,
  local commit, or unmerged branch. Build and test feature branches normally;
  do not interpret production-deploy provenance rules as permission to skip
  compilation. Close authorized work by reconciling current `origin/main`,
  obtaining fresh review, landing the commit upstream, fast-forwarding the
  canonical checkout, and rebuilding/installing every affected operator-facing
  binary. If an external-action gate blocks landing, report the exact branch,
  commit, remaining command, and blocker; never describe the feature as shipped.
- The Chromium browser backend is quarantined behind the default-off `legacy-chromium` feature (details and validation in `crates/AGENTS.md`) pending the OceanWebKit browser host; default builds must compile no chromiumoxide and must never launch or probe a browser.
- Declared minimum supported Rust is 1.88. Do not lower it without pinning the resolved dependency graph and proving every supported feature; run targeted compatibility checks when changing that contract.
- Build: `cargo build --workspace --release`.
- TUI change: `cargo build -p ocean-tui --release`.
- Daemon restarts: standing authorization to restart from `main`; use specific-PID kill, not blind `pkill`.
- Daemon health: `GET /health` (not `/v1/health`).
- Supervised daemon (`dev.risingtides.ocean-daemon` LaunchAgent): `ops/install-ocean-daemon.sh` requires a fresh successful fetch and clean exact `origin/main` content (detached checkouts supported), builds the existing `legacy-chromium` release with `--locked` and an explicit component target directory, and publishes a non-overwriting full-revision artifact. Installation snapshots the previous selection, copied launcher, plist and command link; restart or expected-revision `/health` + `/ready` failure restores those paths and recovers previously loaded supervision while preserving the original failure status. Failed recovery retains its owner-only backup and reports that recovery is unproven. Artifact retirement follows verified success and preserves the previous selected artifact. An operator-disabled label is rejected before promotion; installation and recovery never write enable/disable overrides. Do not change provider credentials, durable databases or permission posture as part of installation.
- Before daemon installation, coordinate an intake quiet window. The installer observes zero executing turns and no nonterminal requests immediately before promotion; this snapshot does not atomically drain or disable intake. A named-job restart can still interrupt a newly admitted turn. Existing installation commands touch live launchd and must never be used as fixture tests.
- Installer rollback restores release selection and supervision, not durable database contents. Prove compatibility with the currently operated room/session schema before a runtime upgrade; installer fixture success and production-feature compilation alone do not authorize replacing a newer runtime with older store semantics.
- The daemon must run from a neutral cwd (`$HOME` by default), never the repo. Its startup guard rejects repository cwd so unbound fallback turns cannot bind to ocean-os; do not bypass this with `OCEAN_ALLOW_REPO_CWD=1`.
- Sessions live under daemon authority via `ocean-agent`; session bugs break TUI and ocean-surface together.

## Verification

### Fast edit loop

- Run the nearest owning crate's narrow command from `crates/AGENTS.md` or its local contract.
- Daemon installer changes: `bash -n ops/install-ocean-daemon.sh` and `PYTHONDONTWRITEBYTECODE=1 python3 ops/test_install_ocean_daemon.py`. Fixtures redirect only the copied script's home-path token, preserve process `HOME`, and substitute build/network/service commands inside a temporary tree; they never install, read auth, or operate live launchd. A real locked production-feature build and merged-revision runtime readback remain separate release checks.
- For docs-only changes, run `cargo xtask docs-check`; it validates active repo-local Markdown file targets (not heading fragments), archive boundaries, canonical workspace-index parity, and non-default-member rationale.

### Targeted completion checks

- `cargo check --workspace`
- Relevant crate tests; use `cargo check --workspace --tests` when shared enums/events fan out across crates.
- Feature-specific checks named by the owning crate contract.
- Use `cargo xtask ci --compatibility` for changes to supported daemon feature/build contracts.
- Use `cargo +1.88.0 xtask ci --msrv` for changes affecting the declared MSRV; these are local tools, not automatic monorepo CI lanes.

### Merge / PR validation

- Automatic monorepo CI requires only `Build Ocean` and `Build Surface`.
- Run relevant local checks for the actual change. `cargo xtask ci` remains an optional broad developer sweep, not a universal merge prerequisite.
- `cargo xtask ci --dry-run` prints the portable command manifest plus omitted CI-only matrix/setup lanes without executing them.
- Fresh reviewer acknowledgement is required for feature, logic, security, protocol, or architecture changes.

The monorepo root `.github/` owns automatic CI. Do not restore test, lint,
audit, MSRV, documentation or compatibility matrices as merge requirements.

## Child devlog Index

- `.ocean/` — Ocean runtime artifacts, config, and session data → `.ocean/AGENTS.md`
- `crates/` — canonical Rust workspace ownership/entry/test index and crate contracts → `crates/AGENTS.md`
- `docs/` — architecture, operator documentation, active plans, and historical archive policy → `docs/AGENTS.md`
- `integrations/` — distributable adapters for external host extension surfaces → `integrations/AGENTS.md`
- `packaging/` — team distribution: npm wrapper package publishing prebuilt `ocean` + `ocean-daemon` binaries via the active monorepo tag-triggered workflow at `../../.github/workflows/release.yml`; the nested `.github/workflows/release.yml` remains for the original-repository migration → `packaging/AGENTS.md`
- `plugins/` — distributable permission-gated Ocean subprocess tool plugins → `plugins/AGENTS.md`
