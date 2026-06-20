# Ocean Bedrock Epoch 01 — P1 Deploy Evidence

Generated: 2026-06-20T05:55:00Z  
Owner: Navigator

## Deployment

Railway service: `ocean-bedrock`  
Project: `rising-tides-agents`  
Environment: `production`  
Deployment ID: `c0ec37fe-f874-4bb8-aa99-d3cc38432819`  
Message: `Deploy Epoch 01 P1 coworker onboarding`

Status from `railway deployment list --service ocean-bedrock --json`: `SUCCESS`

## Live verification

### Health

```txt
curl https://ocean-bedrock-production.up.railway.app/health
```

Result summary:

```json
{
  "ok": true,
  "instance": "ocean-bedrock",
  "version": "0.1.0"
}
```

### Invite exchange smoke against production

Performed a short-TTL production invite smoke using the admin token without printing raw code/token values:

1. `POST /api/v1/invites` created invite `inv_1557dc2b870f` for `p1-deploy-smoke` scoped to `/context/ocean-bedrock`.
2. `POST /api/v1/invites/redeem` redeemed it and minted contributor token `tok_f59382e59576`.
3. Reuse attempt returned `403`.
4. Minted test token was revoked via `DELETE /api/v1/tokens/tok_f59382e59576`.

Result summary:

```json
{
  "ok": true,
  "inviteCreated": "inv_1557dc2b870f",
  "tokenRevoked": "tok_f59382e59576",
  "reuseStatus": 403
}
```

No raw invite code or token value was printed or committed.

## Pre-deploy checks run locally

- `npm run secret:scan` — pass
- `npm run secret:globs:check` — pass
- `npm run smoke` — pass
- `npm run db:check` — pass
