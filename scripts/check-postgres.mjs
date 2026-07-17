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
      to_regclass('longhouse.auth_invites') AS auth_invites_table,
      to_regclass('longhouse.room_members') AS room_members_table
  `);
  const row = result.rows[0];
  // The deferred owner-guard trigger is load-bearing for room integrity;
  // table existence alone is too weak a readiness signal.
  let roomOwnerGuardTrigger = false;
  if (row.room_members_table) {
    const trigger = await pool.query(`
      SELECT 1 FROM pg_trigger t
      JOIN pg_class c ON c.oid = t.tgrelid
      JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'longhouse' AND c.relname = 'room_members'
        AND t.tgname = 'room_members_owner_guard_trg'
        AND t.tgdeferrable AND t.tginitdeferred AND t.tgenabled <> 'D'
    `);
    roomOwnerGuardTrigger = trigger.rows.length > 0;
  }
  // Gate-2 S2 blockers (007): idempotent-redeem and retained-actor columns.
  const bridgeColumns = (await pool.query(`
    SELECT
      EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'longhouse' AND table_name = 'auth_invites'
          AND column_name = 'redemption_id' AND data_type = 'uuid'
      ) AS invite_redemption_id,
      EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = 'longhouse' AND tablename = 'auth_invites'
          AND indexname = 'auth_invites_redemption_id_key'
          AND indexdef LIKE 'CREATE UNIQUE INDEX%'
          AND indexdef LIKE '%(redemption_id IS NOT NULL)%'
      ) AS invite_redemption_id_index,
      EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'longhouse' AND table_name = 'ledger_events'
          AND column_name = 'actor_member_id' AND data_type = 'uuid'
      ) AS ledger_actor_member_id
  `)).rows[0];
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
    roomMembersReady: Boolean(row.room_members_table) && roomOwnerGuardTrigger,
    roomOwnerGuardTrigger,
    bridgeAuthorityReady: Boolean(
      bridgeColumns.invite_redemption_id
      && bridgeColumns.invite_redemption_id_index
      && bridgeColumns.ledger_actor_member_id,
    ),
    bridgeAuthority: {
      inviteRedemptionId: bridgeColumns.invite_redemption_id,
      inviteRedemptionIdIndex: bridgeColumns.invite_redemption_id_index,
      ledgerActorMemberId: bridgeColumns.ledger_actor_member_id,
    },
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
      roomMembers: row.room_members_table,
    },
    sourceAdaptersSeeded,
    semantic: semanticCounts,
    graph: graphCounts,
    ingestJobs: ingestCounts,
  }, null, 2));
} finally {
  await pool.end();
}
