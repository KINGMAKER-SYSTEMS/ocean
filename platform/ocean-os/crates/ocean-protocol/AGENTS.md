# ocean-protocol — Provider Wire Protocol

## Purpose

This crate owns the multi-provider LLM wire protocol layer for Anthropic, OpenAI, Gemini, and OpenAI-compatible providers.

## Ownership

- **Scope:** `crates/ocean-protocol/`
- **Parent contracts:** `../../AGENTS.md` and `../AGENTS.md`
- **Primary responsibilities:** provider request/response translation, streaming protocol handling, provider-specific wire compatibility

## Local Contracts

- Keep provider-specific behavior isolated behind protocol abstractions.
- Do not leak provider quirks into shared `ocean-core` types unless the shared contract intentionally changes.
- Treat streaming event shape changes as compatibility-sensitive.
- OpenAI-compatible and Codex collectors share DSML salvage from
  `src/providers/openai.rs`. Recognize single and doubled Unicode/ASCII bar
  markers; preserve known-leaker gating, structured-call deduplication,
  surrounding prose, truncation recovery and string/JSON argument semantics.
  Empty or structurally broken blocks remain visible text, never tool calls.
- Direct OpenAI API-key GPT-6/5.6 uses `openai-responses` with the shared
  Responses request/stream collector. Bearer API-key requests omit Codex
  originator, beta, version, session and account headers. The API-key route
  replays encrypted reasoning only from its recorded API/provider; foreign thinking
  remains absent. Local HTTP/SSE fixtures verify endpoint, tools and completion.
- Codex OAuth requests using the `codex_cli_rs` originator must carry a current
  `version` header; ChatGPT version-gates newly released Codex models.
- Current Fable/Opus/Sonnet adaptive models use `thinking.type=adaptive` and
  `output_config.effort`, never `budget_tokens`; legacy Off/Minimal requests
  normalize to low only on always-thinking models. Sonnet 5 and Opus 4.8/4.7
  send Off as disabled thinking. Opus 5 accepts `disabled` but then writes
  tool calls into visible text, so Off is low effort there. Sonnet 5.5 Off uses
  `between_tools` at low effort; other settings use adaptive thinking. Opus
  4.8/4.7 belong to the adaptive family (`budget_tokens` is a 400 there) but do
  not think unless asked: with no level chosen they get no `thinking` field.
  No adaptive-family request carries a sampling parameter, with thinking on or
  off. Extended-only models retain their bounded token budgets.
- Adaptive requests send `display: "summarized"`. These models stream empty
  thinking text otherwise, so no client could render reasoning. `between_tools`
  takes no other field, so it never carries `display`.
- An explicit `ThinkingLevel::Off` reaches every encoder (the runtime no longer
  drops it), so each encoder's off shape is live: Anthropic `disabled` or
  `between_tools`, Codex `none`, DeepSeek `disabled`. `None` means the caller
  made no choice and sends the provider default.
- On OpenAI Chat Completions only reasoning families (the o-series, GPT-5 and
  later) receive `reasoning_effort`. Every other id rejects it with a 400, so
  an unrecognized id gets nothing. A Gemini model whose descriptor says it
  does not reason never receives `thinkingConfig`.
- Shared `ThinkingLevel::Max` serializes as max. Current adaptive Claude and
  Codex GPT-6/5.6 encode max directly; older provider vocabularies retain their
  existing bounded highest-effort fallback.
- Codex GPT-6/5.6 preserves xhigh and max. Astra/Sol 6.1 normalize unsupported
  Off/Minimal to low; other GPT-6 models encode Off as none.
- Usage is what the provider reported, mapped onto one shape. Anthropic
  `message_delta` usage is cumulative for the message: replace the running
  counts, never add, and never let a report that omits the input side erase
  what `message_start` said. Every Anthropic usage count tolerates `null`; a
  `message_delta` frame that fails to parse loses the turn's `stop_reason`. A
  Responses round reports usage on `response.completed` and on a length-capped
  `response.incomplete` alike. Gemini output includes thinking tokens, so
  `Usage.reasoning` is a subset of `output` on every provider.
