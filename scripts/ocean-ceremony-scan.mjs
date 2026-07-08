#!/usr/bin/env node
// Ocean Bedrock Coworker Onboarding — real-scan ceremony scanner.
//
// Reads the REAL local agent configs (~/.claude, ~/.codex, ~/.pi, and ~/.omp
// when present), counts agents/skills/projects/mcp servers/docs/sessions, and
// emits the SAME ceremony event timeline (types + phases) as
// scripts/ocean-ceremony-fixture.mjs — but with REAL counts and names from
// this machine. Secrets are NEVER emitted as raw content: sensitive files are
// reported via `agent.secret_skipped` with `visibility:"redacted"` and a
// `displayName` only.
//
// Secret exclusion REUSES `src/ingest-filters.mjs` (DEFAULT_IGNORES +
// classifySensitivePath, the logic exercised by scripts/check-secret-globs.mjs)
// and adds the explicit auth-token-file set from the skill's forbidden list
// (~/.claude/.credentials.json, ~/.codex/auth.json, ~/.pi/agent/auth.json, …).
//
// Usage:
//   node scripts/ocean-ceremony-scan.mjs --dry-run
//   node scripts/ocean-ceremony-scan.mjs --base http://127.0.0.1:8765 --name 'Jane'
//   node scripts/ocean-ceremony-scan.mjs --runtime codex --name 'Codex'
//
// Modes:
//   --dry-run         Print each event as JSONL to stdout. No network. Exits 0
//                     after printing real acceptance counts.
//   (default, live)   POST /api/ceremony/runs, stream frames to
//                     /api/ceremony/runs/:runId/events, then drive the real
//                     preview/ack/sync pipeline.
//
// Human ack gate (default, live):
//   After emitting `ceremony.await_user gate=preview_ack`, the scanner polls
//   `GET /api/state` until `syncUnlocked === true` — i.e. a real human clicks
//   the unlock button in the Ocean stage (which calls POST /api/preview/ack
//   from the browser). The scanner NEVER calls /api/preview/ack itself; doing
//   so would be auto-acking. Use --ack-timeout <sec> to bound the wait
//   (default 300s); 0 = wait forever.
//
// Real work (live), per SKILL Steps 7-8:
//   - Before the preview gate: POST /api/preview (real dry-run). Best-effort.
//   - At the gate: pause + poll /api/state (human acks via the frontend).
//   - After unlock: POST /api/sync (real sync). Best-effort.
//   The ceremony timeline narrates; the real endpoints do the work — one thing.

import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {
  DEFAULT_IGNORES,
  classifySensitivePath,
  toPosixPath,
} from '../src/ingest-filters.mjs';

const ALLOWED_RUNTIMES = new Set(['claude-code', 'codex', 'pi', 'omp', 'other']);

// Display caps. Real totals are always reported in `source_detected.counts`;
// the per-item discovery stream is capped so the ceremony stays responsive on
// machines with hundreds of skills/projects.
const MAX_SKILL_EVENTS = 50;
const MAX_PROJECT_EVENTS = 50;
const MAX_MCP_EVENTS = 50;
const MAX_DOC_EVENTS = 24;
const MAX_SESSION_EVENTS = 40;
const MAX_SECRET_EVENTS = 60;
const MAX_QUEUED_ITEM_EVENTS = 60;

// Walk safety caps.
const WALK_MAX_DEPTH = 8;
const WALK_MAX_FILES = 30000;
const CODEX_ROLLOTT_CWD_CAP = 300;
const CODEX_ROLLOTT_READ_BYTES = 16384;

const HOME = os.homedir();

// --- Args -----------------------------------------------------------------

function parseArgs(argv) {
  const args = {
    dryRun: false,
    base: 'http://127.0.0.1:8765',
    runtime: 'other',
    name: 'Ocean Scan',
    owner: 'ocean-coworker',
    delayMs: 120,
    ackTimeoutSec: 300,
    complete: true,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => argv[i + 1];
    switch (a) {
      case '--dry-run': args.dryRun = true; break;
      case '--base': args.base = next(); i += 1; break;
      case '--runtime': args.runtime = next(); i += 1; break;
      case '--name': args.name = next(); i += 1; break;
      case '--owner': args.owner = next(); i += 1; break;
      case '--delay': args.delayMs = Number(next()); i += 1; break;
      case '--no-delay': args.delayMs = 0; break;
      case '--ack-timeout': args.ackTimeoutSec = Number(next()); i += 1; break;
      case '--no-complete': args.complete = false; break;
      case '-h':
      case '--help': {
        const help = [
          'Ocean ceremony real-scan',
          '',
          'Options:',
          '  --dry-run              Print events as JSONL, no network',
          '  --base <url>           Local app base URL (default http://127.0.0.1:8765)',
          '  --runtime <r>          claude-code|codex|pi|omp|other (default other)',
          '  --name <name>          Runner display name',
          '  --owner <name>         Owner name for the ceremony run',
          '  --delay <ms>           Inter-frame delay (default 120; --no-delay = 0)',
          '  --ack-timeout <sec>    Max seconds to wait for human ack (default 300; 0 = forever)',
          '  --no-complete          Do not POST /api/ceremony/runs/:runId/complete',
          '  -h, --help             Show this help',
          '',
          'Scans ~/.claude, ~/.codex, ~/.pi, and ~/.omp (when present) for real agents,',
          'skills, projects, MCP servers, docs, and sessions. Secrets are never emitted',
          'as raw content (redacted, displayName only).',
        ].join('\n');
        console.log(help);
        process.exit(0);
      }
      default:
        console.error(`Unknown argument: ${a}`);
        process.exit(2);
    }
  }
  if (!ALLOWED_RUNTIMES.has(args.runtime)) {
    console.error(`Invalid --runtime: ${args.runtime}. Must be one of: ${[...ALLOWED_RUNTIMES].join(', ')}`);
    process.exit(2);
  }
  if (!args.base) {
    console.error('--base is required');
    process.exit(2);
  }
  return args;
}

