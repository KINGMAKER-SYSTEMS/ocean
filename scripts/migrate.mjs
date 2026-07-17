#!/usr/bin/env node
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promises as fs } from 'node:fs';
import { Pool } from 'pg';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

const args = process.argv.slice(2);
if (args.includes('--help') || args.includes('-h')) {
  console.log(`Usage:
  DATABASE_URL=postgres://... npm run db:migrate -- --yes
  DATABASE_URL=postgres://... npm run db:migrate -- --yes db/001_longhouse_core.sql db/002_ocean_ledger.sql db/003_source_adapters.sql db/004_source_capabilities_truth.sql db/005_auth_registry.sql db/006_federated_rooms.sql db/007_federation_bridge_authority.sql

Runs SQL migrations against DATABASE_URL. Requires --yes intentionally.`);
  process.exit(0);
}

if (!args.includes('--yes')) {
  console.error('Refusing to run migrations without --yes. This modifies the target database.');
  process.exit(1);
}

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  console.error('DATABASE_URL is not set.');
  process.exit(1);
}

const files = args
  .filter((arg) => arg !== '--yes')
  .map((arg) => path.resolve(repoRoot, arg));

if (files.length === 0) {
  files.push(
    path.resolve(repoRoot, 'db/001_longhouse_core.sql'),
    path.resolve(repoRoot, 'db/002_ocean_ledger.sql'),
    path.resolve(repoRoot, 'db/003_source_adapters.sql'),
    path.resolve(repoRoot, 'db/004_source_capabilities_truth.sql'),
    path.resolve(repoRoot, 'db/005_auth_registry.sql'),
    path.resolve(repoRoot, 'db/006_federated_rooms.sql'),
    path.resolve(repoRoot, 'db/007_federation_bridge_authority.sql'),
  );
}

const pool = new Pool({ connectionString: databaseUrl });
try {
  for (const file of files) {
    const sql = await fs.readFile(file, 'utf8');
    console.log(`Applying ${path.relative(repoRoot, file)}...`);
    await pool.query(sql);
  }
  console.log('Migrations applied.');
} finally {
  await pool.end();
}
