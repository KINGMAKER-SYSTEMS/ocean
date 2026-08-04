#!/usr/bin/env node
import { stdin, stdout, stderr } from 'node:process';

const SERVER_NAME = 'ocean-bedrock-mcp';
const SERVER_VERSION = '0.1.0';
const DEFAULT_TIMEOUT_MS = 60000;

function log(message) {
  stderr.write(`[${SERVER_NAME}] ${message}\n`);
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function baseUrl() {
  return (process.env.OCEAN_BEDROCK_URL || process.env.BEDROCK_URL || 'http://127.0.0.1:8080').replace(/\/$/, '');
}

function bearerToken() {
  return process.env.OCEAN_BEDROCK_TOKEN || process.env.BEDROCK_TOKEN || requiredEnv('OCEAN_BEDROCK_TOKEN');
}

async function bedrock(pathname, options = {}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), Number(process.env.OCEAN_BEDROCK_MCP_TIMEOUT_MS || DEFAULT_TIMEOUT_MS));
  try {
    const response = await fetch(`${baseUrl()}${pathname}`, {
      ...options,
      signal: controller.signal,
      headers: {
        Authorization: `Bearer ${bearerToken()}`,
        ...(options.headers || {}),
      },
    });
    const contentType = response.headers.get('content-type') || '';
    let body;
    if (contentType.includes('application/json')) body = await response.json();
    else if (contentType.startsWith('text/') || contentType.includes('yaml') || contentType.includes('xml')) body = await response.text();
    else body = Buffer.from(await response.arrayBuffer()).toString('base64');
    if (!response.ok) throw new Error(`${options.method || 'GET'} ${pathname} failed: ${response.status} ${JSON.stringify(body)}`);
    return { body, contentType, status: response.status };
  } finally {
    clearTimeout(timeout);
  }
}

function textContent(value) {
  return [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 2) }];
}

function encodePath(value = '/') {
  return encodeURIComponent(value);
}