// --- Small helpers --------------------------------------------------------

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function pathExists(file) {
  try { await stat(file); return true; } catch { return false; }
}

async function readJsonSafe(file) {
  try {
    const raw = await readFile(file, 'utf8');
    return JSON.parse(raw);
  } catch { return null; }
}

async function readTextSafe(file) {
  try { return await readFile(file, 'utf8'); } catch { return null; }
}

async function postJSON(url, body) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body || {}),
  });
  const text = await res.text();
  let json = null;
  if (text) { try { json = JSON.parse(text); } catch { json = null; } }
  if (!res.ok) {
    const err = new Error(`POST ${url} -> ${res.status}: ${text || res.statusText}`);
    err.status = res.status;
    err.body = json;
    throw err;
  }
  return json;
}

async function getJSON(url) {
  const res = await fetch(url);
  const text = await res.text();
  let json = null;
  if (text) { try { json = JSON.parse(text); } catch { json = null; } }
  if (!res.ok) {
    const err = new Error(`GET ${url} -> ${res.status}: ${text || res.statusText}`);
    err.status = res.status;
    throw err;
  }
  return json;
}

// Auth/token files forbidden by the skill, on top of classifySensitivePath.
// These are matched by basename (case-insensitive) so they are caught even
// though the generic secret patterns in ingest-filters do not flag them.
const AUTH_TOKEN_FILES = new Set([
  '.credentials.json',
  'credentials.json',
  'auth.json',
  'auth_tokens.json',
  'setup-token.json',
  'token.json',
]);

function classifyScanSecret(relPath, name) {
  const rel = toPosixPath(relPath);
  const lc = String(name || '').toLowerCase();
  if (AUTH_TOKEN_FILES.has(lc)) return { sensitive: true, reason: 'auth-token-file' };
  if (/sk-ant-oat01/i.test(String(name || ''))) return { sensitive: true, reason: 'auth-token-file' };
  const found = classifySensitivePath(rel);
  if (found.sensitive) return { sensitive: true, reason: found.matchedPattern };
  return { sensitive: false, reason: null };
}

function secretDisplayName(label, rel) {
  const base = toPosixPath(rel).split('/').filter(Boolean).at(-1) || rel;
  return `${label} ${base}`.trim();
}

// Decode a Claude/Pi/OMP encoded session/project directory into a friendly
// project name. These dirs encode absolute paths with '/' -> '-'.
function decodeProjectName(encoded, dropPrefix = 2) {
  const s = String(encoded || '').replace(/^-+/, '');
  const segs = s.split('-').filter(Boolean);
  const tail = segs.length > dropPrefix ? segs.slice(dropPrefix) : segs;
  let name = tail.join('-') || s;
  // Drop a leading 'dev-' / 'code-' segment for cleaner display.
  name = name.replace(/^(dev|code|src|projects|Documents)-/, '');
  return name || s;
}

// --- Classified walker ----------------------------------------------------
// Yields non-ignored, classified entries under `root`. Sensitive files and the
// top of sensitive directories are surfaced as { sensitive:true } so callers
// can record redacted skips; sensitive directories are pruned (not descended).

async function* walkClassified(root) {
  let files = 0;
  async function* recurse(dir, depth) {
    if (depth > WALK_MAX_DEPTH) return;
    let entries;
    try { entries = await readdir(dir, { withFileTypes: true }); }
    catch { return; }
    for (const entry of entries) {
      if (files > WALK_MAX_FILES) return;
      const name = entry.name;
      if (DEFAULT_IGNORES.includes(name)) continue;
      const full = path.join(dir, name);
      const rel = toPosixPath(path.relative(root, full));
      const sec = classifyScanSecret(rel, name);
      if (sec.sensitive) {
        yield { kind: 'sensitive', full, rel, name, reason: sec.reason, isDir: entry.isDirectory() };
        if (entry.isDirectory()) continue; // prune sensitive directory
        continue;
      }
      if (entry.isDirectory()) {
        yield* recurse(full, depth + 1);
      } else if (entry.isFile()) {
        files += 1;
        yield { kind: 'file', full, rel, name };
      }
    }
  }
  yield* recurse(root, 0);
}

// --- MCP config parsers (names/counts only; contents never emitted) -------

async function parseClaudeMcp() {
  const out = [];
  const seen = new Set();
  const add = (name, scope, transport) => {
    if (!name || seen.has(name)) return;
    seen.add(name);
    out.push({ name, scope, transport });
  };
  const dotClaude = await readJsonSafe(path.join(HOME, '.claude.json'));
  if (dotClaude && typeof dotClaude === 'object') {
    const top = dotClaude.mcpServers || {};
    for (const name of Object.keys(top)) add(name, 'user', inferTransport(top[name]));
    const projects = dotClaude.projects || {};
    for (const proj of Object.values(projects)) {
      const ms = proj && proj.mcpServers ? proj.mcpServers : {};
      for (const name of Object.keys(ms)) add(name, 'project', inferTransport(ms[name]));
    }
  }
  const settings = await readJsonSafe(path.join(HOME, '.claude', 'settings.json'));
  if (settings && settings.mcpServers) {
    for (const name of Object.keys(settings.mcpServers)) add(name, 'user', inferTransport(settings.mcpServers[name]));
  }
  return out;
}

async function parseCodexMcp(root) {
  const text = await readTextSafe(path.join(root, 'config.toml'));
  if (!text) return [];
  const out = [];
  const re = /^\[mcp_servers\.([^\].\s]+)\]/gm;
  let m;
  while ((m = re.exec(text)) !== null) {
    const name = m[1];
    // Scan a window after the header for a transport hint.
    const window = text.slice(m.index, m.index + 400);
    let transport = 'stdio';
    if (/^\s*url\s*=/m.test(window) || /^\s*transport\s*=\s*"http"/m.test(window)) transport = 'http';
    out.push({ name, scope: 'user', transport });
  }
  return out;
}

