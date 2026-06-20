#!/usr/bin/env node
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import readline from 'node:readline/promises';
import { stdin as input, stdout as output } from 'node:process';
import crypto from 'node:crypto';
import { DEFAULT_IGNORES } from '../src/ingest-filters.mjs';
import {
  ensureSourceAdapters,
  upsertSourceInstance,
  upsertSourceStream,
} from '../src/sources.mjs';

const DEFAULT_CONFIG_DIR = path.join(os.homedir(), '.config', 'ocean-bedrock');
const DEFAULT_CONFIG_FILE = path.join(DEFAULT_CONFIG_DIR, 'bootstrap.json');
const DEFAULT_EXTENSIONS = [
  '.md', '.mdx', '.txt', '.rst', '.json', '.jsonl', '.yaml', '.yml', '.toml', '.ini',
  '.csv', '.tsv', '.html', '.htm', '.css', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx',
  '.py', '.rs', '.go', '.java', '.rb', '.php', '.sh', '.sql', '.xml', '.pdf', '.docx',
  '.pptx', '.xlsx', '.rtf', '.log',
];

function usage() {
  console.log(`Usage:
  npm run ocean:bootstrap
  npm run ocean:bootstrap -- --server https://... --token <token> --name alice --folder ~/Documents --folder ~/Projects --yes

Creates ~/.config/ocean-bedrock/bootstrap.json and registers selected folders with Bedrock/Longhouse.

Options:
  --server <url>       Bedrock/Longhouse HTTP URL.
  --token <token>      Bearer token/MCP token.
  --name <name>        Coworker/operator name.
  --device <name>      Device name. Default: hostname.
  --folder <path>      Local folder to feed. Can be repeated.
  --remote-prefix <p>  Virtual root. Default: /coworkers/<name>/<device>
  --config <path>      Config output path. Default: ~/.config/ocean-bedrock/bootstrap.json
  --all-files          Feed all files under max size, not just knowledge-document extensions. Secret patterns remain excluded by ingest.
  --max-file-mb <n>    Max file size for ingest. Default: 10.
  --yes                Non-interactive; requires server, token, name, and at least one folder.
`);
}

