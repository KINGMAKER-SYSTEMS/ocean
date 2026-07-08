#!/usr/bin/env node
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { promises as fs } from 'node:fs';
import { spawn, execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { DEFAULT_IGNORES } from '../src/ingest-filters.mjs';
import { WebSocketServer } from 'ws';

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

async function redeemInviteCode(body) {
  const current = await loadConfig();
  const serverUrl = normalizeServer(body.serverUrl || current.serverUrl || DEFAULT_SERVER_URL);
  const code = String(body.code || '').trim();
  if (!code) throw new Error('Paste an invite code first.');
  const response = await fetch(`${serverUrl}/api/v1/invites/redeem`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ code }),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Invite redeem failed: ${response.status}`);
  const next = {
    ...current,
    serverUrl,
    token: result.token,
    tokenKind: 'invite-redeemed-bearer',
    ownerName: body.ownerName || current.ownerName || result.record?.name || '',
    deviceName: body.deviceName || current.deviceName || os.hostname(),
  };
  next.ownerSlug = slugify(next.ownerSlug || next.ownerName);
  next.deviceSlug = slugify(next.deviceSlug || next.deviceName);
  next.remoteRoot = next.remoteRoot || `/coworkers/${next.ownerSlug}/${next.deviceSlug}`;
  await saveConfig(next);
  return { invite: result.invite, token: '[saved]', record: result.record, config: redactConfig(next) };
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
  const nextSchedule = {
    enabled: Boolean(body.enabled),
    intervalMinutes: Math.max(5, Number(body.intervalMinutes || 60)),
  };
  if (nextSchedule.enabled) {
    const appState = await loadAppState();
    requirePreviewAcknowledgement(config, appState, 'schedule');
  }
  config.schedule = nextSchedule;
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

function syncPlanFingerprint(config) {
  const normalized = {
    serverUrl: config.serverUrl,
    ownerSlug: config.ownerSlug,
    deviceSlug: config.deviceSlug,
    remoteRoot: config.remoteRoot,
    feedMode: config.feedMode,
    maxFileBytes: config.maxFileBytes,
    allowedExtensions: config.allowedExtensions,
    defaultIgnores: config.defaultIgnores,
    sources: (config.sources || [])
      .filter((source) => source.enabled)
      .map((source) => ({
        id: source.id,
        label: source.label,
        localPath: source.localPath,
        remotePrefix: source.remotePrefix,
      }))
      .sort((a, b) => String(a.id).localeCompare(String(b.id))),
  };
  return crypto.createHash('sha256').update(JSON.stringify(normalized)).digest('hex');
}

function requirePreviewAcknowledgement(config, appState, reason) {
  const fingerprint = syncPlanFingerprint(config);
  const preview = appState.lastPreview;
  const ack = appState.previewAcknowledgement;
  const valid = preview?.ok
    && preview.planFingerprint === fingerprint
    && ack?.planFingerprint === fingerprint
    && ack?.previewCompletedAt === preview.completedAt
    && ack?.acknowledgedAt;
  if (!valid) {
    const error = new Error(reason === 'schedule'
      ? 'Scheduled sync is locked until you run a dry-run preview and click “I reviewed this preview — unlock sync”.'
      : 'Sync is locked until you run a dry-run preview and click “I reviewed this preview — unlock sync”.');
    error.statusCode = 409;
    error.code = 'PREVIEW_ACK_REQUIRED';
    throw error;
  }
}

async function runPreview() {
  const config = await loadConfig();
  requireConfigured(config);
  if (!config.sources.some((source) => source.enabled)) throw new Error('Add at least one enabled folder before previewing.');

  const startedAt = new Date().toISOString();
  const planFingerprint = syncPlanFingerprint(config);
  logLine('Preview started (dry-run)');
  const result = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [ingestScript, '--config', configPath, '--state', ingestStatePath, '--dry-run', '--report'], {
      cwd: repoRoot,
      env: { ...process.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
    child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
    child.on('error', reject);
    child.on('close', (code) => {
      const parsed = parseLastJson(stdout);
      if (code === 0) resolve({ ok: true, code, stdout: stdout.slice(-20000), stderr: stderr.slice(-20000), result: parsed });
      else reject(new Error(`Preview failed with exit code ${code}: ${stderr || stdout}`));
    });
  });

  const state = await loadAppState();
  state.lastPreview = {
    ...result,
    startedAt,
    completedAt: new Date().toISOString(),
    planFingerprint,
  };
  state.previewAcknowledgement = null;
  await saveAppState(state);
  logLine('Preview finished (dry-run); acknowledgement required before real sync');
  return state.lastPreview;
}

async function acknowledgePreview() {
  const config = await loadConfig();
  requireConfigured(config);
  const state = await loadAppState();
  const fingerprint = syncPlanFingerprint(config);
  if (!state.lastPreview?.ok || state.lastPreview.planFingerprint !== fingerprint) {
    const error = new Error('Run a fresh dry-run preview for the current folders/settings before unlocking sync.');
    error.statusCode = 409;
    error.code = 'FRESH_PREVIEW_REQUIRED';
    throw error;
  }
  state.previewAcknowledgement = {
    planFingerprint: fingerprint,
    previewCompletedAt: state.lastPreview.completedAt,
    acknowledgedAt: new Date().toISOString(),
  };
  await saveAppState(state);
  logLine('Dry-run preview acknowledged; real sync unlocked for current folders/settings');
  return state.previewAcknowledgement;
}

async function runSync(reason = 'manual') {
  if (activeSync) return activeSync;
  const config = await loadConfig();
  requireConfigured(config);
  if (!config.sources.some((source) => source.enabled)) throw new Error('Add at least one enabled folder before syncing.');
  const preflightState = await loadAppState();
  requirePreviewAcknowledgement(config, preflightState, reason);

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


const STATIC_TYPES = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.wasm', 'application/wasm'],
  ['.svg', 'image/svg+xml; charset=utf-8'],
  ['.png', 'image/png'],
]);

function staticType(file) {
  return STATIC_TYPES.get(path.extname(file)) || 'application/octet-stream';
}

async function fileReadable(file) {
  try { await fs.access(file); return true; } catch { return false; }
}

async function sendStatic(res, file) {
  const bytes = await fs.readFile(file);
  res.writeHead(200, {
    'content-type': staticType(file),
    'cache-control': 'no-store',
    'content-length': bytes.length,
  });
  res.end(bytes);
}

// Serves the operator app (ui/app/**), the onboarding stage assets
// (ui/onboarding/**), vendored libraries (assets/vendor/**,
// assets/integrations/**), and the canonical design system (design/**)
// with path containment.
async function serveStatic(req, res, url) {
  if (req.method !== 'GET') return false;
  const prefixes = ['/ui/app/', '/ui/onboarding/', '/assets/vendor/', '/assets/integrations/', '/design/'];
  const match = prefixes.find((p) => url.pathname === p || url.pathname.startsWith(p));
  if (!match) return false;
  const root = path.resolve(repoRoot);
  const target = path.resolve(repoRoot, '.' + url.pathname);
  if (!target.startsWith(root + path.sep)) { send(res, 403, { ok: false, error: 'Forbidden' }); return true; }
  if (!await fileReadable(target)) return false;
  await sendStatic(res, target);
  return true;
}

async function currentState() {
  const config = await loadConfig();
  const appState = await loadAppState();
  const planFingerprint = syncPlanFingerprint(config);
  const preview = appState.lastPreview;
  const ack = appState.previewAcknowledgement;
  const syncUnlocked = Boolean(preview?.ok
    && preview.planFingerprint === planFingerprint
    && ack?.planFingerprint === planFingerprint
    && ack?.previewCompletedAt === preview.completedAt
    && ack?.acknowledgedAt);
  return {
    ok: true,
    activeSync: Boolean(activeSync),
    scheduleNextAt,
    logs: recentLogs.slice(-50),
    syncPlanFingerprint: planFingerprint,
    syncUnlocked,
    config: redactConfig(config),
    appState,
  };
}

// --- Ceremony protocol (Build Lane 1) -------------------------------------
// Standardized skill-driven onboarding ceremony. The conductor is any
// skill-capable agent; it creates a run, streams events over REST, and the
// browser UI subscribes over WebSocket. See
// docs/superpowers/plans/2026-07-07-ocean-skill-onboarding-ceremony.md.
const CEREMONY_EVENT_SCHEMA = 'ocean.ceremony.event.v1';
const CEREMONY_WS_SCHEMA = 'ocean.ceremony.ws.v1';
const CEREMONY_BUFFER_CAP = 1000;
const CEREMONY_HEARTBEAT_MS = 15_000;
const ALLOWED_RUNTIMES = new Set(['claude-code', 'codex', 'pi', 'omp', 'other']);
const ALLOWED_VISIBILITY = new Set(['public', 'private', 'redacted']);
// Safe label/count keys that may render for visibility=redacted events.
// Anything not in this set is dropped before the event reaches a renderer.
const REDACT_SAFE_PAYLOAD_KEYS = new Set([
  'sourceId', 'label', 'logo', 'root', 'displayName', 'relPath', 'reason',
  'name', 'scene', 'focus', 'tone', 'gate', 'target', 'instructions',
  'counts', 'totals', 'progress', 'total', 'indeterminate', 'text',
]);
// Persisted alongside the app state so `--state` test isolation also isolates
// ceremony run metadata. Private payloads are never persisted (see below).
const ceremonyRunsPath = path.join(path.dirname(appStatePath), 'ceremony-runs.json');
// runId -> { runId, createdAt, status, runner, ownerName, mode, events, nextSeq, subscribers }
const ceremonyRuns = new Map();

function nowIso() {
  return new Date().toISOString();
}

function generateRunId() {
  const d = new Date();
  const date = `${d.getUTCFullYear()}${String(d.getUTCMonth() + 1).padStart(2, '0')}${String(d.getUTCDate()).padStart(2, '0')}`;
  return `cer_${date}_${crypto.randomBytes(6).toString('hex')}`;
}

function isRenderableVisibility(visibility) {
  // visibility=private events are kept in the run buffer but never rendered.
  return visibility !== 'private';
}

function redactPayloadForRender(payload, visibility) {
  if (visibility === 'redacted') {
    const safe = {};
    if (payload && typeof payload === 'object') {
      for (const [key, value] of Object.entries(payload)) {
        if (REDACT_SAFE_PAYLOAD_KEYS.has(key)) safe[key] = value;
      }
    }
    safe.redacted = true;
    return safe;
  }
  return payload && typeof payload === 'object' ? payload : {};
}

// Build a render-safe copy of a stored event for WS broadcast / REST replay.
// Private events are filtered out by callers; this never leaks their payload.
function renderableCeremonyEvent(event) {
  const visibility = ALLOWED_VISIBILITY.has(event.visibility) ? event.visibility : 'public';
  return {
    schema: CEREMONY_EVENT_SCHEMA,
    runId: event.runId,
    seq: event.seq,
    ts: event.ts,
    runner: event.runner,
    phase: event.phase,
    type: event.type,
    status: event.status,
    message: event.message,
    visibility,
    payload: redactPayloadForRender(event.payload, visibility),
  };
}

function createCeremonyRun({ runner, ownerName, mode } = {}) {
  const runtime = runner && typeof runner === 'object' ? runner.runtime : undefined;
  if (!ALLOWED_RUNTIMES.has(runtime)) {
    const err = new Error(`runner.runtime must be one of: ${[...ALLOWED_RUNTIMES].join(', ')}`);
    err.statusCode = 400;
    throw err;
  }
  const run = {
    runId: generateRunId(),
    createdAt: nowIso(),
    status: 'running',
    runner: {
      kind: 'skill-runner',
      runtime,
      name: typeof runner.name === 'string' ? runner.name : undefined,
    },
    ownerName: typeof ownerName === 'string' ? ownerName : undefined,
    mode: typeof mode === 'string' ? mode : 'onboarding-import-v1',
    events: [],
    nextSeq: 1,
    subscribers: new Set(),
  };
  ceremonyRuns.set(run.runId, run);
  return run;
}

function broadcastCeremonyEvent(run, event) {
  if (!isRenderableVisibility(event.visibility)) return;
  const frame = { kind: 'event', runId: run.runId, event: renderableCeremonyEvent(event) };
  for (const ws of run.subscribers) {
    if (ws.readyState === 1) ws.send(JSON.stringify(frame));
  }
}

function appendCeremonyEvents(run, incoming) {
  const list = Array.isArray(incoming) ? incoming : [incoming];
  const accepted = [];
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    if (typeof raw.type !== 'string' || !raw.type) continue;
    const visibility = ALLOWED_VISIBILITY.has(raw.visibility) ? raw.visibility : 'public';
    // Server owns seq and ts. A runner-supplied ts is preserved only as
    // payload.runnerTs; a runner-supplied seq is ignored entirely.
    const payload = raw.payload && typeof raw.payload === 'object' ? { ...raw.payload } : {};
    if (typeof raw.ts === 'string' && payload.runnerTs === undefined) payload.runnerTs = raw.ts;
    const event = {
      schema: CEREMONY_EVENT_SCHEMA,
      runId: run.runId,
      seq: run.nextSeq,
      ts: nowIso(),
      runner: run.runner,
      phase: typeof raw.phase === 'string' ? raw.phase : undefined,
      type: raw.type,
      status: typeof raw.status === 'string' ? raw.status : undefined,
      message: typeof raw.message === 'string' ? raw.message : undefined,
      visibility,
      payload,
    };
    run.nextSeq += 1;
    run.events.push(event);
    accepted.push(event);
    broadcastCeremonyEvent(run, event);
  }
  // Bound the per-run buffer to the latest CEREMONY_BUFFER_CAP events.
  if (run.events.length > CEREMONY_BUFFER_CAP) {
    run.events.splice(0, run.events.length - CEREMONY_BUFFER_CAP);
  }
  return accepted;
}

function completeCeremonyRun(run) {
  run.status = 'completed';
  const alreadyCompleted = run.events.some((e) => e.type === 'ceremony.run_completed');
  let broadcast = false;
  if (!alreadyCompleted) {
    const event = {
      schema: CEREMONY_EVENT_SCHEMA,
      runId: run.runId,
      seq: run.nextSeq,
      ts: nowIso(),
      runner: run.runner,
      phase: 'complete',
      type: 'ceremony.run_completed',
      status: 'complete',
      message: 'Ceremony completed',
      visibility: 'public',
      payload: {},
    };
    run.nextSeq += 1;
    run.events.push(event);
    if (run.events.length > CEREMONY_BUFFER_CAP) {
      run.events.splice(0, run.events.length - CEREMONY_BUFFER_CAP);
    }
    broadcastCeremonyEvent(run, event);
    broadcast = true;
  }
  persistCeremonyRuns().catch((error) => logLine(`Ceremony runs persist failed: ${error.message}`));
  return { run, broadcast };
}

// Persist completed-run metadata with private payloads stripped. Best-effort:
// never lets a disk write break the ceremony flow.
async function persistCeremonyRuns() {
  const runs = [];
  for (const run of ceremonyRuns.values()) {
    if (run.status !== 'completed') continue;
    runs.push({
      runId: run.runId,
      createdAt: run.createdAt,
      status: run.status,
      runner: run.runner,
      ownerName: run.ownerName,
      mode: run.mode,
      lastSeq: run.nextSeq - 1,
      events: run.events.filter((e) => isRenderableVisibility(e.visibility)).map(renderableCeremonyEvent),
    });
  }
  await writeJson(ceremonyRunsPath, { version: 1, schema: CEREMONY_EVENT_SCHEMA, updatedAt: nowIso(), runs }, 0o600);
}

function ceremonyRunSummary(run) {
  return {
    runId: run.runId,
    createdAt: run.createdAt,
    status: run.status,
    ownerName: run.ownerName,
    mode: run.mode,
    runner: run.runner,
    lastSeq: run.nextSeq - 1,
    events: run.events.filter((e) => isRenderableVisibility(e.visibility)).length,
  };
}

function isLoopbackSocket(socket) {
  const addr = socket.remoteAddress;
  if (!addr) return false;
  return addr === '127.0.0.1' || addr === '::1' || addr === '::ffff:127.0.0.1';
}

// Same-origin browser clients only. Non-browser clients (no Origin) are
// permitted so skill runners and check scripts can connect directly.
function ceremonyOriginAllowed(req) {
  const origin = req.headers.origin;
  if (!origin) return true;
  let parsed;
  try { parsed = new URL(origin); } catch { return false; }
  return parsed.hostname === '127.0.0.1' || parsed.hostname === 'localhost' || parsed.hostname === '::1';
}

function sendWsFrame(ws, frame) {
  if (ws.readyState === 1) ws.send(JSON.stringify(frame));
}

// Routes a ceremony WebSocket upgrade. Returns true if the upgrade was
// claimed (accepted or rejected); false if it is not a ceremony path.
function handleCeremonyUpgrade(req, socket, head, wss) {
  const url = new URL(req.url, `http://${req.headers.host || `${host}:${requestedPort}`}`);
  if (url.pathname !== '/api/ceremony/ws') return false;
  if (!isLoopbackSocket(socket)) {
    socket.destroy();
    return true;
  }
  if (!ceremonyOriginAllowed(req)) {
    socket.destroy();
    return true;
  }
  const runId = url.searchParams.get('runId');
  const afterRaw = url.searchParams.get('after');
  const after = Number.isInteger(Number(afterRaw)) && Number(afterRaw) >= 0 ? Number(afterRaw) : 0;
  let run = runId ? ceremonyRuns.get(runId) : undefined;
  wss.handleUpgrade(req, socket, head, (ws) => {
    if (!run) {
      sendWsFrame(ws, { kind: 'error', error: 'run not found', runId: runId || null });
      ws.close(4404, 'run not found');
      return;
    }
    sendWsFrame(ws, { kind: 'hello', schema: CEREMONY_WS_SCHEMA, runId: run.runId });
    const snapshotEvents = run.events
      .filter((e) => isRenderableVisibility(e.visibility) && e.seq > after)
      .map(renderableCeremonyEvent);
    sendWsFrame(ws, {
      kind: 'snapshot',
      runId: run.runId,
      lastSeq: run.nextSeq - 1,
      events: snapshotEvents,
    });
    run.subscribers.add(ws);
    ws.on('message', (data) => {
      // V1 ignores all client frames except ping; no mutation over WS.
      try {
        const msg = JSON.parse(data.toString());
        if (msg && msg.kind === 'ping') sendWsFrame(ws, { kind: 'heartbeat', ts: nowIso() });
      } catch {
        // Ignore malformed client frames.
      }
    });
    const detach = () => run.subscribers.delete(ws);
    ws.on('close', detach);
    ws.on('error', detach);
  });
  return true;
}