- `Usage.total_tokens` is the provider-authoritative token footprint; never
  reconstruct it by adding cache breakdowns. Anthropic `input` excludes its
  separately reported cache-read and cache-write tokens, so both cache buckets
  are included in its total. Gemini's effective `promptTokenCount` and
  `totalTokenCount` already include cached content; `cachedContentTokenCount`
  is a subset, not an additional count. OpenAI/Codex cached input details are
  likewise breakdowns of provider input/total.
- Anthropic extended-thinking requests must keep `budget_tokens` at least 1024
  and strictly below `max_tokens`; preserve explicit output caps by clamping the
  thinking budget rather than raising the cap.
- Anthropic assistant thinking history is replayable only with a non-empty
  provider signature. Drop unsigned cross-provider reasoning at wire encoding;
  never convert it into visible text or reject the shared persisted schema.
- Anthropic replay must omit empty text blocks, including empty tool-result text; an empty tool result omits optional `content`. Rolling cache breakpoints skip thinking/redacted-thinking blocks and bind to the last cacheable block.
- Codex OAuth turns with a bound Ocean session must use that stable session id
  for both `prompt_cache_key` and the HTTP `session_id`; a fresh UUID is only
  valid for ad-hoc provider calls with no session.
- Codex (Responses API, `store: false`) must round-trip `reasoning` output
  items: always request `include: ["reasoning.encrypted_content"]`, capture each
  item verbatim (minus `status`) behind the `codex-item:` marker in
  `thinking_signature`, and replay it in stream order immediately before its
  paired item. A trailing reasoning item with no follower is dropped (the API
  400s otherwise), and other providers must never forward marker-signed
  thinking. Dropping these items is the documented trigger for gpt-5.x
  degeneration into malformed tool calls (harmony `to=functions.*` leakage,
  token salad in argument strings). Malformed tool-call argument JSON still
  fails open to `{}` but must WARN with the raw payload.
- Dynamic tool declarations are ordered request-only Kimi K3 epochs. Only the
  exact `openai-completions`/`kimi`/`kimi-k3` route on Moonshot's official endpoint
  may encode nonempty groups, as content-less `role: system` messages at validated
  transcript indices; unsupported routes, duplicate tools, unsorted groups, and
  the 16-tool/512-KiB overflow fail closed. Historical epochs remain separate.
  Final synthesis retains them for history validity while emitting
  `tool_choice: "none"`. K3 omits fixed temperature, uses
  `max_completion_tokens`, maps enabled reasoning to `max`, and replays
  `reasoning_content` only from same-provider `kimi-k3` assistant history.
- Retries are operator-visible, not log-only. `with_retry_observed` notifies an
  optional `StreamOptions::retry_observer` before each backoff sleep; providers
  must pass `options.retry_observer` through so a reconnect can reach a surface
  instead of leaving clients on a silent spinner. `with_retry` remains the
  log-only form for call sites with no user-facing stream. The reported `reason`
  is a fixed `RetryReason` vocabulary classified from the error type — never
  provider body text, which is attacker-influenced, unbounded, and fans out to
  every connected client.
- `OCEAN_PROMPT_CAPTURE_DIR` is an opt-in local diagnostics path: capture the
  complete serialized JSON body only (never request headers or endpoint URLs),
  warn-and-continue on capture failures, and retain owner-only permissions
  because request bodies contain private instructions, transcript, and tool data.



## Work Guidance

- Add focused tests or fixtures when changing provider serialization/deserialization.
- Prefer explicit errors for unsupported provider features.
- Coordinate model-routing assumptions with `ocean-providers` when relevant.

## Verification

- `cargo test -p ocean-protocol api_responses --locked`

- `cargo test -p ocean-protocol dsml_salvage --locked`
- `cargo test -p ocean-protocol merge_ --locked`
- `cargo test -p ocean-protocol`
- `cargo check --workspace`

## Child devlog Index

No child boundaries defined within `ocean-protocol/` at this time.
