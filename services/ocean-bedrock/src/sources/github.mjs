import path from 'node:path';

import { SourceRunner, joinApiPath } from './runner.mjs';
import { resolveSecret } from './secrets.mjs';

const DEFAULT_MAX_FILES = Number(process.env.OCEAN_GITHUB_SYNC_MAX_FILES || 200);
const API_BASE = 'https://api.github.com';

function contentTypeForPath(filePath) {
  const ext = path.posix.extname(filePath).toLowerCase();
  if (['.md', '.mdx'].includes(ext)) return 'text/markdown; charset=utf-8';
  if (['.txt', '.log', '.rst'].includes(ext)) return 'text/plain; charset=utf-8';
  if (['.json', '.jsonl'].includes(ext)) return 'application/json; charset=utf-8';
  if (['.yaml', '.yml'].includes(ext)) return 'application/yaml; charset=utf-8';
  if (ext === '.csv') return 'text/csv; charset=utf-8';
  if (['.js', '.mjs', '.cjs'].includes(ext)) return 'text/javascript; charset=utf-8';
  if (['.ts', '.tsx', '.jsx'].includes(ext)) return 'text/typescript; charset=utf-8';
  if (['.html', '.htm'].includes(ext)) return 'text/html; charset=utf-8';
  if (ext === '.css') return 'text/css; charset=utf-8';
  if (ext === '.py') return 'text/x-python; charset=utf-8';
  if (ext === '.rs') return 'text/rust; charset=utf-8';
  if (ext === '.go') return 'text/go; charset=utf-8';
  if (ext === '.sql') return 'application/sql; charset=utf-8';
  return 'application/octet-stream';
}

function cleanRepoPath(input) {
  const value = String(input || '').trim().replace(/^\/+/, '').replace(/\/+$/, '');
  if (!value) return '';
  const parts = value.split('/').filter(Boolean);
  if (parts.some((part) => part === '.' || part === '..' || part.includes('\0') || part.includes('\\'))) {
    throw new Error(`Invalid GitHub path filter: ${input}`);
  }
  return parts.join('/');
}

function configuredPaths(instance, stream) {
  const config = instance.config || {};
  const selection = stream?.selection || {};
  const raw = [
    ...(Array.isArray(config.paths) ? config.paths : []),
    ...(Array.isArray(selection.paths) ? selection.paths : []),
    config.path,
    selection.path,
  ].filter((value) => value !== undefined && value !== null && value !== '');
  const paths = raw.map(cleanRepoPath);
  return paths.length ? [...new Set(paths)] : [''];
}

function matchesPaths(filePath, pathFilters) {
  return pathFilters.some((prefix) => !prefix || filePath === prefix || filePath.startsWith(`${prefix}/`));
}

function repoConfig(instance) {
  const config = instance.config || {};
  const owner = String(config.owner || '').trim();
  const repo = String(config.repo || '').trim();
  if (!owner || !repo) throw new Error('GitHub source config requires owner and repo.');
  return {
    owner,
    repo,
    branch: String(config.branch || config.ref || 'main').trim() || 'main',
  };
}