async function parsePiMcp(root) {
  // ~/.pi historically used ~/.pi/mcp.json; current layout uses ~/.pi/agent/mcp.json.
  const candidates = [
    path.join(root, 'mcp.json'),
    path.join(root, 'agent', 'mcp.json'),
  ];
  for (const file of candidates) {
    const data = await readJsonSafe(file);
    const servers = (data && (data.mcpServers || data.servers)) || {};
    if (Object.keys(servers).length) {
      return Object.keys(servers).map((name) => ({ name, scope: 'user', transport: inferTransport(servers[name]) }));
    }
  }
  return [];
}

async function parseOmpMcp(root) {
  // OMP keeps config in agent/config.yml (YAML). Tolerant line scan only; we
  // never emit contents, just server names.
  const text = await readTextSafe(path.join(root, 'agent', 'config.yml'));
  if (!text) return [];
  const out = [];
  const re = /^[\s-]{0,4}name:\s*['"]?([A-Za-z0-9_.:-]+)['"]?\s*$/gm;
  let m;
  while ((m = re.exec(text)) !== null) out.push({ name: m[1], scope: 'user', transport: 'stdio' });
  return out;
}

function inferTransport(server) {
  if (!server || typeof server !== 'object') return 'stdio';
  if (typeof server.transport === 'string') return server.transport;
  if (typeof server.url === 'string') return 'http';
  if (typeof server.command === 'string') return 'stdio';
  return 'stdio';
}

// --- Per-source scanners --------------------------------------------------

function newResult(src) {
  return {
    ...src,
    counts: { agents: 0, skills: 0, projects: 0, mcpServers: 0 },
    skills: [],
    _skillSeen: new Set(),
    projects: [],
    mcps: [],
    docs: [],
    sessionSummaries: [],
    secrets: [],
  };
}

function addSecret(result, rel, reason) {
  if (result.secrets.length >= MAX_SECRET_EVENTS) return;
  const key = `${result.sourceId}:${rel}`;
  if (result._secretSeen?.has(key)) return;
  (result._secretSeen ||= new Set()).add(key);
  result.secrets.push({
    sourceId: result.sourceId,
    relPath: rel,
    reason: reason || 'sensitive',
    displayName: secretDisplayName(result.label, rel),
  });
}

function addSkill(result, name, relPath) {
  // Count EVERY unique skill discovered so source_detected.counts.skills is the
  // true total (not the capped list length), while only the first
  // MAX_SKILL_EVENTS become discovery events. Dedup is global per source via a
  // persistent set, so a skill found by both a targeted scan and the shared
  // walk is counted and emitted once.
  if (!name) return;
  const seen = (result._skillSeen ||= new Set());
  if (seen.has(name)) return;
  seen.add(name);
  if (result.skills.length < MAX_SKILL_EVENTS) {
    result.skills.push({
      sourceId: result.sourceId,
      name,
      relPath,
      color: skillColor(result.sourceId),
    });
  }
}

async function scanSourceShared(result, root) {
  // Walks the root, recording secrets and harvesting any session jsonl counts
  // and skill SKILL.md files not already captured by source-specific logic.
  for await (const ent of walkClassified(root)) {
    if (ent.kind === 'sensitive') {
      addSecret(result, ent.rel, ent.reason);
      continue;
    }
    const rel = ent.rel;
    const name = ent.name;
    // Skills: any SKILL.md not already added by the targeted scan.
    if (name === 'SKILL.md' || name.endsWith('.skill.md')) {
      addSkill(result, path.basename(path.dirname(ent.full)), rel);
    }
  }
}

function skillColor(sourceId) {
  return sourceId === 'pi' ? 'violet' : 'cyan';
}

async function scanClaude(src) {
  const result = newResult(src);
  const root = src.root;

  // Agents: top-level ~/.claude/agents/*.md
  const agentsDir = path.join(root, 'agents');
  if (await pathExists(agentsDir)) {
    try {
      const files = await readdir(agentsDir);
      for (const f of files) {
        if (f.endsWith('.md')) result.counts.agents += 1;
      }
    } catch { /* ignore */ }
  }

  // Projects: ~/.claude/projects/<encoded>/ dirs; sessions = *.jsonl per dir.
  const projectsDir = path.join(root, 'projects');
  if (await pathExists(projectsDir)) {
    try {
      const dirs = await readdir(projectsDir, { withFileTypes: true });
      for (const d of dirs) {
        if (!d.isDirectory()) continue;
        const encoded = d.name;
        const projRoot = path.join(projectsDir, encoded);
        let sessionCount = 0;
        try {
          const inner = await readdir(projRoot);
          for (const f of inner) if (f.endsWith('.jsonl')) sessionCount += 1;
        } catch { /* ignore */ }
        result.counts.projects += 1;
        if (result.projects.length < MAX_PROJECT_EVENTS) {
          result.projects.push({
            sourceId: src.sourceId,
            name: decodeProjectName(encoded, 2),
            root: `~/.claude/projects/${encoded}`,
            configDocs: [],
            sessions: sessionCount,
          });
        }
        if (sessionCount > 0 && result.sessionSummaries.length < MAX_SESSION_EVENTS) {
          result.sessionSummaries.push({
            sourceId: src.sourceId,
            project: decodeProjectName(encoded, 2),
            count: sessionCount,
          });
        }
      }
    } catch { /* ignore */ }
  }

  // MCP servers.
  result.mcps = (await parseClaudeMcp()).map((m) => ({ ...m, sourceId: src.sourceId }));

  // Skills + sessions + secrets via shared classified walk.
  await scanSourceShared(result, root);

  // Docs (names only).
  await addDoc(result, root, 'CLAUDE.md');
  await addDoc(result, root, 'AGENTS.md');

  finalizeCounts(result);
  return result;
}

async function scanCodex(src) {
  const result = newResult(src);
  const root = src.root;

  // Agents (rare on Codex).
  const agentsDir = path.join(root, 'agents');
  if (await pathExists(agentsDir)) {
    try {
      const files = await readdir(agentsDir);
      for (const f of files) if (f.endsWith('.md')) result.counts.agents += 1;
    } catch { /* ignore */ }
  }

  // MCP servers from config.toml.
  result.mcps = (await parseCodexMcp(root)).map((m) => ({ ...m, sourceId: src.sourceId }));

  // Sessions + projects from rollout files (cwd lives in a metadata record).
  const sessionsDir = path.join(root, 'sessions');
  const cwdCounts = new Map();
  if (await pathExists(sessionsDir)) {
    let rolloutFiles = [];
    try {
      const stack = [sessionsDir];
      while (rolloutFiles.length < CODEX_ROLLOTT_CWD_CAP && stack.length) {
        const dir = stack.pop();
        let entries;
        try { entries = await readdir(dir, { withFileTypes: true }); }
        catch { continue; }
        for (const e of entries) {
          if (e.isDirectory()) stack.push(path.join(dir, e.name));
          else if (e.name.startsWith('rollout-') && e.name.endsWith('.jsonl')) rolloutFiles.push(path.join(dir, e.name));
        }
      }
    } catch { /* ignore */ }
    for (const file of rolloutFiles) {
      try {
        const buf = await readFile(file);
        const head = buf.subarray(0, CODEX_ROLLOTT_READ_BYTES).toString('utf8');
        const cwdMatch = head.match(/"cwd"\s*:\s*"([^"]+)"/);
        if (cwdMatch) {
          const c = cwdMatch[1];
          cwdCounts.set(c, (cwdCounts.get(c) || 0) + 1);
        }
      } catch { /* ignore */ }
    }
  }

  result.counts.projects = cwdCounts.size;
  for (const [cwd, count] of cwdCounts) {
    const name = path.basename(cwd) || cwd;
    if (result.projects.length < MAX_PROJECT_EVENTS) {
      result.projects.push({
        sourceId: src.sourceId,
        name,
        root: `~${path.sep}${path.relative(HOME, cwd) || name}`,
        configDocs: [],
        sessions: count,
      });
    }
    if (result.sessionSummaries.length < MAX_SESSION_EVENTS) {
      result.sessionSummaries.push({ sourceId: src.sourceId, project: name, count });
    }
  }

  // Skills + secrets via shared classified walk (finds plugin SKILL.md too).
  await scanSourceShared(result, root);

  // Docs.
  await addDoc(result, root, 'AGENTS.md');
  await addDoc(result, root, 'config.toml');

  finalizeCounts(result);
  return result;
}