// --- End ceremony protocol ------------------------------------------------
async function handle(req, res) {
  try {
    const url = new URL(req.url, `http://${req.headers.host || `${host}:${requestedPort}`}`);
    if (await serveStatic(req, res, url)) return;
    if (req.method === 'GET' && url.pathname === '/') {
      const appIndex = path.join(repoRoot, 'ui', 'app', 'index.html');
      if (await fileReadable(appIndex)) { await sendStatic(res, appIndex); return; }
      return send(res, 404, { ok: false, error: 'App page not found' });
    }
    if (req.method === 'GET' && (url.pathname === '/onboarding' || url.pathname === '/onboarding/')) {
      const indexPath = path.join(repoRoot, 'ui', 'onboarding', 'index.html');
      if (await fileReadable(indexPath)) { await sendStatic(res, indexPath); return; }
      return send(res, 404, { ok: false, error: 'Onboarding page not found' });
    }
    if (req.method === 'GET' && url.pathname === '/api/state') return send(res, 200, await currentState());
    if (req.method === 'POST' && url.pathname === '/api/config') return send(res, 200, { ok: true, config: redactConfig(await saveBasicConfig(await readBody(req))) });
    if (req.method === 'POST' && url.pathname === '/api/invite/redeem') return send(res, 200, { ok: true, ...(await redeemInviteCode(await readBody(req))), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/test') {
      const config = await loadConfig();
      const health = await fetch(`${config.serverUrl}/health`).then((r) => r.json());
      const info = await bedrockApi(config, '/api/v1/info');
      return send(res, 200, { ok: true, health, principal: info.principal });
    }
    if (req.method === 'POST' && url.pathname === '/api/folder-dialog') return send(res, 200, { ok: true, path: await chooseFolderDialog() });
    if (req.method === 'POST' && url.pathname === '/api/folders') return send(res, 200, { ok: true, ...(await addFolder(await readBody(req))), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/schedule') return send(res, 200, { ok: true, schedule: await setSchedule(await readBody(req)), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/preview') return send(res, 200, { ok: true, preview: await runPreview(), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/preview/ack') return send(res, 200, { ok: true, acknowledgement: await acknowledgePreview(), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/sync') return send(res, 200, { ok: true, run: await runSync('manual'), state: await currentState() });
    if (req.method === 'POST' && url.pathname === '/api/manifest') {
      const config = await loadConfig();
      return send(res, 200, { ok: true, manifestPath: await writePublicSourceManifest(config), state: await currentState() });
    }
    // --- Ceremony REST endpoints (Build Lane 1) ---
    if (req.method === 'POST' && url.pathname === '/api/ceremony/runs') {
      const run = createCeremonyRun(await readBody(req));
      return send(res, 200, { ok: true, run: ceremonyRunSummary(run) });
    }
    if (req.method === 'POST' && /^\/api\/ceremony\/runs\/[^/]+\/events$/.test(url.pathname)) {
      const runId = decodeURIComponent(url.pathname.split('/')[4]);
      const run = ceremonyRuns.get(runId);
      if (!run) return send(res, 404, { ok: false, error: 'run not found', runId });
      const body = await readBody(req);
      const incoming = Array.isArray(body && body.events)
        ? body.events
        : (body && typeof body.type === 'string' ? [body] : []);
      const accepted = appendCeremonyEvents(run, incoming);
      return send(res, 200, { ok: true, runId: run.runId, accepted: accepted.length, nextSeq: run.nextSeq });
    }
    if (req.method === 'GET' && /^\/api\/ceremony\/runs\/[^/]+$/.test(url.pathname)) {
      const runId = decodeURIComponent(url.pathname.split('/')[4]);
      const run = ceremonyRuns.get(runId);
      if (!run) return send(res, 404, { ok: false, error: 'run not found', runId });
      const events = run.events.filter((e) => isRenderableVisibility(e.visibility)).map(renderableCeremonyEvent);
      return send(res, 200, { ok: true, run: ceremonyRunSummary(run), events });
    }
    if (req.method === 'POST' && /^\/api\/ceremony\/runs\/[^/]+\/complete$/.test(url.pathname)) {
      const runId = decodeURIComponent(url.pathname.split('/')[4]);
      const run = ceremonyRuns.get(runId);
      if (!run) return send(res, 404, { ok: false, error: 'run not found', runId });
      const { run: completed, broadcast } = completeCeremonyRun(run);
      return send(res, 200, { ok: true, run: ceremonyRunSummary(completed), broadcast });
    }
    if (req.method === 'GET' && url.pathname === '/api/ceremony/ws') {
      return send(res, 426, { ok: false, error: 'Upgrade Required' });
    }
    return send(res, 404, { ok: false, error: 'Not found' });
  } catch (error) {
    return send(res, error.statusCode || 500, { ok: false, error: error.message, code: error.code, stack: process.env.NODE_ENV === 'development' ? error.stack : undefined });
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

// --- Ceremony WebSocket wiring (Build Lane 1) -----------------------------
const ceremonyWss = new WebSocketServer({ noServer: true });
server.on('upgrade', (req, socket, head) => {
  try {
    const claimed = handleCeremonyUpgrade(req, socket, head, ceremonyWss);
    if (!claimed) socket.destroy(); // no other upgrade paths in V1
  } catch (error) {
    logLine(`Ceremony WS upgrade failed: ${error.message}`);
    socket.destroy();
  }
});
const ceremonyHeartbeat = setInterval(() => {
  const ts = nowIso();
  for (const run of ceremonyRuns.values()) {
    for (const ws of run.subscribers) sendWsFrame(ws, { kind: 'heartbeat', ts });
  }
}, CEREMONY_HEARTBEAT_MS);
ceremonyHeartbeat.unref?.();
