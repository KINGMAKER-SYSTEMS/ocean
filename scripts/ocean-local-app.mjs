#!/usr/bin/env node
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { promises as fs } from 'node:fs';
import { spawn, execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { DEFAULT_IGNORES } from '../src/ingest-filters.mjs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');
const ingestScript = path.join(__dirname, 'ocean-ingest-local.mjs');

const DEFAULT_CONFIG_DIR = path.join(os.homedir(), '.config', 'ocean-bedrock');
const DEFAULT_CONFIG_FILE = path.join(DEFAULT_CONFIG_DIR, 'bootstrap.json');
const DEFAULT_APP_STATE_FILE = path.join(os.homedir(), '.local', 'state', 'ocean-bedrock', 'local-app-state.json');
const DEFAULT_INGEST_STATE_FILE = path.join(os.homedir(), '.local', 'state', 'ocean-bedrock', 'ingest-state.json');
const DEFAULT_SERVER_URL = 'https://ocean-bedrock-production.up.railway.app';
const DEFAULT_EXTENSIONS = [
  '.md', '.mdx', '.txt', '.rst', '.json', '.jsonl', '.yaml', '.yml', '.toml', '.ini',
  '.csv', '.tsv', '.html', '.htm', '.css', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx',
  '.py', '.rs', '.go', '.java', '.rb', '.php', '.sh', '.sql', '.xml', '.pdf', '.docx',
  '.pptx', '.xlsx', '.rtf', '.log',
];

const args = parseArgs(process.argv.slice(2));
const configPath = path.resolve(expandHome(args.config || process.env.OCEAN_BEDROCK_CONFIG || DEFAULT_CONFIG_FILE));
const appStatePath = path.resolve(expandHome(args.state || DEFAULT_APP_STATE_FILE));
const ingestStatePath = path.resolve(expandHome(args.ingestState || DEFAULT_INGEST_STATE_FILE));
const host = args.host || '127.0.0.1';
const requestedPort = Number(args.port || process.env.OCEAN_BEDROCK_APP_PORT || 8765);

let activeSync = null;
let scheduleTimer = null;
let scheduleNextAt = null;
let recentLogs = [];

function parseArgs(argv) {
  const parsed = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') parsed.help = true;
    else if (arg === '--config') parsed.config = argv[++i];
    else if (arg === '--state') parsed.state = argv[++i];
    else if (arg === '--ingest-state') parsed.ingestState = argv[++i];
    else if (arg === '--host') parsed.host = argv[++i];
    else if (arg === '--port') parsed.port = argv[++i];
    else if (arg === '--no-open') parsed.noOpen = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return parsed;
}

function usage() {
  console.log(`Usage:
  npm run ocean:app
  npm run ocean:app -- --port 8765 --no-open

Starts the local Ocean Bedrock companion GUI at http://127.0.0.1:<port>.
The app stores config in ~/.config/ocean-bedrock/bootstrap.json and only syncs while open.`);
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

function normalizeServer(value) {
  const parsed = new URL(value || DEFAULT_SERVER_URL);
  parsed.pathname = parsed.pathname.replace(/\/$/, '');
  parsed.search = '';
  parsed.hash = '';
  return parsed.toString().replace(/\/$/, '');
}

function sourceIdFor(folder, name, device) {
  return `src_${crypto.createHash('sha256').update(`${name}:${device}:${folder}`).digest('hex').slice(0, 12)}`;
}

function defaultConfig() {
  const deviceName = os.hostname();
  const ownerName = '';
  const ownerSlug = slugify(ownerName);
  const deviceSlug = slugify(deviceName);
  return {
    version: 1,
    serverUrl: DEFAULT_SERVER_URL,
    token: '',
    tokenKind: 'mcp-bearer',
    ownerName,
    ownerSlug,
    deviceName,
    deviceSlug,
    remoteRoot: `/coworkers/${ownerSlug}/${deviceSlug}`,
    feedMode: 'knowledge-docs',
    maxFileBytes: 10 * 1024 * 1024,
    defaultIgnores: DEFAULT_IGNORES,
    allowedExtensions: DEFAULT_EXTENSIONS,
    sources: [],
    schedule: { enabled: false, intervalMinutes: 60 },
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

async function readJson(file, fallback) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return fallback;
    throw error;
  }
}

async function writeJson(file, value, mode = 0o600) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, `${JSON.stringify(value, null, 2)}\n`, { mode });
}

