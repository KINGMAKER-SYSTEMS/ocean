#!/usr/bin/env node

const DEFAULT_URL = 'https://ocean-bedrock-production.up.railway.app';

function usage() {
  console.log(`Usage:
  npm run ocean:triage
  npm run ocean:triage -- --url https://ocean-bedrock-production.up.railway.app --token-file ~/.config/ocean-bedrock/operator-contributor-token.txt

Options:
  --url <url>          Ocean Bedrock URL. Defaults to OCEAN_BEDROCK_URL or production.
  --token <token>      Scoped bearer token. Defaults to OCEAN_BEDROCK_TOKEN.
  --token-file <path>  Read scoped bearer token from a local file.
  --report-path <path> Override report path.
  --force              Bypass per-slot idempotency guard and re-run even if this slot already ran.
`);
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--url') args.url = argv[++i];
    else if (arg === '--token') args.token = argv[++i];
    else if (arg === '--token-file') args.tokenFile = argv[++i];
    else if (arg === '--report-path') args.reportPath = argv[++i];
    else if (arg === '--force') args.force = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function expandHome(value) {
  if (!value) return value;
  if (value === '~') return process.env.HOME;
  if (value.startsWith('~/')) return `${process.env.HOME}/${value.slice(2)}`;
  return value;
}

async function readToken(args) {
  if (args.token) return args.token;
  if (process.env.OCEAN_BEDROCK_TOKEN) return process.env.OCEAN_BEDROCK_TOKEN;
  if (args.tokenFile) {
    const { promises: fs } = await import('node:fs');
    return (await fs.readFile(expandHome(args.tokenFile), 'utf8')).trim();
  }
  throw new Error('Missing token. Set OCEAN_BEDROCK_TOKEN or pass --token-file.');
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  usage();
  process.exit(0);
}

const url = (args.url || process.env.OCEAN_BEDROCK_URL || DEFAULT_URL).replace(/\/$/, '');
const token = await readToken(args);
const response = await fetch(`${url}/api/v1/ocean-context/triage/daily`, {
  method: 'POST',
  headers: {
    Authorization: `Bearer ${token}`,
    'content-type': 'application/json',
  },
  body: JSON.stringify({ reportPath: args.reportPath || null, force: args.force === true }),
});
const body = await response.json();
if (!response.ok) throw new Error(`Triage failed: ${response.status} ${JSON.stringify(body)}`);
console.log(JSON.stringify(body, null, 2));
