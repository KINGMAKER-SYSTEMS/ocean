# Ocean Bedrock Epoch 01 — P1 Postgres Auth Registry Evidence

Generated: 2026-06-23T00:55:00Z  
Owner: Navigator

## Implemented

- Added Postgres auth registry migration: `db/005_auth_registry.sql`.
- Added auth-store abstraction: `src/auth-store.mjs`.
- Server auth can now use either:
  - file store: default, current production-compatible behavior;
  - Postgres store: `OCEAN_AUTH_STORE=postgres` / `OCEAN_BEDROCK_AUTH_STORE=postgres` with `DATABASE_URL` or `OCEAN_AUTH_DATABASE_URL`.
- Token and invite endpoints now route through `state.auth` instead of directly reading/writing the JSON auth file.
- `npm run db:migrate` default migration list now includes `db/005_auth_registry.sql`.
- `npm run db:check` reports `authRegistryReady` and auth table presence.

## Production database migration

Applied:

```txt
npm run db:migrate -- --yes db/005_auth_registry.sql
```

Result: migration applied successfully.

`npm run db:check` result summary:

```json
{
  "ok": true,
  "schemaReady": true,
  "authRegistryReady": true,
  "tables": {
    "authTokens": "longhouse.auth_tokens",
    "authInvites": "longhouse.auth_invites"
  }
}
```

## Postgres auth-store smoke

A direct Postgres auth-store smoke created a short-lived invite, redeemed it once, confirmed reuse was blocked, verified the minted token authenticated, revoked the token, and confirmed the revoked token no longer authenticated.

No raw invite code or token value was printed or committed.

Result summary:

```json
{
  "ok": true,
  "invite": "inv_fd79edbc4e99",
  "tokenRevoked": "tok_7a2f06f76e1e",
  "reuseBlocked": true
}
```

## Local regression checks

```txt
node --check src/auth-store.mjs
node --check src/server.mjs
node --check scripts/migrate.mjs
node --check scripts/check-postgres.mjs
npm run smoke
npm run db:check
npm run secret:scan
```

Results: pass.

## Activation note

The code path exists and DB tables are ready, but the server default remains file auth unless `OCEAN_AUTH_STORE=postgres` or `OCEAN_BEDROCK_AUTH_STORE=postgres` is set. This avoids changing production auth authority without an explicit deployment/config cutover.

## Deployment

After operator re-authentication, deployed with:

```txt
railway up --service ocean-bedrock --detach -m "Deploy Postgres auth registry support"
```

Deployment result:

```json
{
  "id": "040ca711-b91d-427e-ba18-95c4a82e01c0",
  "status": "SUCCESS",
  "message": "Deploy Postgres auth registry support"
}
```

Production health passed:

```json
{
  "ok": true,
  "instance": "ocean-bedrock",
  "version": "0.1.0"
}
```

Production auth smoke passed against the default file auth store: token list returned safe public token records, invite create/redeem worked, invite reuse returned 403, and the minted test token was revoked.

```json
{
  "ok": true,
  "authStore": "file",
  "inviteCreated": "inv_2503e93aedff",
  "tokenRevoked": "tok_1299b9bf783d",
  "reuseStatus": 403
}
```

No raw invite code or token value was printed or committed.