function parseArgs(argv) {
  const args = { folders: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--server') args.server = argv[++i];
    else if (arg === '--token') args.token = argv[++i];
    else if (arg === '--name') args.name = argv[++i];
    else if (arg === '--device') args.device = argv[++i];
    else if (arg === '--folder') args.folders.push(argv[++i]);
    else if (arg === '--remote-prefix') args.remotePrefix = argv[++i];
    else if (arg === '--config') args.config = argv[++i];
    else if (arg === '--all-files') args.allFiles = true;
    else if (arg === '--max-file-mb') args.maxFileMb = Number(argv[++i]);
    else if (arg === '--yes' || arg === '-y') args.yes = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function expandHome(value) {
  if (!value) return value;
  if (value === '~') return os.homedir();
  if (value.startsWith('~/')) return path.join(os.homedir(), value.slice(2));
  return value;
}

function slugify(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'coworker';
}

function normalizeServer(url) {
  const parsed = new URL(url);
  parsed.pathname = parsed.pathname.replace(/\/$/, '');
  parsed.search = '';
  parsed.hash = '';
  return parsed.toString().replace(/\/$/, '');
}

async function promptMissing(args) {
  const rl = readline.createInterface({ input, output });
  try {
    if (!args.server) args.server = await rl.question('Bedrock/Longhouse URL: ');
    if (!args.token) args.token = await rl.question('MCP/Bearer token: ');
    if (!args.name) args.name = await rl.question('Your name or agent handle: ');
    if (!args.device) {
      const answer = await rl.question(`Device name [${os.hostname()}]: `);
      args.device = answer || os.hostname();
    }
    if (!args.folders.length) {
      const answer = await rl.question('Folders to feed, comma-separated: ');
      args.folders = answer.split(',').map((item) => item.trim()).filter(Boolean);
    }
    if (args.allFiles === undefined) {
      const answer = await rl.question('Feed all files under size limit? Default is docs/code only. [y/N]: ');
      args.allFiles = /^y(es)?$/i.test(answer.trim());
    }
  } finally {
    rl.close();
  }
  return args;
}

async function api(config, pathname, options = {}) {
  const response = await fetch(`${config.serverUrl}${pathname}`, {
    ...options,
    headers: {
      Authorization: `Bearer ${config.token}`,
      ...(options.headers || {}),
    },
  });
  const contentType = response.headers.get('content-type') || '';
  const body = contentType.includes('application/json') ? await response.json() : await response.text();
  if (!response.ok) {
    throw new Error(`${options.method || 'GET'} ${pathname} failed: ${response.status} ${JSON.stringify(body)}`);
  }
  return body;
}

async function ensureDirectoryExists(folder) {
  const resolved = path.resolve(expandHome(folder));
  const stat = await fs.stat(resolved);
  if (!stat.isDirectory()) throw new Error(`${resolved} is not a directory.`);
  return resolved;
}

function sourceIdFor(folder, name, device) {
  return `src_${crypto.createHash('sha256').update(`${name}:${device}:${folder}`).digest('hex').slice(0, 12)}`;
}

async function registerSourceRegistry(config, nameSlug, deviceSlug) {
  if (!process.env.DATABASE_URL) return { enabled: false, reason: 'DATABASE_URL is not set' };

  const correlationId = `cor-bootstrap-${nameSlug}-${deviceSlug}`;
  try {
    await ensureSourceAdapters();
    const instance = await upsertSourceInstance(null, {
      adapter_id: 'local_folder',
      name: `${nameSlug}-${deviceSlug}`,
      owner_name: config.ownerName,
      owner_token_id: null,
      remote_prefix: config.remoteRoot,
      config: {
        device: config.deviceName,
        device_slug: config.deviceSlug,
        feed_mode: config.feedMode,
        allowed_extensions: config.allowedExtensions,
        ignore: config.defaultIgnores,
        max_file_bytes: config.maxFileBytes,
        source_count: config.sources.length,
      },
      secret_ref: null,
      clearance: 'CONFIDENTIAL',
      correlationId,
    });

    const streams = [];
    for (const source of config.sources) {
      const stream = await upsertSourceStream(null, {
        source_instance_id: instance.id,
        stream_key: source.id,
        stream_type: 'folder',
        remote_prefix: source.remotePrefix,
        selection: {
          label: source.label,
          local_path: source.localPath,
          enabled: source.enabled,
          recursive: true,
        },
        cursor: {},
        correlationId,
      });
      source.sourceInstanceId = instance.id;
      source.sourceStreamId = stream.id;
      streams.push({ sourceId: source.id, streamId: stream.id });
    }

    config.sourceRegistry = {
      adapterId: 'local_folder',
      sourceInstanceId: instance.id,
      correlationId,
      registeredAt: new Date().toISOString(),
    };

    return { enabled: true, instanceId: instance.id, streams };
  } catch (error) {
    return { enabled: false, error: error.message };
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    usage();
    process.exit(0);
  }

  if (!args.yes) await promptMissing(args);
  if (!args.server || !args.token || !args.name || !args.folders.length) {
    usage();
    throw new Error('Missing required server, token, name, or folder.');
  }

  const nameSlug = slugify(args.name);
  const deviceSlug = slugify(args.device || os.hostname());
  const remoteRoot = args.remotePrefix || `/coworkers/${nameSlug}/${deviceSlug}`;
  const folders = [];
  for (const folder of args.folders) folders.push(await ensureDirectoryExists(folder));

  const configPath = path.resolve(expandHome(args.config || DEFAULT_CONFIG_FILE));
  const config = {
    version: 1,
    serverUrl: normalizeServer(args.server),
    token: args.token,
    tokenKind: 'mcp-bearer',
    ownerName: args.name,
    ownerSlug: nameSlug,
    deviceName: args.device || os.hostname(),
    deviceSlug,
    remoteRoot,
    feedMode: args.allFiles ? 'all-files' : 'knowledge-docs',
    maxFileBytes: Math.max(1, Number(args.maxFileMb || 10)) * 1024 * 1024,
    defaultIgnores: DEFAULT_IGNORES,
    allowedExtensions: args.allFiles ? null : DEFAULT_EXTENSIONS,
    sources: folders.map((folder) => {
      const label = slugify(path.basename(folder) || 'root');
      const id = sourceIdFor(folder, nameSlug, deviceSlug);
      return {
        id,
        label,
        localPath: folder,
        remotePrefix: `${remoteRoot}/${label}`,
        enabled: true,
        createdAt: new Date().toISOString(),
      };
    }),
    createdAt: new Date().toISOString(),
  };

  console.log('Checking server/token...');
  await api(config, '/health');
  const info = await api(config, '/api/v1/info');

  for (const source of config.sources) {
    await api(config, '/api/v1/mkdir', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path: source.remotePrefix }),
    });
  }

  await api(config, '/api/v1/mkdir', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: '/context/ocean-bedrock/sources' }),
  });

  const sourceRegistry = await registerSourceRegistry(config, nameSlug, deviceSlug);
  if (sourceRegistry.error) console.warn(`Source registry skipped: ${sourceRegistry.error}`);

  const publicConfig = { ...config, token: '[redacted]' };
  await api(config, `/api/v1/file?path=${encodeURIComponent(`/context/ocean-bedrock/sources/${nameSlug}-${deviceSlug}.json`)}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: `${JSON.stringify(publicConfig, null, 2)}\n`,
  });

  try {
    await api(config, '/api/v1/ledger/snapshots', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        name: `${args.name} ${config.deviceName} feed bootstrap`,
        correlation_id: `cor-bootstrap-${nameSlug}-${deviceSlug}`,
        virtual_path: `/context/ocean-bedrock/sources/${nameSlug}-${deviceSlug}.json`,
        files: [`/context/ocean-bedrock/sources/${nameSlug}-${deviceSlug}.json`],
        summary: 'Coworker selected local folders to feed into the Ocean knowledge layer.',
        metadata: { sources: publicConfig.sources, feedMode: config.feedMode },
        clearance: 'CONFIDENTIAL',
        tags: ['ocean-bedrock', 'bootstrap', 'local-folder-feed'],
      }),
    });
  } catch (error) {
    console.warn(`Ledger snapshot skipped: ${error.message}`);
  }

  await fs.mkdir(path.dirname(configPath), { recursive: true });
  await fs.writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600 });

  const envPath = path.join(path.dirname(configPath), 'env');
  await fs.writeFile(envPath, `export OCEAN_BEDROCK_URL=${JSON.stringify(config.serverUrl)}\nexport OCEAN_BEDROCK_TOKEN=${JSON.stringify(config.token)}\nexport OCEAN_BEDROCK_CONFIG=${JSON.stringify(configPath)}\n`, { mode: 0o600 });

  console.log(JSON.stringify({
    ok: true,
    server: config.serverUrl,
    principal: info.principal,
    configPath,
    envPath,
    sources: publicConfig.sources,
    sourceRegistry,
    next: `npm run ocean:ingest -- --config ${configPath}`,
  }, null, 2));
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
