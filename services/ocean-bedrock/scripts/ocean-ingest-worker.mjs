#!/usr/bin/env node
import path from 'node:path';
import { createOceanBedrockServer } from '../src/server.mjs';
import { claimNextIngestJob, completeIngestJob, processIngestJob } from '../src/metadata.mjs';

function usage() {
  console.log(`Usage:
  npm run ocean:worker
  npm run ocean:worker -- --once --limit 10

Processes queued longhouse.ingest_jobs for Ocean Bedrock-managed files.

Options:
  --once          Exit when queue is empty.
  --limit <n>     Max jobs to process before exit. Default: Infinity.
  --sleep-ms <n>  Poll sleep when queue is empty. Default: 5000.
`);
}

function parseArgs(argv) {
  const args = { once: false, limit: Infinity, sleepMs: 5000 };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--once') args.once = true;
    else if (arg === '--limit') args.limit = Math.max(1, Number(argv[++i]));
    else if (arg === '--sleep-ms') args.sleepMs = Math.max(100, Number(argv[++i]));
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  usage();
  process.exit(0);
}

if (!process.env.DATABASE_URL) {
  console.error('DATABASE_URL is required.');
  process.exit(1);
}

const { state, ready } = createOceanBedrockServer({
  root: process.env.OCEAN_BEDROCK_ROOT || process.env.RT_NAS_ROOT || path.join(process.cwd(), 'data'),
});
await ready;

const workerId = `ocean-bedrock-worker-${process.pid}`;
let processed = 0;
let idle = 0;

while (processed < args.limit) {
  const job = await claimNextIngestJob(workerId);
  if (!job) {
    if (args.once) break;
    idle += 1;
    if (idle === 1) console.log('queue empty; waiting');
    await sleep(args.sleepMs);
    continue;
  }
  idle = 0;

  try {
    const result = await processIngestJob(state, job);
    await completeIngestJob(job.id, 'done', null);
    processed += 1;
    console.log(JSON.stringify({ jobId: job.id, virtualPath: job.virtual_path, result }));
  } catch (error) {
    const terminal = Number(job.attempts || 0) >= Number(job.max_attempts || 5);
    await completeIngestJob(job.id, terminal ? 'failed' : 'queued', error.message);
    processed += 1;
    console.error(JSON.stringify({ jobId: job.id, virtualPath: job.virtual_path, error: error.message, retry: !terminal }));
  }
}

console.log(JSON.stringify({ ok: true, processed }));
