#!/usr/bin/env node
import { listFailedIngestJobs, requeueIngestJob, abandonIngestJob } from '../src/metadata.mjs';
import { createLedgerStore } from '../src/ledger.mjs';

function usage() {
  console.log(`Usage:
  npm run ocean:jobs                          # list failed ingest jobs
  npm run ocean:jobs -- list
  npm run ocean:jobs -- inspect <id>
  npm run ocean:jobs -- retry <id>
  npm run ocean:jobs -- retry --all [--reset-attempts] [--yes]
  npm run ocean:jobs -- abandon <id> --reason <text>
  npm run ocean:jobs -- abandon --all --reason <text> [--yes]

Subcommands:
  list                List failed ingest jobs (id, job_type, virtual_path, error).
  inspect <id>        Print the full row including payload.
  retry <id>          Requeue a single failed job (status -> queued).
  retry --all         Dry-run: print jobs that would be requeued. Add --yes to mutate.
  abandon <id>        Mark a single failed job cancelled with the given reason.
  abandon --all       Dry-run: print jobs that would be cancelled. Add --yes to mutate.

Options:
  --reset-attempts    When retrying, also reset attempts counter to 0.
  --reason <text>     Required for abandon; recorded as the cancellation reason.
  --yes               Required to execute bulk (--all) mutations; otherwise dry-run only.
  --limit <n>         Max rows to show for list/abandon-all/retry-all. Default: 50.
`);
}

function parseArgs(argv) {
  const args = { subcommand: 'list', all: false, yes: false, resetAttempts: false, limit: 50 };
  let i = 0;
  if (i < argv.length && !argv[i].startsWith('--')) {
    args.subcommand = argv[i++];
  }
  for (; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') { args.help = true; }
    else if (arg === '--all') { args.all = true; }
    else if (arg === '--yes') { args.yes = true; }
    else if (arg === '--reset-attempts') { args.resetAttempts = true; }
    else if (arg === '--reason') { args.reason = argv[++i]; }
    else if (arg === '--limit') { args.limit = Math.max(1, Number(argv[++i])); }
    else if (!args.id && !arg.startsWith('--')) { args.id = arg; }
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function fmt(row) {
  return {
    id: row.id,
    job_type: row.job_type,
    status: row.status,
    virtual_path: row.virtual_path,
    attempts: row.attempts,
    max_attempts: row.max_attempts,
    error: row.error,
    updated_at: row.updated_at,
  };
}

async function appendAuditEvent(ledger, eventType, payload) {
  if (!ledger) return;
  await ledger.append({
    event_type: eventType,
    lab: 'ocean-context',
    actor_type: 'agent',
    actor_id: 'operator',
    actor_name: 'ocean-jobs-cli',
    source_id: 'ocean-bedrock',
    payload,
    clearance: 'CONFIDENTIAL',
    tags: ['ingest', 'job-management'],
  }).catch((err) => {
    console.error('[ocean:jobs] ledger append failed (non-fatal):', err.message);
  });
}

if (!process.env.DATABASE_URL) {
  console.error('DATABASE_URL is required. Run via: railway run -- node scripts/ocean-failed-jobs.mjs');
  process.exit(1);
}

const args = parseArgs(process.argv.slice(2));
if (args.help) { usage(); process.exit(0); }

const ledger = process.env.DATABASE_URL ? createLedgerStore() : null;

if (args.subcommand === 'list') {
  const rows = await listFailedIngestJobs({ limit: args.limit });
  if (!rows.length) { console.log('No failed ingest jobs.'); process.exit(0); }
  for (const row of rows) console.log(JSON.stringify(fmt(row)));
  console.log(`\n${rows.length} failed job(s).`);

} else if (args.subcommand === 'inspect') {
  if (!args.id) { console.error('inspect requires <id>'); process.exit(1); }
  const rows = await listFailedIngestJobs({ limit: 1000 });
  const row = rows.find((r) => r.id === args.id);
  if (!row) { console.error(`Job ${args.id} not found in failed jobs.`); process.exit(1); }
  console.log(JSON.stringify(row, null, 2));

} else if (args.subcommand === 'retry') {
  if (args.all) {
    const rows = await listFailedIngestJobs({ limit: args.limit });
    if (!rows.length) { console.log('No failed jobs to retry.'); process.exit(0); }
    if (!args.yes) {
      console.log(`DRY RUN — would requeue ${rows.length} job(s) (add --yes to mutate):`);
      for (const row of rows) console.log(`  ${row.id}  ${row.virtual_path}  [${row.error}]`);
      process.exit(0);
    }
    let ok = 0;
    for (const row of rows) {
      const updated = await requeueIngestJob(row.id, { resetAttempts: args.resetAttempts });
      if (updated) {
        ok += 1;
        console.log(`requeued: ${row.id}  ${row.virtual_path}`);
        await appendAuditEvent(ledger, 'ocean_context.ingest.job_requeued', { jobId: row.id, virtualPath: row.virtual_path, resetAttempts: args.resetAttempts });
      } else {
        console.warn(`skipped (not failed?): ${row.id}`);
      }
    }
    console.log(`\nRequeued ${ok}/${rows.length}.`);
  } else {
    if (!args.id) { console.error('retry requires <id> or --all'); process.exit(1); }
    const updated = await requeueIngestJob(args.id, { resetAttempts: args.resetAttempts });
    if (!updated) { console.error(`Job ${args.id} not found in failed state.`); process.exit(1); }
    console.log(JSON.stringify(fmt(updated)));
    await appendAuditEvent(ledger, 'ocean_context.ingest.job_requeued', { jobId: args.id, virtualPath: updated.virtual_path, resetAttempts: args.resetAttempts });
  }

} else if (args.subcommand === 'abandon') {
  if (!args.reason) { console.error('abandon requires --reason <text>'); process.exit(1); }
  if (args.all) {
    const rows = await listFailedIngestJobs({ limit: args.limit });
    if (!rows.length) { console.log('No failed jobs to abandon.'); process.exit(0); }
    if (!args.yes) {
      console.log(`DRY RUN — would cancel ${rows.length} job(s) (add --yes to mutate):`);
      for (const row of rows) console.log(`  ${row.id}  ${row.virtual_path}  [${row.error}]`);
      process.exit(0);
    }
    let ok = 0;
    for (const row of rows) {
      const updated = await abandonIngestJob(row.id, args.reason);
      if (updated) {
        ok += 1;
        console.log(`abandoned: ${row.id}  ${row.virtual_path}`);
        await appendAuditEvent(ledger, 'ocean_context.ingest.job_abandoned', { jobId: row.id, virtualPath: row.virtual_path, reason: args.reason });
      } else {
        console.warn(`skipped (not failed?): ${row.id}`);
      }
    }
    console.log(`\nAbandoned ${ok}/${rows.length}.`);
  } else {
    if (!args.id) { console.error('abandon requires <id> or --all'); process.exit(1); }
    const updated = await abandonIngestJob(args.id, args.reason);
    if (!updated) { console.error(`Job ${args.id} not found in failed state.`); process.exit(1); }
    console.log(JSON.stringify(fmt(updated)));
    await appendAuditEvent(ledger, 'ocean_context.ingest.job_abandoned', { jobId: args.id, virtualPath: updated.virtual_path, reason: args.reason });
  }

} else {
  console.error(`Unknown subcommand: ${args.subcommand}`);
  usage();
  process.exit(1);
}