function externalUrl(owner, repo, branchOrSha, filePath) {
  return `https://github.com/${owner}/${repo}/blob/${encodeURIComponent(branchOrSha)}/${filePath.split('/').map(encodeURIComponent).join('/')}`;
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export class GitHubSourceRunner extends SourceRunner {
  constructor() {
    super({ adapterId: 'github', name: 'github' });
    this.rateLimit = null;
  }

  async githubFetch(pathname, token) {
    const response = await fetch(`${API_BASE}${pathname}`, {
      headers: {
        accept: 'application/vnd.github+json',
        'user-agent': 'ocean-bedrock-source-runner',
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
    });

    const remaining = response.headers.get('x-ratelimit-remaining');
    const reset = response.headers.get('x-ratelimit-reset');
    this.rateLimit = {
      remaining: remaining === null ? null : Number(remaining),
      reset: reset === null ? null : Number(reset),
    };

    if (response.status === 403 && this.rateLimit.remaining === 0) {
      throw new Error(`GitHub API rate limit exhausted; reset=${this.rateLimit.reset || 'unknown'}.`);
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`GitHub API ${pathname} failed: ${response.status} ${text.slice(0, 500)}`);
    }

    if (this.rateLimit.remaining !== null && this.rateLimit.remaining <= 1 && this.rateLimit.reset) {
      const waitMs = Math.max(0, (this.rateLimit.reset * 1000) - Date.now()) + 1000;
      if (waitMs > 0 && waitMs < 60_000) await sleep(waitMs);
    }

    return response.json();
  }

  async plan(ctx) {
    const token = ctx.instance.secret_ref ? resolveSecret(ctx.instance.secret_ref) : null;
    const { owner, repo, branch } = repoConfig(ctx.instance);
    const commit = await this.githubFetch(`/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/commits/${encodeURIComponent(branch)}`, token);
    ctx.github = {
      token,
      owner,
      repo,
      branch,
      headSha: commit.sha,
      headDate: commit.commit?.committer?.date || commit.commit?.author?.date || new Date().toISOString(),
      pathFilters: configuredPaths(ctx.instance, ctx.stream),
      maxFiles: Number(ctx.maxRecords || ctx.instance.config?.max_files || DEFAULT_MAX_FILES),
    };
    ctx.setNextCursor(commit.sha, { last_event_timestamp: ctx.github.headDate });
    return { cursor_value: commit.sha };
  }

  async *iterate(ctx) {
    const gh = ctx.github;
    if (!gh) throw new Error('GitHub runner plan() was not called.');
    const basePrefix = ctx.stream?.remote_prefix || ctx.instance.remote_prefix || `/context/github/${gh.owner}/${gh.repo}`;
    const streamType = ctx.stream?.stream_type || ctx.stream?.selection?.resource_type || 'contents';
    if (!['contents', 'repo_contents', 'folder'].includes(streamType)) {
      throw new Error(`GitHub runner supports stream_type=contents only, not ${streamType}.`);
    }

    const files = [];
    if (ctx.mode !== 'snapshot' && ctx.cursorValue) {
      const compare = await this.githubFetch(
        `/repos/${encodeURIComponent(gh.owner)}/${encodeURIComponent(gh.repo)}/compare/${encodeURIComponent(ctx.cursorValue)}...${encodeURIComponent(gh.headSha)}`,
        gh.token,
      );
      for (const file of compare.files || []) {
        const filename = cleanRepoPath(file.filename);
        if (!matchesPaths(filename, gh.pathFilters)) continue;
        if (file.status === 'removed') {
          files.push({ path: filename, sha: file.sha || null, status: 'removed', deleted: true });
        } else if (file.status === 'renamed' && file.previous_filename) {
          const previous = cleanRepoPath(file.previous_filename);
          if (matchesPaths(previous, gh.pathFilters)) files.push({ path: previous, status: 'removed', deleted: true });
          files.push({ path: filename, sha: file.sha, status: file.status });
        } else {
          files.push({ path: filename, sha: file.sha, status: file.status || 'changed' });
        }
      }
    } else {
      const tree = await this.githubFetch(
        `/repos/${encodeURIComponent(gh.owner)}/${encodeURIComponent(gh.repo)}/git/trees/${encodeURIComponent(gh.headSha)}?recursive=1`,
        gh.token,
      );
      for (const item of tree.tree || []) {
        if (item.type !== 'blob') continue;
        const filename = cleanRepoPath(item.path);
        if (!matchesPaths(filename, gh.pathFilters)) continue;
        files.push({ path: filename, sha: item.sha, status: 'snapshot' });
      }
    }

    let yielded = 0;
    for (const file of files) {
      if (yielded >= gh.maxFiles) break;
      yielded += 1;
      const virtualPath = joinApiPath(basePrefix, file.path);
      const sourceRecordId = `contents:${file.path}`;

      if (file.deleted) {
        yield {
          source_record_id: sourceRecordId,
          virtual_path: virtualPath,
          deleted: true,
          source_updated_at: gh.headDate,
          cursor_value: gh.headSha,
          metadata: {
            adapter_id: 'github', owner: gh.owner, repo: gh.repo, branch: gh.branch,
            commit_sha: gh.headSha, path: file.path, status: 'removed',
            external_url: externalUrl(gh.owner, gh.repo, gh.branch, file.path),
          },
        };
        continue;
      }

      if (!file.sha) continue;
      const blob = await this.githubFetch(
        `/repos/${encodeURIComponent(gh.owner)}/${encodeURIComponent(gh.repo)}/git/blobs/${encodeURIComponent(file.sha)}`,
        gh.token,
      );
      if (blob.encoding !== 'base64') throw new Error(`Unsupported GitHub blob encoding for ${file.path}: ${blob.encoding}`);
      const bytes = Buffer.from(String(blob.content || '').replace(/\s+/g, ''), 'base64');

      yield {
        source_record_id: sourceRecordId,
        virtual_path: virtualPath,
        bytes,
        content_type: contentTypeForPath(file.path),
        source_updated_at: gh.headDate,
        cursor_value: gh.headSha,
        metadata: {
          adapter_id: 'github', owner: gh.owner, repo: gh.repo, branch: gh.branch,
          commit_sha: gh.headSha, blob_sha: file.sha, path: file.path, status: file.status,
          external_url: externalUrl(gh.owner, gh.repo, gh.branch, file.path),
        },
      };
    }
  }

  async commit() {
    return { metadata: { rate_limit: this.rateLimit } };
  }
}

export function createGitHubSourceRunner() {
  return new GitHubSourceRunner();
}