async function loadConfig() {
  const loaded = await readJson(configPath, null);
  const config = { ...defaultConfig(), ...(loaded || {}) };
  config.serverUrl = normalizeServer(config.serverUrl || DEFAULT_SERVER_URL);
  config.ownerSlug = slugify(config.ownerSlug || config.ownerName);
  config.deviceSlug = slugify(config.deviceSlug || config.deviceName || os.hostname());
  config.remoteRoot = config.remoteRoot || `/coworkers/${config.ownerSlug}/${config.deviceSlug}`;
  config.defaultIgnores = Array.isArray(config.defaultIgnores) ? config.defaultIgnores : DEFAULT_IGNORES;
  config.allowedExtensions = config.allowedExtensions === null ? null : (Array.isArray(config.allowedExtensions) ? config.allowedExtensions : DEFAULT_EXTENSIONS);
  config.sources = Array.isArray(config.sources) ? config.sources : [];
  config.schedule = { enabled: false, intervalMinutes: 60, ...(config.schedule || {}) };
  return config;
}

async function saveConfig(config) {
  config.updatedAt = new Date().toISOString();
  await writeJson(configPath, config, 0o600);
  return config;
}

function redactConfig(config) {
  return {
    ...config,
    token: config.token ? '[saved]' : '',
    tokenPresent: Boolean(config.token),
    configPath,
    appStatePath,
    ingestStatePath,
  };
}

async function loadAppState() {
  return readJson(appStatePath, { version: 1, lastRun: null });
}

async function saveAppState(state) {
  await writeJson(appStatePath, state, 0o600);
}

function logLine(message) {
  const line = `[${new Date().toISOString()}] ${message}`;
  recentLogs.push(line);
  if (recentLogs.length > 200) recentLogs = recentLogs.slice(-200);
  console.log(line);
}

function requireConfigured(config) {
  if (!config.serverUrl) throw new Error('Set the Ocean Bedrock URL first.');
  if (!config.token) throw new Error('Paste and save your invite/token first. Secret-pattern files are excluded by default during ingest.');
  if (!config.ownerName) throw new Error('Set your name first.');
  if (!config.deviceName) throw new Error('Set this device name first.');
}

