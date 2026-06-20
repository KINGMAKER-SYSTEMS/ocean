#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const DEFAULT_PATHS = ['README.md', 'docs', 'scripts', 'src', 'workflows', 'db', 'package.json'];
const paths = process.argv.slice(2).filter(Boolean);
const scanPaths = paths.length ? paths : DEFAULT_PATHS;

const patterns = [
  { name: 'OpenAI-style API key', re: /\bsk-[A-Za-z0-9][A-Za-z0-9_-]{20,}\b/g },
  { name: 'Slack token', re: /\bxox[baprs]-[A-Za-z0-9-]{20,}\b/g },
  { name: 'GitHub token', re: /\bgh[pousr]_[A-Za-z0-9_]{20,}\b/g },
  { name: 'AWS access key', re: /\bAKIA[0-9A-Z]{16}\b/g },
  { name: 'private key block', re: /-----BEGIN [A-Z ]*PRIVATE KEY-----/g },
  { name: 'Postgres URL with password', re: /postgres(?:ql)?:\/\/[^\s:@]+:[^\s@]+@[^\s]+/g },
  { name: 'Telegram bot token', re: /\b\d{6,}:[A-Za-z0-9_-]{20,}\b/g },
];

const allowlist = [
  /dev-token-change-me/,
  /validateSecretRef\('ghp_abcdefghijklmnopqrstuvwxyz1234567890'\)/,
  /secret-scan\.mjs/,
  /P0-TRACKING-MAP\.md/,
  /OCEAN-\d+\/obe-p0/,
];

function trackedFiles() {
  const out = execFileSync('git', ['ls-files', '--', ...scanPaths], { cwd: ROOT, encoding: 'utf8' });
  return out.split('\n').filter(Boolean);
}

function shouldSkip(file) {
  return file.includes('node_modules/') || file.includes('/.git/') || file.startsWith('.pi/') || file.startsWith('.worktrees/') || file.startsWith('.wrangler/');
}

const findings = [];
for (const file of trackedFiles()) {
  if (shouldSkip(file)) continue;
  let text;
  try {
    const stat = fs.statSync(path.join(ROOT, file));
    if (!stat.isFile() || stat.size > 2_000_000) continue;
    text = fs.readFileSync(path.join(ROOT, file), 'utf8');
  } catch {
    continue;
  }
  const lines = text.split(/\r?\n/);
  lines.forEach((line, idx) => {
    if (allowlist.some((re) => re.test(line) || re.test(file))) return;
    for (const pattern of patterns) {
      pattern.re.lastIndex = 0;
      if (pattern.re.test(line)) findings.push({ file, line: idx + 1, type: pattern.name });
    }
  });
}

if (findings.length) {
  console.error(JSON.stringify({ ok: false, findings }, null, 2));
  process.exit(1);
}

console.log(JSON.stringify({ ok: true, scannedFiles: trackedFiles().filter((f) => !shouldSkip(f)).length }, null, 2));
