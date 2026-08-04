#!/usr/bin/env node
import os from 'node:os';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import crypto from 'node:crypto';
import { DEFAULT_IGNORES, classifySensitivePath, classifySkip } from '../src/ingest-filters.mjs';

const DEFAULT_CONFIG_FILE = path.join(os.homedir(), '.config', 'ocean-bedrock', 'bootstrap.json');
const DEFAULT_STATE_FILE = path.join(os.homedir(), '.local', 'state', 'ocean-bedrock', 'ingest-state.json');

function usage() {
  console.log(`Usage:
  npm run ocean:ingest
  npm run ocean:ingest -- --config ~/.config/ocean-bedrock/bootstrap.json --dry-run

Options:
  --config <path>      Bootstrap config path.
  --state <path>       Local ingest state path.
  --source <id>        Only ingest one source id. Can be repeated.
  --dry-run            Scan and report without uploading.
  --all                Override config and allow all extensions, but still exclude secrets.
  --include-secrets    Explicitly include secret-pattern files. Avoid unless supervised.
  --report             Include detailed skippedFiles in output/manifest.
  --max-files <n>      Stop after n changed files.
  --max-file-mb <n>    Override max file size.
`);
}

function parseArgs(argv) {
  const args = { sources: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') args.help = true;
    else if (arg === '--config') args.config = argv[++i];
    else if (arg === '--state') args.state = argv[++i];
    else if (arg === '--source') args.sources.push(argv[++i]);
    else if (arg === '--dry-run') args.dryRun = true;
    else if (arg === '--report') args.report = true;
    else if (arg === '--all') args.all = true;
    else if (arg === '--include-secrets') args.includeSecrets = true;
    else if (arg === '--max-files') args.maxFiles = Number(argv[++i]);
    else if (arg === '--max-file-mb') args.maxFileMb = Number(argv[++i]);
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function expandHome(value) {
  if (!value) return value;
  if (value === '~') return os.homedir();
  if (value.startsWith('~/')) return path.join(os.homedir(), value.slice(2));
  return value;
}

function slugify(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'coworker';
}

function toPosix(value) {
  return value.split(path.sep).join('/');
}

function remoteJoin(prefix, rel) {
  return `${prefix.replace(/\/$/, '')}/${toPosix(rel).replace(/^\/+/, '')}`;
}

function shouldIncludeFile(filePath, config, args) {
  if (!args.includeSecrets && classifySensitivePath(filePath).sensitive) return false;
  if (args.all || config.feedMode === 'all-files' || config.allowedExtensions === null) return true;
  const ext = path.extname(filePath).toLowerCase();
  return (config.allowedExtensions || []).includes(ext);
}

function skipReasonForFile(filePath, config, args, stat = null) {
  if (!args.includeSecrets) {
    const sensitive = classifySensitivePath(filePath);
    if (sensitive.sensitive) return { reason: 'sensitive', matchedPattern: sensitive.matchedPattern };
  }
  if (stat && stat.size > Number(config.maxFileBytes || Infinity)) return { reason: 'size', matchedPattern: null };
  if (!shouldIncludeFile(filePath, config, args)) return { reason: 'extension', matchedPattern: null };
  return null;
}

async function readJson(file, fallback) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return fallback;
    throw error;
  }
}

async function writeJson(file, value) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

async function sha256File(filePath) {
  const hash = crypto.createHash('sha256');
  const handle = await fs.open(filePath, 'r');
  try {
    const stream = handle.createReadStream();
    for await (const chunk of stream) hash.update(chunk);
    return hash.digest('hex');
  } finally {
    await handle.close().catch(() => {});
  }
}

async function* walk(root, ignores, options = {}) {
  const stack = [''];
  while (stack.length) {
    const rel = stack.pop();
    const dir = path.join(root, rel);
    let entries;
    try {
      entries = await fs.readdir(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const childRel = path.join(rel, entry.name);
      const childPath = path.join(root, childRel);
      const skip = classifySkip(toPosix(childRel), entry.name, ignores);
      if (skip.skipped) {
        options.onSkip?.({ rel: childRel, path: childPath, reason: skip.reason, matchedPattern: skip.matchedPattern, isDirectory: entry.isDirectory() });
        continue;
      }
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) stack.push(childRel);
      else if (entry.isFile()) yield { rel: childRel, path: childPath };
    }
  }
}

