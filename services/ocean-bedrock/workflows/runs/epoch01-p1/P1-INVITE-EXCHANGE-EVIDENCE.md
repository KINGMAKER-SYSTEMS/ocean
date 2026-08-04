# Ocean Bedrock Epoch 01 — P1 Invite Exchange Evidence

Generated: 2026-06-20T05:52:00Z  
Owner: Navigator

## Implemented

- Added invite support to JSON auth registry in `src/auth.mjs`:
  - `createInvite(authFile, options)`
  - `redeemInvite(authFile, code)`
  - `publicInviteRecord(record)`
- Added server routes in `src/server.mjs`:
  - `GET /api/v1/invites` — admin, lists safe invite metadata
  - `POST /api/v1/invites` — admin, creates one-time invite and returns code once
  - `POST /api/v1/invites/redeem` — unauthenticated redemption route that consumes a valid unexpired invite and returns a scoped token once
- Added invite redemption to local app:
  - UI field for invite code
  - `POST /api/invite/redeem` local endpoint
  - redeemed token stored in local config with existing 0600 config behavior
- Updated smoke test to cover invite create, redeem, and reuse rejection.
- Updated coworker/local app docs.
- Added operator CLI: `scripts/issue-invite.mjs` / `npm run invite:create`.

## Safety behavior

- Invite codes are stored hashed using the same sha256 hash format as tokens.
- Invite list responses do not expose hashes or codes.
- Invites are single-use; second redemption returns 403.
- Admin invites are rejected by default by `createInvite` unless explicitly allowed in function options.
- Redemption audit event records metadata only, not code/token value.

## Verification

### Syntax

```txt
node --check src/auth.mjs
node --check src/server.mjs
node --check scripts/ocean-local-app.mjs
node --check scripts/smoke-test.mjs
```

Result: all passed.

### Smoke

```txt
npm run smoke
```

Result:

```txt
ocean-bedrock smoke test passed
```

Smoke now verifies:

- admin can create invite,
- create response returns a one-time code but not invite hash,
- public redemption returns a contributor token,
- invite reuse fails with 403.

## Remaining gaps

- Token/invite registry is still backed by the auth JSON file. Postgres-backed token/invite registry remains a later hardening phase.
- Operator-facing invite creation CLI exists; richer UI/Linear workflow integration can still be added later.
