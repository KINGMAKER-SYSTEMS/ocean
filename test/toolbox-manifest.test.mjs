// Pins toolboxManifest() (src/ocean-context.mjs) to the tools the MCP server
// actually exports. The manifest is what a coworker's agent reads to learn
// which bedrock_* tools exist, and it had drifted: 11 hardcoded names vs 14
// exported by scripts/ocean-bedrock-mcp.mjs (missing bedrock_toolbox_manifest,
// bedrock_lock, bedrock_unlock). The MCP script starts a stdio JSON-RPC loop
// on import, so we grep its source for tool names instead of importing it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { toolboxManifest } from '../src/ocean-context.mjs';

const MCP_SCRIPT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'scripts', 'ocean-bedrock-mcp.mjs');

test('toolboxManifest lists exactly the tools the MCP script exports', () => {
  const source = readFileSync(MCP_SCRIPT, 'utf8');
  const exported = [...source.matchAll(/name: '(bedrock_\w+)'/g)].map((m) => m[1]);
  assert.ok(exported.length >= 14, `sanity: MCP script defines ${exported.length} bedrock_* tools`);
  assert.equal(new Set(exported).size, exported.length, 'MCP tool names are unique');

  const manifest = toolboxManifest().mcp.tools;
  assert.equal(new Set(manifest).size, manifest.length, 'manifest tool names are unique');
  assert.deepEqual(
    [...manifest].sort(),
    [...exported].sort(),
    'manifest tool list must match the MCP script tool-for-tool (update both together)',
  );
});