async function scanPi(src) {
  const result = newResult(src);
  // Real ~/.pi layout keeps config under ~/.pi/agent.
  const agentDir = path.join(src.root, 'agent');
  const root = (await pathExists(agentDir)) ? agentDir : src.root;
  result.scanRoot = root;

  // Skills: <root>/skills/*/SKILL.md
  await collectSkillsFromDir(result, path.join(root, 'skills'));

  // Sessions + projects: <root>/sessions/<encoded>/*.jsonl
  await collectEncodedSessions(result, path.join(root, 'sessions'), 1);

  // MCP servers from mcp.json.
  result.mcps = (await parsePiMcp(src.root)).map((m) => ({ ...m, sourceId: src.sourceId }));

  // Agents: <root>/agents/*.md (if present).
  const agentsDir = path.join(root, 'agents');
  if (await pathExists(agentsDir)) {
    try {
      const files = await readdir(agentsDir);
      for (const f of files) if (f.endsWith('.md')) result.counts.agents += 1;
    } catch { /* ignore */ }
  }

  // Shared walk for skills/secrets/docs across the root (catches run-history,
  // stray .env, keys, etc.).
  await scanSourceShared(result, root);

  await addDoc(result, root, 'AGENTS.md');
  await addDoc(result, root, 'SYSTEM.md');

  finalizeCounts(result);
  return result;
}

async function scanOmp(src) {
  const result = newResult(src);
  const agentDir = path.join(src.root, 'agent');
  const root = (await pathExists(agentDir)) ? agentDir : src.root;
  result.scanRoot = root;

  // Agents: <root>/agents/*.md
  const agentsDir = path.join(root, 'agents');
  if (await pathExists(agentsDir)) {
    try {
      const files = await readdir(agentsDir);
      for (const f of files) if (f.endsWith('.md')) result.counts.agents += 1;
    } catch { /* ignore */ }
  }

  // Skills: <root>/managed-skills/* (each dir is a managed skill).
  const skillsDir = path.join(root, 'managed-skills');
  if (await pathExists(skillsDir)) {
    try {
      const dirs = await readdir(skillsDir, { withFileTypes: true });
      for (const d of dirs) {
        if (!d.isDirectory()) continue;
        addSkill(result, d.name, `agent/managed-skills/${d.name}`);
      }
    } catch { /* ignore */ }
  }

  // Sessions + projects: <root>/sessions/<encoded>/...
  await collectEncodedSessions(result, path.join(root, 'sessions'), 1);

  // MCP servers.
  result.mcps = (await parseOmpMcp(src.root)).map((m) => ({ ...m, sourceId: src.sourceId }));

  // Shared walk for skills/secrets.
  await scanSourceShared(result, root);

  await addDoc(result, root, 'AGENTS.md');
  await addDoc(result, root, 'CLAUDE.md');

  finalizeCounts(result);
  return result;
}