const tools = [
  {
    name: 'bedrock_info',
    description: 'Return Ocean Bedrock instance info and current token principal.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
  },
  {
    name: 'bedrock_list',
    description: 'List files/directories at a scoped Ocean Bedrock virtual path.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', default: '/' },
        depth: { type: 'number', default: 1 },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_read',
    description: 'Read a file from Ocean Bedrock. Text files return text; binary files return base64 text.',
    inputSchema: {
      type: 'object',
      required: ['path'],
      properties: { path: { type: 'string' } },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_write',
    description: 'Write text content to an Ocean Bedrock file path.',
    inputSchema: {
      type: 'object',
      required: ['path', 'content'],
      properties: {
        path: { type: 'string' },
        content: { type: 'string' },
        contentType: { type: 'string', default: 'text/plain; charset=utf-8' },
        ifNoneMatch: { type: 'boolean', default: false },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_mkdir',
    description: 'Create an Ocean Bedrock directory recursively.',
    inputSchema: {
      type: 'object',
      required: ['path'],
      properties: { path: { type: 'string' } },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_search',
    description: 'Search scoped text files in Ocean Bedrock.',
    inputSchema: {
      type: 'object',
      required: ['query'],
      properties: {
        query: { type: 'string' },
        path: { type: 'string', default: '/' },
        limit: { type: 'number', default: 50 },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_semantic_search',
    description: 'Semantic Vectorize search over scoped Ocean Context chunks, with lexical fallback when embeddings are unavailable.',
    inputSchema: {
      type: 'object',
      required: ['query'],
      properties: {
        query: { type: 'string' },
        path: { type: 'string', default: '/' },
        limit: { type: 'number', default: 10 },
        mode: { type: 'string', enum: ['semantic', 'lexical'], default: 'semantic' },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_graph_neighborhood',
    description: 'Return graph nodes/edges around a scoped file or graph node.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string' },
        nodeId: { type: 'string' },
        depth: { type: 'number', default: 1 },
        limit: { type: 'number', default: 100 },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_toolbox_manifest',
    description: 'Return the Ocean Toolbox manifest for staging MCP, skills, local sync, and device auth.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
  },
  {
    name: 'bedrock_triage_daily',
    description: 'Run Ocean Context daily triage and write a report/ledger event.',
    inputSchema: {
      type: 'object',
      properties: {
        reportPath: { type: 'string' },
        correlationId: { type: 'string' },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_lock',
    description: 'Acquire a lock lease for a path before collaborative edits.',
    inputSchema: {
      type: 'object',
      required: ['path'],
      properties: {
        path: { type: 'string' },
        ttlSeconds: { type: 'number', default: 900 },
        note: { type: 'string' },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_unlock',
    description: 'Release a lock by lock id.',
    inputSchema: {
      type: 'object',
      required: ['lockId'],
      properties: { lockId: { type: 'string' } },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_trace',
    description: 'Trace Ocean Ledger events by correlation id.',
    inputSchema: {
      type: 'object',
      required: ['correlationId'],
      properties: {
        correlationId: { type: 'string' },
        limit: { type: 'number', default: 1000 },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'bedrock_snapshot',
    description: 'Create an Ocean Ledger context snapshot event.',
    inputSchema: {
      type: 'object',
      required: ['name'],
      properties: {
        name: { type: 'string' },
        correlationId: { type: 'string' },
        virtualPath: { type: 'string' },
        files: { type: 'array', items: { type: 'string' } },
        events: { type: 'array', items: { type: 'string' } },
        summary: { type: 'string' },
        clearance: { type: 'string', default: 'CONFIDENTIAL' },
        metadata: { type: 'object' },
      },
      additionalProperties: false,
    },
  },
];

async function callTool(name, args = {}) {
  if (name === 'bedrock_info') return textContent((await bedrock('/api/v1/info')).body);
  if (name === 'bedrock_list') return textContent((await bedrock(`/api/v1/tree?path=${encodePath(args.path || '/')}&depth=${Number(args.depth || 1)}`)).body);
  if (name === 'bedrock_read') return textContent((await bedrock(`/api/v1/file?path=${encodePath(args.path)}&inline=1`)).body);
  if (name === 'bedrock_write') {
    const headers = { 'content-type': args.contentType || 'text/plain; charset=utf-8' };
    if (args.ifNoneMatch) headers['if-none-match'] = '*';
    return textContent((await bedrock(`/api/v1/file?path=${encodePath(args.path)}`, { method: 'PUT', headers, body: args.content || '' })).body);
  }
  if (name === 'bedrock_mkdir') {
    return textContent((await bedrock('/api/v1/mkdir', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ path: args.path }) })).body);
  }
  if (name === 'bedrock_search') return textContent((await bedrock(`/api/v1/search?q=${encodeURIComponent(args.query)}&path=${encodePath(args.path || '/')}&limit=${Number(args.limit || 50)}`)).body);
  if (name === 'bedrock_semantic_search') return textContent((await bedrock(`/api/v1/semantic/search?q=${encodeURIComponent(args.query)}&path=${encodePath(args.path || '/')}&limit=${Number(args.limit || 10)}${args.mode === 'lexical' ? '&mode=lexical' : ''}`)).body);
  if (name === 'bedrock_graph_neighborhood') {
    const params = new URLSearchParams();
    if (args.path) params.set('path', args.path);
    if (args.nodeId) params.set('nodeId', args.nodeId);
    params.set('depth', String(args.depth || 1));
    params.set('limit', String(args.limit || 100));
    return textContent((await bedrock(`/api/v1/graph/neighborhood?${params}`)).body);
  }
  if (name === 'bedrock_toolbox_manifest') return textContent((await bedrock('/api/v1/toolbox/manifest')).body);
  if (name === 'bedrock_triage_daily') {
    return textContent((await bedrock('/api/v1/ocean-context/triage/daily', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ reportPath: args.reportPath || null, correlationId: args.correlationId || null }),
    })).body);
  }
  if (name === 'bedrock_lock') {
    return textContent((await bedrock('/api/v1/locks', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ path: args.path, ttlSeconds: args.ttlSeconds || 900, note: args.note || null }) })).body);
  }
  if (name === 'bedrock_unlock') return textContent((await bedrock(`/api/v1/locks/${encodeURIComponent(args.lockId)}`, { method: 'DELETE' })).body);
  if (name === 'bedrock_trace') return textContent((await bedrock(`/api/v1/ledger/trace?correlation_id=${encodeURIComponent(args.correlationId)}&limit=${Number(args.limit || 1000)}`)).body);
  if (name === 'bedrock_snapshot') {
    return textContent((await bedrock('/api/v1/ledger/snapshots', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        name: args.name,
        correlation_id: args.correlationId || null,
        virtual_path: args.virtualPath || null,
        files: args.files || [],
        events: args.events || [],
        summary: args.summary || null,
        clearance: args.clearance || 'CONFIDENTIAL',
        metadata: args.metadata || {},
      }),
    })).body);
  }
  throw new Error(`Unknown tool: ${name}`);
}

async function handle(request) {
  const { id, method, params = {} } = request;
  if (method === 'initialize') {
    return {
      jsonrpc: '2.0',
      id,
      result: {
        protocolVersion: params.protocolVersion || '2024-11-05',
        capabilities: { tools: {}, resources: {} },
        serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
      },
    };
  }
  if (method === 'notifications/initialized') return null;
  if (method === 'tools/list') return { jsonrpc: '2.0', id, result: { tools } };
  if (method === 'tools/call') {
    const content = await callTool(params.name, params.arguments || {});
    return { jsonrpc: '2.0', id, result: { content } };
  }
  if (method === 'resources/list') {
    return {
      jsonrpc: '2.0',
      id,
      result: {
        resources: ['docs', 'context', 'coworkers', 'sessions', 'handoffs'].map((root) => ({
          uri: `ocean-bedrock://${root}`,
          name: `/${root}`,
          description: `Ocean Bedrock /${root}`,
          mimeType: 'application/json',
        })),
      },
    };
  }
  if (method === 'resources/read') {
    const uri = params.uri || '';
    const root = uri.replace(/^ocean-bedrock:\/\//, '').replace(/^\/+/, '') || 'context';
    const body = (await bedrock(`/api/v1/tree?path=${encodePath(`/${root}`)}&depth=2`)).body;
    return {
      jsonrpc: '2.0',
      id,
      result: { contents: [{ uri, mimeType: 'application/json', text: JSON.stringify(body, null, 2) }] },
    };
  }
  return { jsonrpc: '2.0', id, error: { code: -32601, message: `Method not found: ${method}` } };
}

let buffer = '';
stdin.setEncoding('utf8');
stdin.on('data', (chunk) => {
  buffer += chunk;
  let newline;
  while ((newline = buffer.indexOf('\n')) !== -1) {
    const line = buffer.slice(0, newline).trim();
    buffer = buffer.slice(newline + 1);
    if (!line) continue;
    Promise.resolve()
      .then(() => handle(JSON.parse(line)))
      .then((response) => {
        if (response) stdout.write(`${JSON.stringify(response)}\n`);
      })
      .catch((error) => {
        let id = null;
        try { id = JSON.parse(line).id ?? null; } catch {}
        stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32000, message: error.message } })}\n`);
      });
  }
});

log(`ready; url=${baseUrl()}`);
