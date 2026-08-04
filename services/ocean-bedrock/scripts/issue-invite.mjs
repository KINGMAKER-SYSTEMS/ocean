#!/usr/bin/env node
import fs from 'node:fs/promises';

function parseArgs(argv) {
  const args = { scopes: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--url') args.url = argv[++i];
    else if (arg === '--admin-token') args.adminToken = argv[++i];
    else if (arg === '--admin-token-file') args.adminTokenFile = argv[++i];
    else if (arg === '--name') args.name = argv[++i];
    else if (arg === '--role') args.role = argv[++i];
    else if (arg === '--scope') args.scopes.push(argv[++i]);
    else if (arg === '--ttl-minutes') args.ttlMinutes = Number(argv[++i]);
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function usage() {
  console.log(`Usage:
  npm run invite:create -- --name alice --scope /coworkers/alice --scope /sessions/alice --scope /context/ocean-bedrock

Options:
  --url <url>                Ocean Bedrock URL. Defaults to OCEAN_BEDROCK_URL or production URL.
  --admin-token <token>      Admin bearer token. Prefer --admin-token-file.
  --admin-token-file <path>  File containing admin token. Defaults to /root/.config/ocean-bedrock/railway-admin-token.txt when present.
  --name <name>              Invite/token name. Required.
  --role <role>              Role. Default: contributor.
  --scope <path>             Token scope. Repeatable. Default: /coworkers/<name>, /sessions/<name>, /context/ocean-bedrock.
  --ttl-minutes <n>          Invite TTL. Default: 60.
`);
}

function slugify(value) {
  return String(value || '').trim().toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'coworker';
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  usage();
  process.exit(0);
}
if (!args.name) {
  usage();
  throw new Error('--name is required');
}

const url = String(args.url || process.env.OCEAN_BEDROCK_URL || 'https://ocean-bedrock-production.up.railway.app').replace(/\/$/, '');
const tokenFile = args.adminTokenFile || process.env.OCEAN_BEDROCK_ADMIN_TOKEN_FILE || '/root/.config/ocean-bedrock/railway-admin-token.txt';
const adminToken = args.adminToken || (await fs.readFile(tokenFile, 'utf8').then((value) => value.trim()).catch(() => ''));
if (!adminToken) throw new Error('Missing admin token. Pass --admin-token-file or --admin-token.');

const owner = slugify(args.name);
const scopes = args.scopes.length ? args.scopes : [`/coworkers/${owner}`, `/sessions/${owner}`, '/context/ocean-bedrock'];
const response = await fetch(`${url}/api/v1/invites`, {
  method: 'POST',
  headers: {
    Authorization: `Bearer ${adminToken}`,
    'content-type': 'application/json',
  },
  body: JSON.stringify({
    name: args.name,
    role: args.role || 'contributor',
    scopes,
    ttlMinutes: args.ttlMinutes || 60,
  }),
});
const body = await response.json();
if (!response.ok) throw new Error(`Invite create failed: ${response.status} ${JSON.stringify(body)}`);

console.log(JSON.stringify({
  ok: true,
  url,
  invite: body.invite,
  code: body.code,
  note: 'Show this code once to the coworker. It is single-use and expires.',
}, null, 2));