async function collectSkillsFromDir(result, dir) {
  if (!(await pathExists(dir))) return;
  try {
    const dirs = await readdir(dir, { withFileTypes: true });
    for (const d of dirs) {
      if (!d.isDirectory()) continue;
      const skillName = d.name;
      const skillRoot = path.join(dir, skillName);
      // A skill is a dir containing SKILL.md (or any .md).
      let hasSkill = false;
      try {
        const inner = await readdir(skillRoot);
        hasSkill = inner.some((f) => f === 'SKILL.md' || f.endsWith('.md'));
      } catch { /* ignore */ }
      if (hasSkill) {
        addSkill(result, skillName, toPosixPath(path.relative(result.scanRoot || result.root, path.join(skillRoot, 'SKILL.md'))));
      }
    }
  } catch { /* ignore */ }
}

async function collectEncodedSessions(result, dir, dropPrefix) {
  if (!(await pathExists(dir))) return;
  try {
    const dirs = await readdir(dir, { withFileTypes: true });
    for (const d of dirs) {
      if (!d.isDirectory()) continue;
      const encoded = d.name;
      const projRoot = path.join(dir, encoded);
      let sessionCount = 0;
      try {
        const inner = await readdir(projRoot);
        for (const f of inner) if (f.endsWith('.jsonl')) sessionCount += 1;
      } catch { /* ignore */ }
      result.counts.projects += 1;
      if (result.projects.length < MAX_PROJECT_EVENTS) {
        result.projects.push({
          sourceId: result.sourceId,
          name: decodeProjectName(encoded, dropPrefix),
          root: toPosixPath(path.relative(result.root, projRoot)),
          configDocs: [],
          sessions: sessionCount,
        });
      }
      if (sessionCount > 0 && result.sessionSummaries.length < MAX_SESSION_EVENTS) {
        result.sessionSummaries.push({
          sourceId: result.sourceId,
          project: decodeProjectName(encoded, dropPrefix),
          count: sessionCount,
        });
      }
    }
  } catch { /* ignore */ }
}

async function addDoc(result, dir, name) {
  const file = path.join(dir, name);
  if (await pathExists(file)) {
    if (result.docs.length < MAX_DOC_EVENTS) {
      result.docs.push({
        sourceId: result.sourceId,
        name,
        relPath: toPosixPath(path.relative(result.root, file)) || name,
      });
    }
  }
}

function finalizeCounts(result) {
  result.counts.skills = result._skillSeen ? result._skillSeen.size : result.skills.length;
  result.counts.projects = Math.max(result.counts.projects, result.projects.length);
  result.counts.mcpServers = result.mcps.length;
}

// --- Orchestrate the scan -------------------------------------------------

async function runScan() {
  const sources = [
    { sourceId: 'claude-code', label: 'Claude Code', logo: 'claude', root: path.join(HOME, '.claude'), scanner: scanClaude },
    { sourceId: 'codex', label: 'Codex', logo: 'codex', root: path.join(HOME, '.codex'), scanner: scanCodex },
    { sourceId: 'pi', label: 'Pi', logo: 'pi', root: path.join(HOME, '.pi'), scanner: scanPi },
    { sourceId: 'omp', label: 'OMP', logo: 'omp', root: path.join(HOME, '.omp'), scanner: scanOmp },
  ];
  const results = [];
  for (const src of sources) {
    if (!(await pathExists(src.root))) continue;
    try {
      const res = await src.scanner(src);
      results.push(res);
    } catch (err) {
      // A failed source must not abort the whole ceremony.
      console.error(`[scan] ${src.sourceId}: scan failed (${err.message}); skipping`);
    }
  }
  return results;
}

// --- Frame builder (mirrors ocean-ceremony-fixture.mjs timeline) ----------

function ev(type, phase, status, message, payload, visibility) {
  const event = { type, phase, status, message, payload };
  if (visibility) event.visibility = visibility;
  return event;
}

function single(event, hook) {
  return { kind: 'single', event, hook };
}

function batch(events, hook) {
  return { kind: 'batch', events, hook };
}

// Round-robin interleave across sources so every source is represented in a
// capped discovery stream (Claude's volume must not crowd out Codex/Pi/OMP).
function interleaveBySource(results, pick, cap) {
  const groups = results.map((r) => pick(r) || []);
  const out = [];
  let added = true;
  let idx = 0;
  while (added && out.length < cap) {
    added = false;
    for (const g of groups) {
      if (idx < g.length) {
        out.push(g[idx]);
        added = true;
        if (out.length >= cap) break;
      }
    }
    idx += 1;
  }
  return out;
}

