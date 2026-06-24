import path from 'node:path';
import crypto from 'node:crypto';

export function slugify(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'node';
}

export function stableRef(prefix, value) {
  return `${prefix}:${crypto.createHash('sha256').update(String(value)).digest('hex')}`;
}

export async function upsertGraphNode(db, node) {
  const result = await db.query(
    `INSERT INTO longhouse.graph_nodes (node_type, name, virtual_path, object_id, external_ref, properties)
     VALUES ($1, $2, $3, $4::uuid, $5, $6::jsonb)
     ON CONFLICT (node_type, external_ref) DO UPDATE SET
       name = EXCLUDED.name,
       virtual_path = COALESCE(EXCLUDED.virtual_path, longhouse.graph_nodes.virtual_path),
       object_id = COALESCE(EXCLUDED.object_id, longhouse.graph_nodes.object_id),
       properties = longhouse.graph_nodes.properties || EXCLUDED.properties,
       updated_at = now()
     RETURNING *`,
    [node.node_type, node.name || null, node.virtual_path || null, node.object_id || null, node.external_ref, JSON.stringify(node.properties || {})],
  );
  return result.rows[0];
}

export async function upsertGraphEdge(db, edge) {
  const result = await db.query(
    `INSERT INTO longhouse.graph_edges (from_node_id, to_node_id, predicate, weight, source_object_id, properties)
     VALUES ($1::uuid, $2::uuid, $3, $4, $5::uuid, $6::jsonb)
     ON CONFLICT (from_node_id, to_node_id, predicate) DO UPDATE SET
       weight = EXCLUDED.weight,
       source_object_id = COALESCE(EXCLUDED.source_object_id, longhouse.graph_edges.source_object_id),
       properties = longhouse.graph_edges.properties || EXCLUDED.properties,
       created_at = longhouse.graph_edges.created_at
     RETURNING *`,
    [edge.from_node_id, edge.to_node_id, edge.predicate, edge.weight ?? 1, edge.source_object_id || null, JSON.stringify(edge.properties || {})],
  );
  return result.rows[0];
}

