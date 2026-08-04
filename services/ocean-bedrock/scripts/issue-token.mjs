#!/usr/bin/env node
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { addToken, loadAuthFile, publicTokenRecord } from '../src/auth.mjs';

// Append a token-lifecycle event to the bedrock audit log so a token issued via
// this CLI leaves the same trail as the HTTP route (which audits `token.create`)
// and the README's "audit log for token changes" promise holds for the
// documented onboarding path. The audit file is a sibling of the auth file (the
// server's metaRoot layout), honoring the same env override. File-append only
// and fail-soft — the ledger half of the server's appendAudit needs a live DB,
// and an audit failure must never sink a successful token issuance.
async function appendTokenAudit(authFile, event) {
  const auditFile = path.resolve(
    process.env.OCEAN_BEDROCK_AUDIT_FILE
      || process.env.RT_NAS_AUDIT_FILE
      || path.join(path.dirname(authFile), 'audit.jsonl'),
  );
  try {
    await fs.mkdir(path.dirname(auditFile), { recursive: true });
    await fs.appendFile(auditFile, `${JSON.stringify({ ts: new Date().toISOString(), ...event })}\n`);
  } catch (error) {
    console.error('[ocean-bedrock] failed to write token audit event:', error.message);
  }
}

function usage() {
  console.log(`Usage:
  npm run token:create -- --name <name> [--role agent|readonly|readwrite|admin] [--scope /path] [--expires-at ISO]
  npm run token:create -- --list

Options:
  --name <name>          Human/agent name for the token.
  --role <role>          readonly, contributor, agent, readwrite, or admin. Default: contributor.
  --scope <path>         Path scope. Can be repeated. Default: /.
  --token <secret>       Use a provided secret instead of generating one.
  --expires-at <iso>     Expiration timestamp.
  --auth-file <path>     Token registry path. Default: data/.ocean-bedrock/tokens.json.
  --root <path>          Root data directory used to infer auth-file.
  --list                 List token records without secrets.
`);
}

function parseArgs(argv) {
  const args = { scopes: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--list') args.list = true;
    else if (arg === '--name') args.name = argv[++i];
    else if (arg === '--role') args.role = argv[++i];
    else if (arg === '--scope') args.scopes.push(argv[++i]);
    else if (arg === '--token') args.token = argv[++i];
    else if (arg === '--expires-at') args.expiresAt = argv[++i];
    else if (arg === '--auth-file') args.authFile = argv[++i];
    else if (arg === '--root') args.root = argv[++i];
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  usage();
  process.exit(0);
}

const root = path.resolve(args.root || process.env.OCEAN_BEDROCK_ROOT || process.env.RT_NAS_ROOT || path.join(process.cwd(), 'data'));
const authFile = path.resolve(args.authFile || process.env.OCEAN_BEDROCK_AUTH_FILE || process.env.RT_NAS_AUTH_FILE || path.join(root, '.ocean-bedrock', 'tokens.json'));

if (args.list) {
  const auth = await loadAuthFile(authFile);
  console.log(JSON.stringify({ authFile, tokens: auth.tokens.map(publicTokenRecord) }, null, 2));
  process.exit(0);
}

if (!args.name) {
  usage();
  console.error('\nMissing required --name.');
  process.exit(1);
}

const role = args.role || 'contributor';
const scopes = args.scopes.length ? args.scopes : ['/'];
const created = await addToken(authFile, {
  name: args.name,
  role,
  scopes,
  token: args.token,
  expiresAt: args.expiresAt || null,
  createdBy: 'local-cli',
});

await appendTokenAudit(authFile, {
  action: 'token.create',
  actor: 'local-cli',
  name: args.name,
  role,
  scopes,
  tokenId: created.record?.id || null,
});

console.log(JSON.stringify({ authFile, ...created }, null, 2));
console.error('\nSave the token now. It is only printed once.');
