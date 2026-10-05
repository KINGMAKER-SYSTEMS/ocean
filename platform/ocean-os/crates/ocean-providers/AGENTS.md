# ocean-providers — Model and Credential Routing

## Purpose

Own Ocean's provider/model catalog, credential resolution, readiness and shared
auth-file write custody. Provider wire encoding remains in `ocean-protocol`.

## Ownership

- Scope: `crates/ocean-providers/`; parent: `../AGENTS.md`.
- `src/lib.rs` owns routing/catalog and exports the shared custody interface.
- `src/auth_file.rs` owns cooperating Ocean writers' lease and publication.
- `src/native_claude.rs` owns read-only native Claude login discovery.

## Local Contracts

- Preserve forward model/catalog routes and feature credential isolation.
- Public Codex routes include GPT-6.1 Sol, GPT-6 Sol, GPT-6 Luna and GPT-6
  Astra. Claude Opus aliases track 5.5 (1M/128K); explicit 5/4.x ids remain
  routable for pinned sessions. Explicit Anthropic/Claude Code Fable 5.1,
  Opus 5.5 and Sonnet 5.5 routes retain the same 1M/128K limits as the catalog.
  Fable aliases retain Fable 5.1; Sonnet aliases
  track 5.5 (1M/128K), retaining exact older ids for pinned sessions.
- Explicit OpenAI API-key GPT-6/5.6 routes retain API auth and published
  1,050,000/128,000 limits. `openai_uses_responses` is the shared exact-model
  selection predicate for runtime construction and history replay; bare ids
  keep their existing Codex OAuth routing.
- `model_routes` owns provider-qualified picker ids (`provider/model`), wire
  `model_id`, legacy aliases and separate API/subscription choices. Qualified
  selections override ambient `OCEAN_PROVIDER`; bare ids retain prior routing.
  `catalog_model` validates membership without credentials. Readiness is resolved
  per auth route and never inherits a global provider pin.
- `model_effort_levels` lists a level only when choosing it changes the request
  the encoder builds. Levels the encoder folds into another are omitted, `off`
  appears only where thinking can be turned off, and a route whose encoder
  sends no effort parameter (GLM, MiniMax, Kimi K2.x, GPT-4o, Gemini 2.0
  Flash) has an empty list, which clients render as no effort control. Change
  it with the encoder.
- Readiness catalog responses include additive model-specific `effort_levels`;
  advertise max only where Ocean's encoder supports the provider vocabulary.
- Every public catalog id resolves back to the same wire id and provider.
  Credential readiness remains separate from account entitlement and live inference.
- Claude Code credential precedence is explicit environment, valid Ocean OAuth,
  then native login. Default macOS discovery reads `Claude Code-credentials`
  through `/usr/bin/security` with a two-second deadline and 32 KiB cap, then
  checks the native `.claude/.credentials.json`. Native tokens require future
  expiry and `user:inference`; never refresh, import, write, or log them.
  `CLAUDE_CONFIG_DIR` selects only its own credential file and bypasses the
  default keychain. Native subscription auth never supplies Anthropic API keys.
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
