#!/usr/bin/env node
// Gate-2 S1D protocol harness.
//
// The disposable-Postgres smoke owns the real protocol interactions and race
// fixtures. This harness composes its named proofs into the frozen Gate-2
// acceptance matrix, failing closed if any required proof disappears or the
// underlying smoke exits nonzero.

import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export const PROTOCOL_PROOFS = Object.freeze({
  'G2-AC1 membership and scope on every room operation': Object.freeze([
    'DB human row bound to redeemed principal id',
    'non-member path-only read 403',
    'admin without membership: stream 403',
    'removed member roster 403 membership_revoked',
    'removed member ledger write denied',
  ]),
  'G2-AC2 scoped single-use invite and owned members': Object.freeze([
    'derived contributor role',
    'derived exact scope',
    'redeem 201 contributor token',
    'double redeem 403',
    'self-join 201 {member}',
    'all-new batch 201, order preserved',
    'no-membership (foreign) agent mint 403',
    'cross-principal agent FK rejected',
  ]),
  'G2-AC3 authorized ascending pageable replay': Object.freeze([
    'after_sequence walk: strict ascending, no overlap, full stream, limit=2 bound respected on every page',
    'after_sequence numeric-in-string only: abc 400',
    'after_sequence u64-max: 200 empty, no bigint overflow',
    'non-member trace 403',
  ]),
  'G2-AC4 race-free bounded authenticated fanout': Object.freeze([
    'both first snapshot reads held',
    'mid-snapshot append accepted while both snapshots held',
    'subscriber A: EXACT visible-sequence completeness through live marker',
    'subscriber B: EXACT visible-sequence completeness through live marker',
    'resume from Last-Event-ID: strictly after cursor, no dupe',
    'overflow drain completed within bound',
    'overflow -> parsed resync_required: after_sequence == last accepted room_event id, then NOTHING after terminal',
    'no token material in ANY frame of ANY subscriber',
  ]),
  'G2-AC5 producer-scoped idempotency and conflict': Object.freeze([
    'two producer streams both source_sequence=1 succeed independently',
    'exact producer retry returns prior event',
    'different content same tuple 409 + details',
    'concurrent same-tuple same-content: one event, both 201',
  ]),
  'G2-AC10 open-stream and subsequent-operation revocation': Object.freeze([
    'member removal -> revoked membership_revoked',
    'token revocation -> revoked token_invalid',
    'removed member roster 403 membership_revoked',
    'removed principal same-token rejoin 403 membership_revoked',
    'removed member ledger write denied',
    'remove-vs-selfJoin: no resurrection',
  ]),
});

export function passedCheckNames(output) {
  const names = new Set();
  for (const line of output.split(/\r?\n/)) {
    const match = /^\s*✓\s+(.+?)\s*$/.exec(line);
    if (match) names.add(match[1]);
  }
  return names;
}

export function evaluateProtocolProofs(output) {
  const passed = passedCheckNames(output);
  const sections = Object.entries(PROTOCOL_PROOFS).map(([acceptanceCriterion, required]) => ({
    acceptanceCriterion,
    missing: required.filter((name) => !passed.has(name)),
  }));
  return {
    ok: sections.every(({ missing }) => missing.length === 0),
    sections,
  };
}

export function formatProtocolReport(report) {
  const lines = ['\nrooms-protocol-harness:'];
  for (const { acceptanceCriterion, missing } of report.sections) {
    lines.push(`  ${missing.length === 0 ? 'PASS' : 'FAIL'} ${acceptanceCriterion}`);
    for (const proof of missing) lines.push(`    missing: ${proof}`);
  }
  return lines.join('\n');
}

async function run() {
  let output = '';
  const child = spawn(process.execPath, [path.resolve(repoRoot, 'scripts/rooms-postgres-smoke.mjs')], {
    cwd: repoRoot,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  child.stdout.on('data', (chunk) => {
    const text = chunk.toString();
    output += text;
    process.stdout.write(text);
  });
  child.stderr.on('data', (chunk) => {
    const text = chunk.toString();
    output += text;
    process.stderr.write(text);
  });

  const exitCode = await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', resolve);
  });
  if (exitCode !== 0) {
    console.error(`rooms-protocol-harness: underlying PostgreSQL smoke exited ${exitCode}`);
    process.exitCode = 1;
    return;
  }

  const report = evaluateProtocolProofs(output);
  console.log(formatProtocolReport(report));
  process.exitCode = report.ok ? 0 : 1;
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : '';
if (import.meta.url === invokedPath) await run();
