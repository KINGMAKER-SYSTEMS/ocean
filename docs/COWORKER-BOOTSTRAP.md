# Coworker Bootstrap

This is the coworker-facing flow for feeding selected local folders into the Ocean knowledge layer.

You choose exactly which folders are active. The bootstrapper saves a local config, then the ingest command uploads changed files to the shared Bedrock/Longhouse HTTP service using your scoped MCP/Bearer token.

## What you need

- Node.js 22+
- Bedrock/Longhouse URL
- Your one-time invite code, or a scoped MCP/Bearer token fallback
- The folders you want to feed, e.g. `~/Documents/RisingTides`, `~/Projects/ocean-notes`

## Bootstrap

Interactive:

```bash
npm run ocean:bootstrap
```

Non-interactive:

```bash
npm run ocean:bootstrap -- \
  --server https://bedrock.example.com \
  --token '<MCP_TOKEN>' \
  --name alice \
  --folder ~/Documents/RisingTides \
  --folder ~/Projects/ocean-notes \
  --yes
```

This creates:

```txt
~/.config/ocean-bedrock/bootstrap.json
~/.config/ocean-bedrock/env
```

The token is stored locally with file mode `0600`. Do not commit or share this file.

For GUI onboarding, prefer a one-time invite code. The local app redeems the code with `/api/v1/invites/redeem`, stores the returned scoped token locally, and the invite cannot be reused.

## Ingest selected folders

Dry run first:

```bash
npm run ocean:ingest -- --dry-run
```

Upload changed files:

```bash
npm run ocean:ingest
```

Only changed files are uploaded after the first run. Local state is stored at:

```txt
~/.local/state/ocean-bedrock/ingest-state.json
```

## Default include/skip behavior

By default, ingest includes common knowledge/document/code files:

```txt
.md .txt .json .yaml .csv .html .js .ts .py .rs .go .sql .pdf .docx .pptx .xlsx ...
```

It skips common noisy folders/files:

```txt
.git node_modules .venv target dist build .next .cache __pycache__ .DS_Store
```

It also blocks common secret-bearing paths by default, including:

```txt
.env .env.* *.pem *.key *.p12 *.pfx id_rsa id_dsa id_ecdsa id_ed25519
.aws/ .ssh/ .gnupg/ .npmrc .pypirc .netrc .htpasswd *.tfstate *.kdbx
*secret* *credential* service-account*.json
```

Dry-run output includes `skippedByReason` and, for dry-run/report mode, `skippedFiles` with paths and matched patterns. It reports paths/reasons only, never file contents.

Max file size defaults to `10 MB`.

To feed all non-secret file types under the max size:

```bash
npm run ocean:bootstrap -- --all-files ...
# or just for one ingest run:
npm run ocean:ingest -- --all
```

`--all` does **not** include secret-pattern files. Secret-pattern files require the explicit supervised override:

```bash
npm run ocean:ingest -- --include-secrets
```

Do not use `--include-secrets` for coworker rollout unless a human operator has approved the exact folder and paths.

## Where your files land

Your files are mapped to:

```txt
/coworkers/<your-name>/<device>/<folder-label>/...
```

A redacted source registration is written to:

```txt
/context/ocean-bedrock/sources/<your-name>-<device>.json
```

Each ingest run writes a manifest to:

```txt
/context/ocean-bedrock/manifests/...
```

## Security notes

- Only choose folders you want shared with the company knowledge layer.
- Do not select password manager exports, SSH key folders, raw credential folders, or private personal directories.
- Your token should be scoped; it should not be an admin token.
- Standard coworker scopes should be limited to `/coworkers/<name>/<device>`, `/sessions/<name>`, and `/context/ocean-bedrock` unless an operator explicitly grants more.
- Contributor tokens can read/write/lock within scope, but should not be able to delete.
- If a token leaks, ask an admin to revoke it and issue a new one.