function buildFrames(results, options = {}) {
  const runner = { runtime: options.runtime || 'other', name: options.name || 'Ocean Scan' };
  const ownerName = options.ownerName || 'ocean-coworker';
  const frames = [];
  const counts = {};
  const bump = (type) => { counts[type] = (counts[type] || 0) + 1; };
  const track = (event) => { bump(event.type); return event; };

  // --- Boot / scene -------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.run_started',
    'boot', 'started',
    `Onboarding ceremony started for ${ownerName}`,
    { ownerName, runner, mode: 'onboarding-import-v1' },
  ))));

  frames.push(single(track(ev(
    'ceremony.scene_set',
    'boot', 'progress',
    'Welcome to Ocean',
    { scene: 'welcome', focus: 'globe', tone: 'calm_build' },
  ))));

  // --- Scan scene ---------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'scan', 'progress',
    'Scanning your agent constellations',
    { scene: 'constellation_scan', focus: 'globe', tone: 'calm_build' },
  ))));

  const sourceLabels = results.map((r) => r.label).filter(Boolean);
  frames.push(single(track(ev(
    'ceremony.loading_set',
    'scan', 'progress',
    `Looking for ${sourceLabels.length ? sourceLabels.join(', ') : 'local agent'} configs`,
    { label: 'Scanning local agent configs', indeterminate: true },
  ))));

  // source_detected (one per real source).
  for (const r of results) {
    frames.push(single(track(ev(
      'agent.source_detected',
      'scan', 'complete',
      `${r.label} setup detected`,
      {
        sourceId: r.sourceId,
        label: r.label,
        logo: r.logo,
        root: r.root.replace(HOME, '~'),
        counts: { ...r.counts },
      },
    ))));
  }

  // Secrets skipped — early, so the user sees safety in real time.
  const allSecrets = results.flatMap((r) => r.secrets);
  if (allSecrets.length) {
    frames.push(batch(allSecrets.map((s) => track(ev(
      'agent.secret_skipped',
      'scan', 'info',
      `Sensitive ${s.displayName} skipped`,
      { sourceId: s.sourceId, relPath: s.relPath, reason: s.reason, displayName: s.displayName },
      'redacted',
    )))));
  }
  // Skills.
  const skillsShown = interleaveBySource(results, (r) => r.skills, MAX_SKILL_EVENTS);
  if (skillsShown.length) {
    frames.push(batch(skillsShown.map((skill) => track(ev(
      'agent.skill_detected',
      'scan', 'progress',
      `Skill found: ${skill.name}`,
      {
        sourceId: skill.sourceId,
        name: skill.name,
        relPath: skill.relPath,
        display: { orbit: 'skills', color: skill.color },
        import: {
          eligible: true,
          targetPath: `/context/ocean-bedrock/imports/${skill.sourceId}/skills/${skill.name}/SKILL.md`,
        },
      },
    )))));
  }
  const allSkills = results.flatMap((r) => r.skills);

  // Projects.
  const allProjects = results.flatMap((r) => r.projects);
  const projectsShown = interleaveBySource(results, (r) => r.projects, MAX_PROJECT_EVENTS);
  if (projectsShown.length) {
    frames.push(batch(projectsShown.map((proj) => track(ev(
      'agent.project_detected',
      'scan', 'progress',
      `Project found: ${proj.name}`,
      {
        sourceId: proj.sourceId,
        name: proj.name,
        root: proj.root,
        configDocs: [...(proj.configDocs || [])],
        sessions: proj.sessions || 0,
        import: { eligible: true, targetPath: `/sessions/${ownerName}/imported/${proj.sourceId}/${proj.name}/` },
      },
    )))));
  }

  // MCP servers.
  const mcpsShown = interleaveBySource(results, (r) => r.mcps, MAX_MCP_EVENTS);
  if (mcpsShown.length) {
    frames.push(batch(mcpsShown.map((mcp) => track(ev(
      'agent.mcp_detected',
      'scan', 'progress',
      `MCP server found: ${mcp.name}`,
      {
        sourceId: mcp.sourceId,
        name: mcp.name,
        transport: mcp.transport,
        scope: mcp.scope,
        import: { eligible: true },
      },
    )))));
  }
  const allMcps = results.flatMap((r) => r.mcps);

  // Docs (names only).
  const docsShown = interleaveBySource(results, (r) => r.docs, MAX_DOC_EVENTS);
  if (docsShown.length) {
    frames.push(batch(docsShown.map((doc) => track(ev(
      'agent.doc_detected',
      'scan', 'progress',
      `Doc found: ${doc.name}`,
      {
        sourceId: doc.sourceId,
        name: doc.name,
        relPath: doc.relPath,
        import: { eligible: true },
      },
    )))));
  }
  const allDocs = results.flatMap((r) => r.docs);

  // Sessions (counts only).
  const sessionsShown = interleaveBySource(results, (r) => r.sessionSummaries, MAX_SESSION_EVENTS);
  if (sessionsShown.length) {
    frames.push(batch(sessionsShown.map((session) => track(ev(
      'agent.session_detected',
      'scan', 'progress',
      `Sessions found: ${session.count} for ${session.project}`,
      {
        sourceId: session.sourceId,
        project: session.project,
        count: session.count,
        import: { eligible: true, targetPath: `/sessions/${ownerName}/imported/${session.sourceId}/${session.project}/` },
      },
    )))));
  }
  const allSessions = results.flatMap((r) => r.sessionSummaries);

  // --- Review scene -------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'review', 'progress',
    'Reviewing discovered sources',
    { scene: 'source_review', focus: 'sources', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ceremony.toast',
    'review', 'info',
    'Auth files and secrets were skipped automatically',
    { tone: 'success', text: `No tokens or keychain items imported (${allSecrets.length} skipped).` },
  ))));

  // --- Import plan + preview ---------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'import', 'progress',
    'Preparing import preview',
    { scene: 'import_stream', focus: 'stream', tone: 'calm_build' },
  ))));

  const queuedItems = [
    ...allSkills.map((s) => ({ kind: 'skill', sourceId: s.sourceId, name: s.name, relPath: s.relPath })),
    ...allProjects.map((p) => ({ kind: 'project', sourceId: p.sourceId, name: p.name, root: p.root })),
    ...allMcps.map((m) => ({ kind: 'mcp', sourceId: m.sourceId, name: m.name })),
  ];

  frames.push(single(track(ev(
    'import.plan_started',
    'import', 'started',
    `Import plan built for ${queuedItems.length} items`,
    { itemCount: queuedItems.length, sources: results.map((s) => s.sourceId) },
  ))));

  frames.push(batch(queuedItems.slice(0, MAX_QUEUED_ITEM_EVENTS).map((item) => track(ev(
    'import.item_queued',
    'import', 'progress',
    `Queued ${item.kind}: ${item.name}`,
    {
      kind: item.kind,
      sourceId: item.sourceId,
      name: item.name,
      relPath: item.relPath || item.root,
      import: { eligible: true },
    },
  )))));

  if (allSecrets.length) {
    frames.push(batch(allSecrets.map((s) => track(ev(
      'import.item_skipped',
      'import', 'info',
      `Skipped sensitive ${s.displayName}`,
      { sourceId: s.sourceId, relPath: s.relPath, reason: s.reason },
      'redacted',
    )))));
  }

  frames.push(single(track(ev(
    'import.plan_completed',
    'import', 'complete',
    'Import preview ready for review',
    { queued: queuedItems.length, skipped: allSecrets.length },
  ))));

  // --- Connect / invite + folders ----------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'connect', 'progress',
    'Connecting Ocean',
    { scene: 'connect_ocean', focus: 'globe', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ocean.invite_redeemed',
    'connect', 'complete',
    'Invite redeemed',
    { ownerName },
  ))));

  const repoRoot = process.cwd();
  frames.push(single(track(ev(
    'ocean.folder_added',
    'connect', 'progress',
    `Folder staged for import: ${repoRoot}`,
    { path: repoRoot, label: path.basename(repoRoot) },
  ))));

  // --- Preview gate (HUMAN ack) ------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'preview', 'progress',
    'Dry-run preview ready',
    { scene: 'preview_gate', focus: 'ack_button', tone: 'calm_build' },
  ))));

  // Hook: run the real dry-run preview before narrating it.
  frames.push(single(track(ev(
    'ocean.preview_started',
    'preview', 'started',
    'Dry-run preview generated',
    { itemCount: queuedItems.length },
  )), 'run_preview'));

  frames.push(single(track(ev(
    'ocean.preview_completed',
    'preview', 'complete',
    'Preview complete — awaiting your review',
    { queued: queuedItems.length, skipped: allSecrets.length },
  ))));

  frames.push(single(track(ev(
    'ocean.ack_required',
    'preview', 'blocked',
    'Acknowledge the preview to unlock sync',
    { gate: 'preview_ack' },
  ))));

  frames.push(single(track(ev(
    'ceremony.highlight',
    'preview', 'info',
    'Review the preview, then click the unlock button',
    { target: 'ack_button' },
  ))));

  // THE human ack gate. The scanner pauses here and waits for a real human
  // click surfaced through /api/state syncUnlocked=true. Never auto-ack.
  frames.push(single(track(ev(
    'ceremony.await_user',
    'ack', 'blocked',
    'Waiting for you to acknowledge the preview',
    {
      gate: 'preview_ack',
      instructions: 'In the Ocean stage, click "I reviewed this preview — unlock sync". The scanner resumes automatically.',
    },
  )), 'await_ack'));

  frames.push(single(track(ev(
    'ocean.ack_observed',
    'ack', 'complete',
    'Preview acknowledged by user — sync unlocked',
    { gate: 'preview_ack' },
  )), 'ack_observed'));

  // --- Sync ---------------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'sync', 'progress',
    'Syncing your context into Ocean',
    { scene: 'sync_stream', focus: 'stream', tone: 'calm_build' },
  ))));

  // Hook: run the real sync.
  frames.push(single(track(ev(
    'ocean.sync_started',
    'sync', 'started',
    'Sync started',
    { items: queuedItems.length },
  )), 'run_sync'));

  frames.push(single(track(ev(
    'ceremony.loading_set',
    'sync', 'progress',
    'Streaming import into Ocean',
    { label: 'Syncing', progress: 1, total: queuedItems.length },
  ))));

  frames.push(single(track(ev(
    'ocean.sync_completed',
    'sync', 'complete',
    'Sync complete',
    { items: queuedItems.length },
  ))));

  // --- Complete -----------------------------------------------------------
  frames.push(single(track(ev(
    'ceremony.scene_set',
    'complete', 'complete',
    'Onboarding complete',
    { scene: 'complete', focus: 'globe', tone: 'calm_build' },
  ))));

  frames.push(single(track(ev(
    'ceremony.run_completed',
    'complete', 'complete',
    `Onboarding ceremony complete for ${ownerName}`,
    { ownerName, sources: results.map((s) => s.sourceId) },
  ))));

  return { frames, counts, runner, ownerName, results };
}