async function api(config, pathname, options = {}) {
  const response = await fetch(`${config.serverUrl}${pathname}`, {
    ...options,
    headers: {
      Authorization: `Bearer ${config.token}`,
      ...(options.headers || {}),
    },
  });
  const contentType = response.headers.get('content-type') || '';
  const body = contentType.includes('application/json') ? await response.json() : await response.text();
  if (!response.ok) {
    throw new Error(`${options.method || 'GET'} ${pathname} failed: ${response.status} ${JSON.stringify(body)}`);
  }
  return body;
}

function contentTypeFor(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  if (['.md', '.mdx', '.txt', '.rst', '.log'].includes(ext)) return 'text/plain; charset=utf-8';
  if (['.json', '.jsonl'].includes(ext)) return 'application/json; charset=utf-8';
  if (['.yaml', '.yml'].includes(ext)) return 'application/yaml; charset=utf-8';
  if (ext === '.csv') return 'text/csv; charset=utf-8';
  if (ext === '.html' || ext === '.htm') return 'text/html; charset=utf-8';
  if (ext === '.css') return 'text/css; charset=utf-8';
  if (['.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx'].includes(ext)) return 'text/javascript; charset=utf-8';
  if (ext === '.pdf') return 'application/pdf';
  return 'application/octet-stream';
}

async function startSourceRegistryRun(config, args) {
  if (args.dryRun) return { enabled: false, reason: 'dry-run' };

  const ownerSlug = config.ownerSlug || slugify(config.ownerName);
  const deviceSlug = config.deviceSlug || slugify(config.deviceName);
  const correlationId = `cor-ingest-${ownerSlug}-${deviceSlug}-${Date.now()}`;

  try {
    const plan = await api(config, '/api/v1/sync/local-folder/plan', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        ownerName: config.ownerName || ownerSlug,
        ownerSlug,
        deviceName: config.deviceName || deviceSlug,
        deviceSlug,
        remoteRoot: config.remoteRoot || `/coworkers/${ownerSlug}/${deviceSlug}`,
        feedMode: config.feedMode,
        allowedExtensions: config.allowedExtensions,
        defaultIgnores: config.defaultIgnores,
        maxFileBytes: config.maxFileBytes,
        sources: config.sources || [],
        clearance: 'CONFIDENTIAL',
        correlationId,
      }),
    });

    const streams = new Map();
    for (const stream of plan.streams || []) {
      streams.set(stream.stream_key, stream);
    }

    return {
      enabled: true,
      correlationId: plan.correlationId || correlationId,
      instance: plan.sourceInstance,
      streams,
      run: plan.syncRun,
      records: [],
      batches: { sent: 0, records: 0 },
    };
  } catch (error) {
    return { enabled: false, error: error.message };
  }
}