async function bedrockApi(config, pathname, options = {}) {
  requireConfigured(config);
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

async function writePublicSourceManifest(config) {
  const publicConfig = redactConfig(config);
  publicConfig.token = '[redacted]';
  delete publicConfig.tokenPresent;
  delete publicConfig.configPath;
  delete publicConfig.appStatePath;
  delete publicConfig.ingestStatePath;

  for (const source of config.sources || []) {
    await bedrockApi(config, '/api/v1/mkdir', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path: source.remotePrefix }),
    });
  }
  await bedrockApi(config, '/api/v1/mkdir', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: '/context/ocean-bedrock/sources' }),
  });
  const manifestPath = `/context/ocean-bedrock/sources/${config.ownerSlug}-${config.deviceSlug}.json`;
  await bedrockApi(config, `/api/v1/file?path=${encodeURIComponent(manifestPath)}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: `${JSON.stringify(publicConfig, null, 2)}\n`,
  });
  return manifestPath;
}

async function saveBasicConfig(body) {
  const current = await loadConfig();
  const token = body.token && body.token !== '[saved]' ? String(body.token).trim() : current.token;
  const ownerName = String(body.ownerName || current.ownerName || '').trim();
  const deviceName = String(body.deviceName || current.deviceName || os.hostname()).trim();
  const ownerSlug = slugify(ownerName);
  const deviceSlug = slugify(deviceName);
  const remoteRoot = body.remoteRoot ? String(body.remoteRoot).trim() : `/coworkers/${ownerSlug}/${deviceSlug}`;
  const feedMode = body.feedMode === 'all-files' ? 'all-files' : 'knowledge-docs';
  const maxFileBytes = Math.max(1, Number(body.maxFileMb || current.maxFileBytes / 1024 / 1024 || 10)) * 1024 * 1024;

  const next = {
    ...current,
    serverUrl: normalizeServer(body.serverUrl || current.serverUrl || DEFAULT_SERVER_URL),
    token,
    ownerName,
    ownerSlug,
    deviceName,
    deviceSlug,
    remoteRoot,
    feedMode,
    maxFileBytes,
    allowedExtensions: feedMode === 'all-files' ? null : DEFAULT_EXTENSIONS,
    defaultIgnores: DEFAULT_IGNORES,
  };

  next.sources = next.sources.map((source) => {
    const label = source.label || slugify(path.basename(source.localPath || '') || 'folder');
    return {
      ...source,
      label,
      remotePrefix: source.remotePrefix || `${next.remoteRoot}/${label}`,
    };
  });

  await saveConfig(next);
  reschedule(next).catch((error) => logLine(`Schedule reset failed: ${error.message}`));
  return next;
}

async function addFolder(body) {
  const config = await loadConfig();
  requireConfigured(config);
  const localPath = path.resolve(expandHome(String(body.path || '').trim()));
  const stat = await fs.stat(localPath);
  if (!stat.isDirectory()) throw new Error(`${localPath} is not a folder.`);

  const label = slugify(body.label || path.basename(localPath) || 'folder');
  const id = sourceIdFor(localPath, config.ownerSlug, config.deviceSlug);
  const source = {
    id,
    label,
    localPath,
    remotePrefix: `${config.remoteRoot}/${label}`,
    enabled: true,
    createdAt: new Date().toISOString(),
  };
  const existingIndex = config.sources.findIndex((candidate) => candidate.id === id || candidate.localPath === localPath);
  if (existingIndex >= 0) config.sources[existingIndex] = { ...config.sources[existingIndex], ...source };
  else config.sources.push(source);
  await saveConfig(config);

  let manifestPath = null;
  try {
    manifestPath = await writePublicSourceManifest(config);
  } catch (error) {
    logLine(`Remote source manifest skipped: ${error.message}`);
  }

  return { source, manifestPath };
}

async function setSchedule(body) {
  const config = await loadConfig();
  config.schedule = {
    enabled: Boolean(body.enabled),
    intervalMinutes: Math.max(5, Number(body.intervalMinutes || 60)),
  };
  await saveConfig(config);
  await reschedule(config);
  return config.schedule;
}

function runCommand(command, commandArgs, options = {}) {
  return new Promise((resolve, reject) => {
    execFile(command, commandArgs, options, (error, stdout, stderr) => {
      if (error) {
        error.stdout = stdout;
        error.stderr = stderr;
        reject(error);
      } else {
        resolve({ stdout, stderr });
      }
    });
  });
}

async function commandExists(command) {
  try {
    await runCommand('which', [command]);
    return true;
  } catch {
    return false;
  }
}

async function chooseFolderDialog() {
  if (process.platform === 'darwin') {
    const result = await runCommand('osascript', ['-e', 'POSIX path of (choose folder with prompt "Choose a folder to sync with Ocean Bedrock")']);
    return result.stdout.trim().replace(/\/$/, '');
  }
  if (process.platform === 'win32') {
    const script = `Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Choose a folder to sync with Ocean Bedrock'; if ($d.ShowDialog() -eq 'OK') { Write-Output $d.SelectedPath }`;
    const result = await runCommand('powershell.exe', ['-NoProfile', '-Command', script]);
    return result.stdout.trim();
  }
  if (await commandExists('zenity')) {
    const result = await runCommand('zenity', ['--file-selection', '--directory', '--title', 'Choose a folder to sync with Ocean Bedrock']);
    return result.stdout.trim();
  }
  if (await commandExists('kdialog')) {
    const result = await runCommand('kdialog', ['--getexistingdirectory', os.homedir()]);
    return result.stdout.trim();
  }
  throw new Error('No native folder picker found. Paste the folder path manually. On Linux install zenity or kdialog for click-to-choose.');
}

function parseLastJson(stdout) {
  const text = String(stdout || '').trim();
  const first = text.indexOf('{');
  const last = text.lastIndexOf('}');
  if (first === -1 || last === -1 || last <= first) return null;
  try {
    return JSON.parse(text.slice(first, last + 1));
  } catch {
    return null;
  }
}

