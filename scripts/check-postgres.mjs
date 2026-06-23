#!/usr/bin/env node
import { Pool } from 'pg';

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  console.error('DATABASE_URL is not set.');
  process.exit(1);
}

const pool = new Pool({ connectionString: databaseUrl });
try {
  const result = await pool.query(`
    SELECT
      current_database() AS database,
      current_user AS user,
      to_regclass('longhouse.mounts') AS mounts_table,
      to_regclass('longhouse.ledger_events') AS ledger_events_table,
      to_regclass('longhouse.context_snapshots') AS context_snapshots_table,
      to_regclass('longhouse.source_adapters') AS source_adapters_table,
      to_regclass('longhouse.source_instances') AS source_instances_table,
      to_regclass('longhouse.source_streams') AS source_streams_table,
      to_regclass('longhouse.source_sync_runs') AS source_sync_runs_table,
      to_regclass('longhouse.source_records') AS source_records_table,
      to_regclass('longhouse.embedding_chunks') AS embedding_chunks_table,
      to_regclass('longhouse.graph_nodes') AS graph_nodes_table,
      to_regclass('longhouse.graph_edges') AS graph_edges_table,
      to_regclass('longhouse.ingest_jobs') AS ingest_jobs_table,
      to_regclass('longhouse.auth_tokens') AS auth_tokens_table,
      to_regclass('longhouse.auth_invites') AS auth_invites_table
  `);
  const row = result.rows[0];
  let sourceAdaptersSeeded = 0;
  if (row.source_adapters_table) {
    const adapterCount = await pool.query(`
      SELECT count(*)::int AS count
      FROM longhouse.source_adapters
      WHERE id = ANY($1::text[])
    `, [[
      'local_folder',
      'github',
      'telegram',
      'slack',
      'notion',
      'linear',
      'google_drive',
      'r2',
    ]]);
    sourceAdaptersSeeded = adapterCount.rows[0]?.count || 0;
  }
  const sourceAdapterTablesReady = Boolean(
    row.source_adapters_table
    && row.source_instances_table
    && row.source_streams_table
    && row.source_sync_runs_table
    && row.source_records_table,
  );
  const semanticCounts = row.embedding_chunks_table ? (await pool.query(`
    SELECT
      count(*)::int AS chunks,
      count(*) FILTER (WHERE vectorize_id IS NOT NULL)::int AS vectorized,
      count(*) FILTER (WHERE vectorize_id IS NULL)::int AS pending
    FROM longhouse.embedding_chunks
  `)).rows[0] : null;
  const graphCounts = row.graph_nodes_table ? (await pool.query(`
    SELECT
      (SELECT count(*)::int FROM longhouse.graph_nodes) AS nodes,
      (SELECT count(*)::int FROM longhouse.graph_edges) AS edges
  `)).rows[0] : null;
  const ingestCounts = row.ingest_jobs_table ? (await pool.query(`
    SELECT status, count(*)::int AS count
    FROM longhouse.ingest_jobs
    GROUP BY status
    ORDER BY status
  `)).rows : [];
  console.log(JSON.stringify({
    ok: true,
    database: row.database,
    user: row.user,
    schemaReady: Boolean(row.mounts_table && row.ledger_events_table && row.context_snapshots_table && sourceAdapterTablesReady),
    authRegistryReady: Boolean(row.auth_tokens_table && row.auth_invites_table),
    tables: {
      mounts: row.mounts_table,
      ledgerEvents: row.ledger_events_table,
      contextSnapshots: row.context_snapshots_table,
      sourceAdapters: row.source_adapters_table,
      sourceInstances: row.source_instances_table,
      sourceStreams: row.source_streams_table,
      sourceSyncRuns: row.source_sync_runs_table,
      sourceRecords: row.source_records_table,
      embeddingChunks: row.embedding_chunks_table,
      graphNodes: row.graph_nodes_table,
      graphEdges: row.graph_edges_table,
      ingestJobs: row.ingest_jobs_table,
      authTokens: row.auth_tokens_table,
      authInvites: row.auth_invites_table,
    },
    sourceAdaptersSeeded,
    semantic: semanticCounts,
    graph: graphCounts,
    ingestJobs: ingestCounts,
  }, null, 2));
} finally {
  await pool.end();
}