async function flushSourceRecords(config, sourceRegistry) {
  if (!sourceRegistry.enabled || !sourceRegistry.records?.length) return;
  while (sourceRegistry.records.length) {
    const batch = sourceRegistry.records.splice(0, 100);
    const result = await api(config, '/api/v1/sync/local-folder/records:batch', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        source_instance_id: sourceRegistry.instance.id,
        sync_run_id: sourceRegistry.run.id,
        correlationId: sourceRegistry.correlationId,
        records: batch,
      }),
    });
    if (result.ok === false) console.warn(`Source records batch had errors: ${JSON.stringify(result.errors || [])}`);
    sourceRegistry.batches.sent += 1;
    sourceRegistry.batches.records += batch.length;
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    usage();
    process.exit(0);
  }

  const configPath = path.resolve(expandHome(args.config || process.env.OCEAN_BEDROCK_CONFIG || DEFAULT_CONFIG_FILE));
  const statePath = path.resolve(expandHome(args.state || DEFAULT_STATE_FILE));
  const config = await readJson(configPath, null);
  if (!config) throw new Error(`Config not found: ${configPath}. Run npm run ocean:bootstrap first.`);
  if (!config.serverUrl || !config.token) throw new Error('Config is missing serverUrl/token.');

  const state = await readJson(statePath, { version: 1, files: {} });
  const selected = new Set(args.sources);
  const maxFileBytes = Math.max(1, Number(args.maxFileMb || 0)) * 1024 * 1024 || Number(config.maxFileBytes || 10 * 1024 * 1024);
  const maxFiles = args.maxFiles ? Math.max(1, Number(args.maxFiles)) : Infinity;
  const startedAt = new Date().toISOString();
  const sourceRegistry = await startSourceRegistryRun(config, args);
  if (sourceRegistry.error) console.warn(`Source registry skipped: ${sourceRegistry.error}`);

  const detailedSkipped = Boolean(args.dryRun || args.report);
  const manifest = {
    version: 1,
    ownerName: config.ownerName,
    deviceName: config.deviceName,
    serverUrl: config.serverUrl,
    startedAt,
    completedAt: null,
    dryRun: Boolean(args.dryRun),
    includeSecrets: Boolean(args.includeSecrets),
    sources: [],
    totals: { scanned: 0, skipped: 0, skippedByReason: {}, unchanged: 0, changed: 0, uploaded: 0, bytesUploaded: 0, errors: 0 },
  };

  function recordSkip(sourceSummary, rel, reason, matchedPattern = null) {
    manifest.totals.skipped += 1;
    manifest.totals.skippedByReason[reason] = (manifest.totals.skippedByReason[reason] || 0) + 1;
    sourceSummary.skipped += 1;
    sourceSummary.skippedByReason[reason] = (sourceSummary.skippedByReason[reason] || 0) + 1;
    if (detailedSkipped) sourceSummary.skippedFiles.push({ rel: toPosix(rel), reason, ...(matchedPattern ? { matchedPattern } : {}) });
  }

  for (const source of config.sources || []) {
    if (!source.enabled) continue;
    if (selected.size && !selected.has(source.id)) continue;
    const root = path.resolve(expandHome(source.localPath));
    const stream = sourceRegistry.enabled ? sourceRegistry.streams.get(source.id) : null;
    const sourceSummary = {
      id: source.id,
      label: source.label,
      localPath: root,
      remotePrefix: source.remotePrefix,
      sourceInstanceId: sourceRegistry.enabled ? sourceRegistry.instance.id : source.sourceInstanceId || null,
      sourceStreamId: stream?.id || source.sourceStreamId || null,
      files: [],
      skipped: 0,
      skippedByReason: {},
      skippedFiles: detailedSkipped ? [] : undefined,
      unchanged: 0,
      errors: [],
    };
    manifest.sources.push(sourceSummary);

    for await (const item of walk(root, config.defaultIgnores || DEFAULT_IGNORES, {
      onSkip: (skipped) => recordSkip(sourceSummary, skipped.rel, skipped.reason, skipped.matchedPattern),
    })) {
      if (manifest.totals.changed >= maxFiles) break;
      manifest.totals.scanned += 1;

      try {
        const stat = await fs.stat(item.path);
        if (stat.size > maxFileBytes) {
          recordSkip(sourceSummary, item.rel, 'size');
          continue;
        }
        const fileSkip = skipReasonForFile(item.path, { ...config, maxFileBytes }, args, stat);
        if (fileSkip) {
          recordSkip(sourceSummary, item.rel, fileSkip.reason, fileSkip.matchedPattern);
          continue;
        }

        const hash = await sha256File(item.path);
        const stateKey = `${source.id}:${toPosix(item.rel)}`;
        const remotePath = remoteJoin(source.remotePrefix, item.rel);
        const prior = state.files[stateKey];
        if (prior && prior.sha256 === hash && prior.size === stat.size && prior.remotePath === remotePath) {
          manifest.totals.unchanged += 1;
          sourceSummary.unchanged += 1;
          continue;
        }

        manifest.totals.changed += 1;
        const fileRecord = {
          rel: toPosix(item.rel),
          remotePath,
          size: stat.size,
          sha256: hash,
          mtime: stat.mtime.toISOString(),
          uploaded: false,
        };
        sourceSummary.files.push(fileRecord);

        if (!args.dryRun) {
          const bytes = await fs.readFile(item.path);
          await api(config, `/api/v1/file?path=${encodeURIComponent(remotePath)}`, {
            method: 'PUT',
            headers: { 'content-type': contentTypeFor(item.path) },
            body: bytes,
          });
          fileRecord.uploaded = true;
          manifest.totals.uploaded += 1;
          manifest.totals.bytesUploaded += stat.size;

          if (sourceRegistry.enabled) {
            const sourceRecordId = `${source.id}:${toPosix(item.rel)}`;
            sourceRegistry.records.push({
              stream_id: stream?.id || null,
              source_record_id: sourceRecordId,
              virtual_path: remotePath,
              source_updated_at: stat.mtime.toISOString(),
              content_sha256: hash,
              metadata: {
                source_id: source.id,
                label: source.label,
                rel: toPosix(item.rel),
                local_path_label: path.basename(root),
                size: stat.size,
                content_type: contentTypeFor(item.path),
              },
            });
            fileRecord.sourceRecordId = sourceRecordId;
            if (sourceRegistry.records.length >= 100) {
              try {
                await flushSourceRecords(config, sourceRegistry);
              } catch (registryError) {
                fileRecord.sourceRegistryError = registryError.message;
                console.warn(`Source records batch skipped: ${registryError.message}`);
              }
            }
          }

          state.files[stateKey] = {
            sha256: hash,
            size: stat.size,
            mtime: stat.mtime.toISOString(),
            remotePath,
            uploadedAt: new Date().toISOString(),
          };
        }
      } catch (error) {
        manifest.totals.errors += 1;
        sourceSummary.errors.push({ rel: toPosix(item.rel), error: error.message });
      }
    }
  }

  manifest.completedAt = new Date().toISOString();
  const manifestPath = `/context/ocean-bedrock/manifests/${config.ownerSlug || 'coworker'}-${config.deviceSlug || 'device'}-${Date.now()}.json`;

  if (!args.dryRun) {
    await api(config, '/api/v1/mkdir', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path: '/context/ocean-bedrock/manifests' }),
    });
    await api(config, `/api/v1/file?path=${encodeURIComponent(manifestPath)}`, {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: `${JSON.stringify(manifest, null, 2)}\n`,
    });
    try {
      await api(config, '/api/v1/ledger/events', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          event_type: 'ingest.local_folder_synced',
          correlation_id: `cor-ingest-${config.ownerSlug || 'coworker'}-${config.deviceSlug || 'device'}`,
          lab: 'ocean-context',
          virtual_path: manifestPath,
          payload: {
            ownerName: config.ownerName,
            deviceName: config.deviceName,
            manifestPath,
            totals: manifest.totals,
            sources: manifest.sources.map((source) => ({ id: source.id, label: source.label, remotePrefix: source.remotePrefix })),
          },
          clearance: 'CONFIDENTIAL',
          tags: ['ocean-bedrock', 'ingest', 'local-folder-feed'],
        }),
      });
    } catch (error) {
      console.warn(`Ledger event skipped: ${error.message}`);
    }
    if (sourceRegistry.enabled) {
      try {
        await flushSourceRecords(config, sourceRegistry);
        await api(config, '/api/v1/sync/local-folder/commit', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            sync_run_id: sourceRegistry.run.id,
            stats: {
              scanned_count: manifest.totals.scanned,
              changed_count: manifest.totals.changed,
              uploaded_count: manifest.totals.uploaded,
              skipped_count: manifest.totals.skipped,
              error_count: manifest.totals.errors,
              manifest_path: manifestPath,
              metadata: {
                unchanged_count: manifest.totals.unchanged,
                owner_name: config.ownerName,
                device_name: config.deviceName,
                record_batches: sourceRegistry.batches,
              },
            },
            correlationId: sourceRegistry.correlationId,
          }),
        });
      } catch (registryError) {
        console.warn(`Source sync run completion skipped: ${registryError.message}`);
      }
    }

    state.lastRunAt = manifest.completedAt;
    state.lastManifestPath = manifestPath;
    await writeJson(statePath, state);
  }

  console.log(JSON.stringify({
    ok: true,
    dryRun: Boolean(args.dryRun),
    configPath,
    statePath,
    manifestPath: args.dryRun ? null : manifestPath,
    sourceRegistry: sourceRegistry.enabled
      ? { enabled: true, sourceInstanceId: sourceRegistry.instance.id, syncRunId: sourceRegistry.run.id, batches: sourceRegistry.batches }
      : { enabled: false, reason: sourceRegistry.reason || sourceRegistry.error || 'unavailable' },
    totals: manifest.totals,
    sources: manifest.sources,
  }, null, 2));
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