export function extractHeadings(text, limit = 80) {
  const headings = [];
  const lines = String(text || '').split(/\r?\n/);
  for (const line of lines) {
    const match = line.match(/^(#{1,6})\s+(.+?)\s*#*$/);
    if (!match) continue;
    headings.push({ level: match[1].length, title: match[2].trim() });
    if (headings.length >= limit) break;
  }
  return headings;
}

export function extractTags(text, limit = 80) {
  const tags = new Set();
  const regex = /(^|\s)#([a-zA-Z][a-zA-Z0-9_-]{2,50})\b/g;
  let match;
  while ((match = regex.exec(String(text || ''))) && tags.size < limit) tags.add(match[2].toLowerCase());
  return [...tags];
}

export function extractLinks(text, limit = 80) {
  const links = [];
  const regex = /\[[^\]]+\]\(([^)\s]+)(?:\s+"[^"]+")?\)/g;
  let match;
  while ((match = regex.exec(String(text || ''))) && links.length < limit) links.push(match[1]);
  return links;
}

export function pathPrefixes(virtualPath) {
  const parts = String(virtualPath || '').split('/').filter(Boolean);
  const prefixes = [];
  for (let i = 1; i < parts.length; i += 1) prefixes.push(`/${parts.slice(0, i).join('/')}`);
  return prefixes;
}

export async function extractGraphForObject(db, objectId) {
  const objectResult = await db.query('SELECT * FROM longhouse.objects WHERE id = $1', [objectId]);
  const object = objectResult.rows[0];
  if (!object || object.deleted_at) return { skipped: true, reason: 'object missing or deleted' };

  const chunksResult = await db.query('SELECT chunk_text FROM longhouse.embedding_chunks WHERE object_id = $1 ORDER BY chunk_index ASC LIMIT 200', [objectId]);
  const text = chunksResult.rows.map((row) => row.chunk_text).join('\n\n');
  const fileNode = await upsertGraphNode(db, {
    node_type: 'file',
    name: path.posix.basename(object.virtual_path),
    virtual_path: object.virtual_path,
    object_id: object.id,
    external_ref: object.virtual_path,
    properties: { contentType: object.content_type, sizeBytes: Number(object.size_bytes || 0), chunks: chunksResult.rows.length, graphExtractedAt: new Date().toISOString() },
  });

  let nodes = 1;
  let edges = 0;
  let previousDir = null;
  for (const prefix of pathPrefixes(object.virtual_path)) {
    const dirNode = await upsertGraphNode(db, {
      node_type: 'directory',
      name: path.posix.basename(prefix),
      virtual_path: prefix,
      external_ref: `directory:${prefix}`,
      properties: { path: prefix },
    });
    nodes += 1;
    if (previousDir) {
      await upsertGraphEdge(db, { from_node_id: previousDir.id, to_node_id: dirNode.id, predicate: 'contains', source_object_id: object.id });
      edges += 1;
    }
    previousDir = dirNode;
  }
  if (previousDir) {
    await upsertGraphEdge(db, { from_node_id: previousDir.id, to_node_id: fileNode.id, predicate: 'contains', source_object_id: object.id });
    edges += 1;
  }

  for (const heading of extractHeadings(text)) {
    const headingNode = await upsertGraphNode(db, {
      node_type: 'heading',
      name: heading.title,
      virtual_path: object.virtual_path,
      object_id: object.id,
      external_ref: `heading:${object.virtual_path}#${slugify(heading.title)}`,
      properties: { level: heading.level },
    });
    await upsertGraphEdge(db, { from_node_id: fileNode.id, to_node_id: headingNode.id, predicate: 'has_heading', source_object_id: object.id, properties: { level: heading.level } });
    nodes += 1;
    edges += 1;
  }

  for (const tag of extractTags(text)) {
    const topicNode = await upsertGraphNode(db, { node_type: 'topic', name: tag, external_ref: `topic:${tag}`, properties: { source: 'hashtag' } });
    await upsertGraphEdge(db, { from_node_id: fileNode.id, to_node_id: topicNode.id, predicate: 'mentions_topic', source_object_id: object.id });
    nodes += 1;
    edges += 1;
  }

  for (const link of extractLinks(text)) {
    const isVirtualPath = link.startsWith('/');
    const linkNode = await upsertGraphNode(db, {
      node_type: isVirtualPath ? 'file_ref' : 'external_link',
      name: isVirtualPath ? path.posix.basename(link) : link,
      virtual_path: isVirtualPath ? link : null,
      external_ref: isVirtualPath ? link : stableRef('external_link', link),
      properties: { href: link },
    });
    await upsertGraphEdge(db, { from_node_id: fileNode.id, to_node_id: linkNode.id, predicate: 'links_to', source_object_id: object.id, properties: { href: link } });
    nodes += 1;
    edges += 1;
  }

  const sourceRecords = await db.query(
    `SELECT sr.id, sr.source_record_id, sr.source_instance_id, si.adapter_id, si.name AS source_name
     FROM longhouse.source_records sr
     LEFT JOIN longhouse.source_instances si ON si.id = sr.source_instance_id
     WHERE sr.object_id = $1
     LIMIT 25`,
    [objectId],
  ).catch(() => ({ rows: [] }));

  for (const record of sourceRecords.rows) {
    const sourceNode = await upsertGraphNode(db, {
      node_type: 'source_instance',
      name: record.source_name || record.source_instance_id,
      external_ref: `source_instance:${record.source_instance_id}`,
      properties: { adapterId: record.adapter_id },
    });
    const recordNode = await upsertGraphNode(db, {
      node_type: 'source_record',
      name: record.source_record_id,
      virtual_path: object.virtual_path,
      object_id: object.id,
      external_ref: `source_record:${record.source_instance_id}:${record.source_record_id}`,
      properties: { sourceInstanceId: record.source_instance_id },
    });
    await upsertGraphEdge(db, { from_node_id: sourceNode.id, to_node_id: recordNode.id, predicate: 'emits_record', source_object_id: object.id });
    await upsertGraphEdge(db, { from_node_id: recordNode.id, to_node_id: fileNode.id, predicate: 'materializes_as', source_object_id: object.id });
    nodes += 2;
    edges += 2;
  }

  await db.query(
    `UPDATE longhouse.objects SET metadata = metadata || $2::jsonb, updated_at = now() WHERE id = $1`,
    [objectId, JSON.stringify({ graphExtractedAt: new Date().toISOString(), graphNodesTouched: nodes, graphEdgesTouched: edges })],
  );

  return { extracted: true, objectId, virtualPath: object.virtual_path, nodesTouched: nodes, edgesTouched: edges };
}

function positiveLimit(value, fallback = 100, max = 1000) {
  const number = Number(value || fallback);
  if (!Number.isFinite(number)) return fallback;
  return Math.max(1, Math.min(Math.floor(number), max));
}

export async function listGraphNodes(db, filters = {}) {
  const where = [];
  const values = [];
  function eq(column, value) {
    if (value === undefined || value === null || value === '') return;
    values.push(value);
    where.push(`${column} = $${values.length}`);
  }
  eq('node_type', filters.node_type || filters.type);
  if (filters.path && filters.path !== '/') {
    values.push(filters.path.replace(/\/$/, ''));
    where.push(`(virtual_path = $${values.length} OR virtual_path LIKE $${values.length + 1})`);
    values.push(`${filters.path.replace(/\/$/, '')}/%`);
  } else {
    where.push('virtual_path IS NOT NULL');
  }
  if (filters.q) {
    values.push(`%${filters.q}%`);
    where.push(`(name ILIKE $${values.length} OR external_ref ILIKE $${values.length})`);
  }
  values.push(positiveLimit(filters.limit));
  const limitParam = values.length;
  const result = await db.query(
    `SELECT * FROM longhouse.graph_nodes ${where.length ? `WHERE ${where.join(' AND ')}` : ''} ORDER BY updated_at DESC, created_at DESC LIMIT $${limitParam}`,
    values,
  );
  return result.rows;
}

export async function graphNeighborhood(db, filters = {}) {
  let start;
  if (filters.node_id || filters.nodeId) {
    const result = await db.query('SELECT * FROM longhouse.graph_nodes WHERE id = $1', [filters.node_id || filters.nodeId]);
    start = result.rows[0];
  } else if (filters.path) {
    const result = await db.query(
      `SELECT * FROM longhouse.graph_nodes WHERE virtual_path = $1 AND node_type IN ('file', 'directory', 'file_ref') ORDER BY CASE WHEN node_type = 'file' THEN 0 ELSE 1 END LIMIT 1`,
      [filters.path],
    );
    start = result.rows[0];
  }
  if (!start) return { start: null, nodes: [], edges: [] };

  const maxDepth = Math.max(1, Math.min(Number(filters.depth || 1), 3));
  const limit = positiveLimit(filters.limit, 100, 500);
  const nodeMap = new Map([[start.id, start]]);
  const edgeMap = new Map();
  let frontier = [start.id];

  for (let depth = 0; depth < maxDepth && frontier.length && nodeMap.size < limit; depth += 1) {
    const result = await db.query(
      `SELECT ge.*, from_node.id AS from_id, from_node.node_type AS from_type, from_node.name AS from_name,
              from_node.virtual_path AS from_virtual_path, from_node.external_ref AS from_external_ref,
              to_node.id AS to_id, to_node.node_type AS to_type, to_node.name AS to_name,
              to_node.virtual_path AS to_virtual_path, to_node.external_ref AS to_external_ref
       FROM longhouse.graph_edges ge
       JOIN longhouse.graph_nodes from_node ON from_node.id = ge.from_node_id
       JOIN longhouse.graph_nodes to_node ON to_node.id = ge.to_node_id
       WHERE ge.from_node_id = ANY($1::uuid[]) OR ge.to_node_id = ANY($1::uuid[])
       ORDER BY ge.created_at DESC
       LIMIT $2`,
      [frontier, limit],
    );
    const next = [];
    for (const row of result.rows) {
      edgeMap.set(row.id, {
        id: row.id,
        from_node_id: row.from_node_id,
        to_node_id: row.to_node_id,
        predicate: row.predicate,
        weight: row.weight,
        source_object_id: row.source_object_id,
        properties: row.properties || {},
        created_at: row.created_at,
      });
      for (const side of ['from', 'to']) {
        const node = {
          id: row[`${side}_id`],
          node_type: row[`${side}_type`],
          name: row[`${side}_name`],
          virtual_path: row[`${side}_virtual_path`],
          external_ref: row[`${side}_external_ref`],
        };
        if (!nodeMap.has(node.id)) {
          nodeMap.set(node.id, node);
          next.push(node.id);
        }
      }
    }
    frontier = next;
  }

  return { start, nodes: [...nodeMap.values()], edges: [...edgeMap.values()] };
}