async function runSync(reason = 'manual') {
  if (activeSync) return activeSync;
  const config = await loadConfig();
  requireConfigured(config);
  if (!config.sources.some((source) => source.enabled)) throw new Error('Add at least one enabled folder before syncing.');

  activeSync = new Promise((resolve, reject) => {
    const startedAt = new Date().toISOString();
    logLine(`Sync started (${reason})`);
    const child = spawn(process.execPath, [ingestScript, '--config', configPath, '--state', ingestStatePath], {
      cwd: repoRoot,
      env: { ...process.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
    child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
    child.on('error', reject);
    child.on('close', async (code) => {
      const completedAt = new Date().toISOString();
      const parsed = parseLastJson(stdout);
      const state = await loadAppState();
      state.lastRun = {
        ok: code === 0,
        code,
        reason,
        startedAt,
        completedAt,
        stdout: stdout.slice(-20000),
        stderr: stderr.slice(-20000),
        result: parsed,
      };
      await saveAppState(state);
      if (code === 0) {
        logLine(`Sync finished (${reason})`);
        resolve(state.lastRun);
      } else {
        const error = new Error(`Sync failed with exit code ${code}: ${stderr || stdout}`);
        logLine(error.message);
        reject(error);
      }
    });
  }).finally(() => {
    activeSync = null;
  });
  return activeSync;
}

async function reschedule(config = null) {
  if (scheduleTimer) clearTimeout(scheduleTimer);
  scheduleTimer = null;
  scheduleNextAt = null;
  const effective = config || await loadConfig();
  const schedule = effective.schedule || {};
  if (!schedule.enabled) return;
  const delayMs = Math.max(5, Number(schedule.intervalMinutes || 60)) * 60 * 1000;
  const tick = async () => {
    try {
      await runSync('schedule');
    } catch (error) {
      logLine(`Scheduled sync failed: ${error.message}`);
    } finally {
      scheduleNextAt = new Date(Date.now() + delayMs).toISOString();
      scheduleTimer = setTimeout(tick, delayMs);
    }
  };
  scheduleNextAt = new Date(Date.now() + delayMs).toISOString();
  scheduleTimer = setTimeout(tick, delayMs);
}

async function readBody(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  if (!chunks.length) return {};
  const raw = Buffer.concat(chunks).toString('utf8');
  return raw ? JSON.parse(raw) : {};
}

function send(res, status, body, headers = {}) {
  const isString = typeof body === 'string';
  const payload = isString ? body : JSON.stringify(body, null, 2);
  res.writeHead(status, {
    'content-type': isString ? 'text/html; charset=utf-8' : 'application/json; charset=utf-8',
    'cache-control': 'no-store',
    ...headers,
  });
  res.end(payload);
}

async function currentState() {
  const config = await loadConfig();
  const appState = await loadAppState();
  return {
    ok: true,
    activeSync: Boolean(activeSync),
    scheduleNextAt,
    logs: recentLogs.slice(-50),
    config: redactConfig(config),
    appState,
  };
}

async function handle(req, res) {
  try {
    const url = new URL(req.url, `http://${req.headers.host || `${host}:${requestedPort}`}`);
    if (req.method === 'GET' && url.pathname === '/') return send(res, 200, INDEX_HTML);
    if (req.method === 'GET' && url.pathname === '/api/state') return send(res, 200, await currentState());
    if (req.method === 'POST' && url.pathname === '/api/config') return send(res, 200, { ok: true, config: redactConfig(await saveBasicConfig(await readBody(req))) });
    if (req.method === 'POST' && url.pathname === '/api/test') {
      const config = await loadConfig();
      const health = await fetch(`${config.serverUrl}/health`).then((r) => r.json());
      const info = await bedrockApi(config, '/api/v1/info');
      return send(res, 200, { ok: true, health, principal: info.principal });
    }
    if (req.method === 'POST' && url.pathname === '/api/folder-dialog') return send(res, 200, { ok: true, path: await chooseFolderDialog() });
    if (req.method === 'POST' && url.pathname === '/api/folders') return send(res, 200, { ok: true, ...(await addFolder(await readBody(req))), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/schedule') return send(res, 200, { ok: true, schedule: await setSchedule(await readBody(req)), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/sync') return send(res, 200, { ok: true, run: await runSync('manual'), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/manifest') {
      const config = await loadConfig();
      return send(res, 200, { ok: true, manifestPath: await writePublicSourceManifest(config), state: await currentState() });
    }
    return send(res, 404, { ok: false, error: 'Not found' });
  } catch (error) {
    return send(res, 500, { ok: false, error: error.message, stack: process.env.NODE_ENV === 'development' ? error.stack : undefined });
  }
}

function openUrl(url) {
  const commands = process.platform === 'darwin'
    ? [['open', [url]]]
    : process.platform === 'win32'
      ? [['cmd', ['/c', 'start', '', url]]]
      : [['xdg-open', [url]], ['sensible-browser', [url]]];
  for (const [command, commandArgs] of commands) {
    try {
      const child = spawn(command, commandArgs, { stdio: 'ignore', detached: true });
      child.unref();
      return;
    } catch {
      // Try next opener.
    }
  }
}

const INDEX_HTML = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Ocean Bedrock Local Sync</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #07111f; color: #edf6ff; }
    body { margin: 0; min-height: 100vh; background: radial-gradient(circle at top left, #123a5f 0, #07111f 38%, #050914 100%); }
    main { width: min(1120px, calc(100vw - 32px)); margin: 0 auto; padding: 32px 0 48px; }
    h1 { font-size: clamp(32px, 5vw, 56px); margin: 0 0 8px; letter-spacing: -0.05em; }
    h2 { margin: 0 0 16px; font-size: 20px; }
    p { color: #a9bdd1; line-height: 1.55; }
    .hero { display: flex; gap: 20px; justify-content: space-between; align-items: end; margin-bottom: 24px; }
    .pill { display: inline-flex; align-items: center; gap: 8px; padding: 8px 12px; border: 1px solid #244b73; border-radius: 999px; background: rgba(15, 37, 64, 0.72); color: #bfe4ff; font-size: 13px; }
    .grid { display: grid; grid-template-columns: repeat(12, 1fr); gap: 16px; }
    .card { grid-column: span 6; background: rgba(8, 20, 36, 0.82); border: 1px solid rgba(98, 157, 207, 0.22); box-shadow: 0 22px 80px rgba(0,0,0,.28); border-radius: 24px; padding: 22px; backdrop-filter: blur(16px); }
    .wide { grid-column: span 12; }
    label { display: block; font-size: 12px; color: #91aac2; margin: 12px 0 6px; text-transform: uppercase; letter-spacing: .08em; }
    input, select { width: 100%; box-sizing: border-box; border: 1px solid #254965; border-radius: 14px; padding: 12px 13px; background: #071422; color: #f4fbff; font-size: 15px; outline: none; }
    input:focus, select:focus { border-color: #57b5ff; box-shadow: 0 0 0 3px rgba(87,181,255,.16); }
    .row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
    .actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 16px; }
    button { border: 0; border-radius: 999px; padding: 11px 16px; font-weight: 800; cursor: pointer; color: #03101f; background: #7fd1ff; }
    button.secondary { color: #dcefff; background: #15334f; border: 1px solid #2a577c; }
    button.danger { color: #ffecec; background: #5b2530; border: 1px solid #9c4655; }
    button:disabled { opacity: .55; cursor: wait; }
    .sources { display: grid; gap: 10px; margin-top: 14px; }
    .source { display: flex; justify-content: space-between; gap: 12px; padding: 12px; border: 1px solid #213f59; border-radius: 16px; background: #071724; }
    .source strong { display: block; }
    .source small { color: #9fb5c8; word-break: break-all; }
    .integration-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; }
    .integration { padding: 14px; border-radius: 18px; border: 1px solid #22445f; background: #071724; }
    .integration.live { border-color: #2db783; }
    .integration span { display: block; font-weight: 800; }
    .integration small { color: #91aac2; }
    pre { white-space: pre-wrap; word-break: break-word; max-height: 360px; overflow: auto; background: #020811; border: 1px solid #1e3a55; border-radius: 18px; padding: 14px; color: #bde5ff; }
    .status { min-height: 22px; margin-top: 12px; color: #bde5ff; }
    .ok { color: #78f0b2; } .bad { color: #ff9da8; }
    @media (max-width: 760px) { .card { grid-column: span 12; } .row { grid-template-columns: 1fr; } .hero { display: block; } }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <div>
        <div class="pill">🌊 Ocean Bedrock Local Sync</div>
        <h1>Pick folders. Pick schedule. Done.</h1>
        <p>No terminal required after this is running. Paste the invite token once, choose folders, and sync while this app is open.</p>
      </div>
      <div class="pill" id="sync-pill">Checking...</div>
    </section>

    <section class="grid">
      <div class="card">
        <h2>1. Connect</h2>
        <label>Ocean Bedrock URL</label>
        <input id="serverUrl" placeholder="https://ocean-bedrock-production.up.railway.app" />
        <label>Invite / sync token</label>
        <input id="token" type="password" placeholder="Paste token once" />
        <div class="row">
          <div><label>Your name</label><input id="ownerName" placeholder="operator-contributor" /></div>
          <div><label>This device</label><input id="deviceName" placeholder="macbook" /></div>
        </div>
        <div class="row">
          <div><label>Mode</label><select id="feedMode"><option value="knowledge-docs">Docs/code only</option><option value="all-files">All files under size limit</option></select></div>
          <div><label>Max file MB</label><input id="maxFileMb" type="number" min="1" max="250" value="10" /></div>
        </div>
        <div class="actions">
          <button onclick="saveConfig()">Save connection</button>
          <button class="secondary" onclick="testConnection()">Test</button>
        </div>
        <div class="status" id="connectStatus"></div>
      </div>

      <div class="card">
        <h2>2. Choose integrations</h2>
        <div class="integration-grid">
          <div class="integration live"><span>Local folders</span><small>Live now</small></div>
          <div class="integration"><span>GitHub</span><small>Next</small></div>
          <div class="integration"><span>Telegram</span><small>Next</small></div>
          <div class="integration"><span>Notion</span><small>Coming soon</small></div>
          <div class="integration"><span>Slack</span><small>Coming soon</small></div>
          <div class="integration"><span>Linear</span><small>Coming soon</small></div>
        </div>
        <label>Folder path</label>
        <input id="folderPath" placeholder="Click Choose Folder or paste a path" />
        <label>Folder label (optional)</label>
        <input id="folderLabel" placeholder="documents, projects, notes" />
        <div class="actions">
          <button class="secondary" onclick="chooseFolder()">Choose folder</button>
          <button onclick="addFolder()">Add folder</button>
          <button class="secondary" onclick="writeManifest()">Refresh manifest</button>
        </div>
        <div class="status" id="folderStatus"></div>
      </div>

      <div class="card">
        <h2>3. Schedule</h2>
        <label>Auto sync</label>
        <select id="scheduleEnabled"><option value="false">Manual only</option><option value="true">On a schedule while app is open</option></select>
        <label>Every</label>
        <select id="intervalMinutes"><option value="15">15 minutes</option><option value="30">30 minutes</option><option value="60">1 hour</option><option value="180">3 hours</option><option value="360">6 hours</option><option value="1440">1 day</option></select>
        <div class="actions">
          <button onclick="saveSchedule()">Save schedule</button>
          <button class="secondary" onclick="runSync()">Sync now</button>
        </div>
        <div class="status" id="scheduleStatus"></div>
      </div>

      <div class="card">
        <h2>Selected folders</h2>
        <div class="sources" id="sources"></div>
      </div>

      <div class="card wide">
        <h2>Activity</h2>
        <pre id="activity">Loading...</pre>
      </div>
    </section>
  </main>

  <script>
    let state = null;
    const $ = (id) => document.getElementById(id);

    async function api(path, body = undefined) {
      const options = body === undefined ? {} : { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) };
      const res = await fetch(path, options);
      const data = await res.json();
      if (!res.ok || data.ok === false) throw new Error(data.error || 'Request failed');
      return data;
    }

    function setStatus(id, text, ok = true) {
      $(id).innerHTML = text ? '<span class="' + (ok ? 'ok' : 'bad') + '">' + escapeHtml(text) + '</span>' : '';
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[ch]));
    }

    function render() {
      if (!state) return;
      const c = state.config;
      $('serverUrl').value = c.serverUrl || '';
      $('token').value = c.tokenPresent ? '[saved]' : '';
      $('ownerName').value = c.ownerName || '';
      $('deviceName').value = c.deviceName || '';
      $('feedMode').value = c.feedMode || 'knowledge-docs';
      $('maxFileMb').value = Math.round((c.maxFileBytes || 10485760) / 1024 / 1024);
      $('scheduleEnabled').value = String(Boolean(c.schedule && c.schedule.enabled));
      $('intervalMinutes').value = String((c.schedule && c.schedule.intervalMinutes) || 60);
      $('sync-pill').textContent = state.activeSync ? 'Sync running...' : (state.scheduleNextAt ? 'Next sync: ' + new Date(state.scheduleNextAt).toLocaleString() : 'Manual sync');
      $('sources').innerHTML = (c.sources || []).length ? c.sources.map((s) => '<div class="source"><div><strong>' + escapeHtml(s.label) + '</strong><small>' + escapeHtml(s.localPath) + '<br>' + escapeHtml(s.remotePrefix) + '</small></div><div>' + (s.enabled ? '✅' : '⏸️') + '</div></div>').join('') : '<p>No folders yet. Add one above.</p>';
      const last = state.appState && state.appState.lastRun ? '\n\nLast run:\n' + JSON.stringify(state.appState.lastRun.result || state.appState.lastRun, null, 2) : '';
      $('activity').textContent = (state.logs || []).join('\n') + last;
    }

    async function refresh() {
      state = await api('/api/state');
      render();
    }

    async function saveConfig() {
      try {
        const data = await api('/api/config', {
          serverUrl: $('serverUrl').value,
          token: $('token').value,
          ownerName: $('ownerName').value,
          deviceName: $('deviceName').value,
          feedMode: $('feedMode').value,
          maxFileMb: $('maxFileMb').value,
        });
        state.config = data.config;
        setStatus('connectStatus', 'Saved.');
        await refresh();
      } catch (e) { setStatus('connectStatus', e.message, false); }
    }

    async function testConnection() {
      try {
        setStatus('connectStatus', 'Testing...');
        const data = await api('/api/test', {});
        setStatus('connectStatus', 'Connected as ' + (data.principal && data.principal.name ? data.principal.name : 'token user'));
      } catch (e) { setStatus('connectStatus', e.message, false); }
    }

    async function chooseFolder() {
      try {
        setStatus('folderStatus', 'Opening folder picker...');
        const data = await api('/api/folder-dialog', {});
        $('folderPath').value = data.path;
        setStatus('folderStatus', 'Folder selected.');
      } catch (e) { setStatus('folderStatus', e.message, false); }
    }

    async function addFolder() {
      try {
        setStatus('folderStatus', 'Adding folder...');
        await api('/api/folders', { path: $('folderPath').value, label: $('folderLabel').value });
        $('folderPath').value = '';
        $('folderLabel').value = '';
        setStatus('folderStatus', 'Folder added.');
        await refresh();
      } catch (e) { setStatus('folderStatus', e.message, false); }
    }

    async function writeManifest() {
      try {
        setStatus('folderStatus', 'Refreshing manifest...');
        const data = await api('/api/manifest', {});
        setStatus('folderStatus', 'Manifest saved to ' + data.manifestPath);
        await refresh();
      } catch (e) { setStatus('folderStatus', e.message, false); }
    }

    async function saveSchedule() {
      try {
        await api('/api/schedule', { enabled: $('scheduleEnabled').value === 'true', intervalMinutes: Number($('intervalMinutes').value) });
        setStatus('scheduleStatus', 'Schedule saved.');
        await refresh();
      } catch (e) { setStatus('scheduleStatus', e.message, false); }
    }

    async function runSync() {
      try {
        setStatus('scheduleStatus', 'Syncing... keep this window open.');
        await api('/api/sync', {});
        setStatus('scheduleStatus', 'Sync complete.');
        await refresh();
      } catch (e) { setStatus('scheduleStatus', e.message, false); await refresh(); }
    }

    refresh();
    setInterval(refresh, 5000);
  </script>
</body>
</html>`;

if (args.help) {
  usage();
  process.exit(0);
}

await reschedule().catch((error) => logLine(`Schedule init failed: ${error.message}`));

const server = http.createServer((req, res) => {
  handle(req, res).catch((error) => send(res, 500, { ok: false, error: error.message }));
});

server.listen(requestedPort, host, () => {
  const address = server.address();
  const url = `http://${host}:${address.port}/`;
  logLine(`Ocean Bedrock local sync app running at ${url}`);
  logLine(`Config: ${configPath}`);
  if (!args.noOpen) openUrl(url);
});
