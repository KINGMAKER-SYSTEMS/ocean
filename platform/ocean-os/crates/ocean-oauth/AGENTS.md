# ocean-oauth — Provider Login Flows Child Doc

## Purpose

This crate owns browser OAuth 2.0 + PKCE login for provider subscriptions: bind a localhost callback server, hand the caller an authorize URL to open, catch the redirect, exchange the code for tokens, and write the credential block into Ocean's auth file.

## Ownership

- **Scope:** `crates/ocean-oauth/`
- **Parent contracts:** `../AGENTS.md` and `../../AGENTS.md`
- **Primary responsibilities:** authorize-URL construction, PKCE/state generation, localhost callback capture, authorization-code token exchange, atomic auth-file block writes.

## Local Contracts

- Flow constants (endpoints, client ids, scopes, callback ports) mirror OMP's working implementation (`@oh-my-pi/pi-ai` `registry/oauth/{anthropic,openai-codex}.ts`). Do not change them without re-verifying against a working client.
- Claude binds port 54545 with ephemeral fallback; Codex is pinned to `http://localhost:1455/auth/callback` — no fallback, since OpenAI validates the registered redirect URI.
- ChatGPT-plan login is a separate Sign in with ChatGPT public-client flow for the public Responses API. First registration uses `dynamic_agent_client`; retain the callback-issued client id, stable `ext_agent_host_id`, validated OIDC subject/id token, and granted scopes in `openai-chatgpt`. Use loopback `127.0.0.1`, PKCE/state/nonce, request `chatgpt.tokens.use.direct`, and fail closed if that scope is absent. Returning sign-in reuses the issued client id and must match the saved verified subject. See [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source/sign-in).
- Written blocks (`claude-code`, `openai-codex`, `openai-chatgpt`) must stay consumable by `ocean-providers` credential resolution AND `ocean-agent::oauth_refresh` (`type:"oauth"`, `access`, `refresh`, `expires` in epoch ms; `accountId` for Codex; issued registration/account metadata for ChatGPT plan).
- This crate performs fresh logins only. Token refresh lives in `ocean-agent::oauth_refresh` / `ocean-protocol::oauth` — never duplicate it here.
- Token endpoints honor the same env overrides as the refresh pass: `OCEAN_OAUTH_ANTHROPIC_TOKEN_URL`, `OCEAN_OAUTH_OPENAI_TOKEN_URL`, `OCEAN_OAUTH_CHATGPT_TOKEN_URL`.
- Auth-file writers fresh-read/merge under `ocean-providers::lock_auth_file` and
  use its guard-bound unique create-new publisher, Unix0600 at creation. Lock
  failure/timeout refuses the write; unrelated provider blocks survive. Login
  completion runs blocking custody off the async worker, after token exchange.
- Pre-rename failures preserve the old file. Post-rename directory-sync failure
  reports published bytes with durability unconfirmed and never rolls back a
  stale snapshot. Shared custody does not enroll external CLI writers.
- Token-exchange HTTP failures report only the fixed provider classification and
  numeric status, never consume/return arbitrary response bodies in diagnostics.
  Successful exchange parsing and public login APIs retain their existing shape.
- Plain API-key writes also serve isolated feature credentials such as `xai`
  and `openai-realtime`; they retain the same atomic 0600 merge contract and
  must never overwrite unrelated agent OAuth or API-key blocks.
- Tests never touch the real `~/.config/ocean-rs/auth.json` and never hit real provider endpoints.

## Work Guidance

- `PublicationFence` retains revocation plus publication custody for one attempt.
  `finish_with_publication` moves the custody guard into the blocking writer;
  aborting its async waiter cannot detach publication from settlement. Callers
  revoke, stop the session task, and settle the retained fence before reporting
  cancellation or removing credentials. Settled successful publication remains
  a succeeded login even when its async waiter was aborted. A cancelled wait may be
  retried on the same fence; revoked queued writers never publish.

- Consumers: `ocean-tui` `/login` (`Action::Login` → `begin`/`finish`) and
  the daemon's operator-only `/v1/auth/providers*` routes
  (`ocean-daemon/src/provider_auth.rs`), which also use `oauth_block_status`
  (token-free block presence/refreshability/expiry) and `logout` (atomic block
  removal under the same custody lease that never creates a missing auth
  file).
- Keep `begin()` non-blocking beyond the port bind; everything slow belongs in `finish()`.

## Verification

- `cargo test -p ocean-oauth`
- `cargo test -p ocean-oauth store --locked -- --test-threads=1`
- `cargo test -p ocean-oauth token_exchange_failures --locked`
- `cargo check --workspace` before merge.

## Child devlog Index

- No child boundaries defined within `ocean-oauth/` at this time.