// --- Dry run --------------------------------------------------------------

async function dryRun(built) {
  for (const frame of built.frames) {
    if (frame.kind === 'single') {
      process.stdout.write(`${JSON.stringify(frame.event)}\n`);
    } else {
      for (const event of frame.events) process.stdout.write(`${JSON.stringify(event)}\n`);
    }
  }
  console.error('\n[dry-run] Real scan counts per source:');
  for (const r of built.results) {
    console.error(`  ${r.label.padEnd(12)} agents=${r.counts.agents} skills=${r.counts.skills} projects=${r.counts.projects} mcpServers=${r.counts.mcpServers}  (root ${r.root.replace(HOME, '~')})`);
  }
  console.error('\n[dry-run] Event counts:');
  for (const [type, n] of Object.entries(built.counts).sort()) {
    console.error(`  ${type}: ${n}`);
  }
  console.error(`[dry-run] Sources: ${built.results.length}. Secrets skipped (redacted): ${built.results.reduce((a, r) => a + r.secrets.length, 0)}.`);
  console.error('[dry-run] No network calls made.');
}

// --- Live run -------------------------------------------------------------

function isAwaitAckFrame(frame) {
  return frame.kind === 'single' && frame.hook === 'await_ack';
}

async function waitForSyncUnlocked(base, timeoutSec, onTick) {
  const deadline = timeoutSec > 0 ? Date.now() + timeoutSec * 1000 : Infinity;
  let lastMsg = 0;
  for (;;) {
    let state;
    try {
      state = await getJSON(`${base}/api/state`);
    } catch (err) {
      if (Date.now() > deadline) throw new Error(`ack wait aborted; /api/state error: ${err.message}`);
      await sleep(2000);
      continue;
    }
    if (state && state.syncUnlocked === true) return true;
    if (Date.now() > deadline) return false;
    if (onTick && Date.now() - lastMsg > 4000) {
      lastMsg = Date.now();
      onTick(state, Math.max(0, deadline - Date.now()));
    }
    await sleep(1500);
  }
}

