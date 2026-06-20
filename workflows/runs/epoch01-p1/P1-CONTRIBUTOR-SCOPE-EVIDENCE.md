# Ocean Bedrock Epoch 01 — P1 Contributor Scope Evidence

Generated: 2026-06-20T05:45:00Z  
Owner: Navigator

## Purpose

Verify the existing contributor token behavior before wider coworker rollout.

## Commands run

Using `https://ocean-bedrock-production.up.railway.app` and the existing local operator contributor token file. Raw token value was not printed.

### Health

```txt
curl /health
```

Result:

```json
{
  "ok": true,
  "instance": "ocean-bedrock",
  "version": "0.1.0"
}
```

### Scoped write under contributor path

```txt
PUT /api/v1/file?path=/coworkers/operator-contributor/p1-scope-check-<timestamp>.md
```

Result: `put ok`

### Contributor delete blocked

```txt
DELETE /api/v1/file?path=/coworkers/operator-contributor/p1-scope-check-<timestamp>.md
```

Result: `403`

### Out-of-scope vault write blocked

```txt
PUT /api/v1/file?path=/vault/p1-scope-check.md
```

Result: `403`

## Verdict

Contributor token behavior matches the coworker rollout guardrail:

- scoped write within `/coworkers/operator-contributor` works,
- delete is blocked,
- out-of-scope `/vault` write is blocked,
- no raw token values were logged.

## Follow-up

A test file may remain under `/coworkers/operator-contributor` because delete is intentionally blocked for contributor tokens. If cleanup is needed, use admin token deliberately and record the action.