async function liveRun(args, built) {
  const { frames, counts, runner, ownerName } = built;

  // 1. Create the ceremony run.
  const created = await postJSON(`${args.base}/api/ceremony/runs`, {
    runner,
    ownerName,
    mode: 'onboarding-import-v1',
  });
  if (!created || !created.ok || !created.run || !created.run.runId) {
    throw new Error(`Failed to create ceremony run: ${JSON.stringify(created)}`);
  }
  const { runId } = created.run;
  console.error(`[live] Ceremony run created: ${runId}`);

  const eventsUrl = `${args.base}/api/ceremony/runs/${runId}/events`;
  let accepted = 0;

  let humanAcked = false;
  let gateReached = false;
  for (const frame of frames) {
    // Before narrating the preview: run the real dry-run preview.
    if (frame.hook === 'run_preview') {
      try {
        const preview = await postJSON(`${args.base}/api/preview`, {});
        console.error(`[live] POST /api/preview ok=${preview && preview.ok}`);
      } catch (err) {
        console.error(`[live] POST /api/preview unavailable (${err.message}); narrating preview only.`);
      }
    }

    // Never narrate an ack/sync that did not happen: after a gate timeout,
    // drop the ack_observed and sync-phase frames instead of lying.
    const frameType = frame.kind === 'single' ? frame.event?.type : frame.events?.[0]?.type;
    const framePhase = frame.kind === 'single' ? frame.event?.phase : frame.events?.[0]?.phase;
    if (gateReached && !humanAcked && (frameType === 'ocean.ack_observed' || framePhase === 'sync')) {
      console.error(`[live] -- skipped ${frameType} (no confirmed human ack)`);
      continue;
    }

    const body = frame.kind === 'single' ? frame.event : { events: frame.events };
    const res = await postJSON(eventsUrl, body);
    if (!res || !res.ok) throw new Error(`Event POST failed: ${JSON.stringify(res)}`);
    accepted += res.accepted || (frame.kind === 'single' ? 1 : frame.events.length);
    const summary = frame.kind === 'single'
      ? frame.event.type
      : `batch(${frame.events.length}x ${frame.events[0] && frame.events[0].type})`;
    console.error(`[live] -> ${summary}  (accepted=${res.accepted}, nextSeq=${res.nextSeq})`);

    // At the human ack gate: pause and poll. NEVER call /api/preview/ack.
    if (isAwaitAckFrame(frame)) {
      console.error('[live] Paused at preview ack gate. Waiting for human acknowledgement...');
      console.error('[live]   (the scanner never auto-acks; unlock happens in the Ocean stage)');
      gateReached = true;
      const unlocked = await waitForSyncUnlocked(args.base, args.ackTimeoutSec, (state, remainingMs) => {
        const remain = args.ackTimeoutSec > 0 ? Math.round(remainingMs / 1000) : null;
        console.error(`[live]   still waiting for syncUnlocked=true (currently=${state && state.syncUnlocked}${remain !== null ? `, ${remain}s left` : ''})...`);
      });
      humanAcked = unlocked;
      if (!unlocked) {
        console.error('[live] Ack timeout reached. Continuing to completion without a confirmed human ack (ceremony events only).');
      } else {
        console.error('[live] syncUnlocked=true detected. Resuming after ack gate.');
      }
    }

    // After ack observed: run the real sync.
    if (frame.hook === 'run_sync' && humanAcked) {
      try {
        const syncRes = await postJSON(`${args.base}/api/sync`, {});
        console.error(`[live] POST /api/sync ok=${syncRes && syncRes.ok}`);
      } catch (err) {
        console.error(`[live] POST /api/sync unavailable (${err.message}); narrating sync only.`);
      }
    } else if (frame.hook === 'run_sync') {
      console.error('[live] Skipping POST /api/sync: the ack gate was never confirmed by a human.');
    }

    if (args.delayMs > 0) await sleep(args.delayMs);
  }

  // 2. Complete the run.
  if (args.complete) {
    try {
      const done = await postJSON(`${args.base}/api/ceremony/runs/${runId}/complete`, {});
      console.error(`[live] Run completed: ok=${done && done.ok}, broadcast=${done && done.broadcast}`);
    } catch (err) {
      console.error(`[live] complete call failed: ${err.message}`);
    }
  }

  console.error(`\n[live] Done. runId=${runId} accepted=${accepted}`);
  console.error('[live] Event counts:');
  for (const [type, n] of Object.entries(counts).sort()) {
    console.error(`  ${type}: ${n}`);
  }
}

// --- Main -----------------------------------------------------------------

async function main() {
  const args = parseArgs(process.argv.slice(2));
  console.error(`[scan] Scanning real agent configs (${['~/.claude', '~/.codex', '~/.pi', '~/.omp'].join(', ')})...`);
  const results = await runScan();
  if (!results.length) {
    console.error('[scan] No agent config sources found. Nothing to onboard.');
    process.exit(1);
  }
  const built = buildFrames(results, { runtime: args.runtime, name: args.name, ownerName: args.owner });

  if (args.dryRun) {
    await dryRun(built);
    return;
  }

  console.error(`[live] Streaming real-scan ceremony -> ${args.base}`);
  console.error(`[live] runtime=${args.runtime} owner=${args.owner} sources=${results.length} frames=${built.frames.length}`);
  try {
    await liveRun(args, built);
  } catch (err) {
    if (err && (err.code === 'ECONNREFUSED' || /\bECONNREFUSED\b/.test(err.message))) {
      console.error(`\n[live] Could not reach ${args.base}. Is the local app running?`);
      console.error('[live] Start it with:  npm run ocean:app');
      console.error('[live] Or run in dry-run mode:  node scripts/ocean-ceremony-scan.mjs --dry-run');
      process.exit(1);
    }
    console.error(`\n[live] Error: ${err.message}`);
    process.exit(1);
  }
}

main();
