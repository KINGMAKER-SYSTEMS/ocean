import path from 'node:path';
import { promises as fs } from 'node:fs';
import { createReadStream } from 'node:fs';
import { Readable } from 'node:stream';
import { HttpError } from '../adapter.mjs';

const CF_API = 'https://api.cloudflare.com/client/v4';

export class R2StorageAdapter {
  constructor(mount) {
    this.mount = mount;
    this.id = mount.id;
    this.type = mount.type;
    this.bucket = mount.config?.bucket;
    if (!this.bucket) throw new Error(`R2 mount "${mount.id}" missing config.bucket`);

    this._accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
    this._token = process.env.CLOUDFLARE_API_TOKEN;
    if (!this._accountId || !this._token) {
      throw new Error(`R2 mount "${mount.id}" requires CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN`);
    }

    this.capabilities = mount.capabilities || { read: true, write: true, delete: true, move: true };
    this.diskRoot = null;
  }

  _baseUrl() {
    return `${CF_API}/accounts/${this._accountId}/r2/buckets/${this.bucket}/objects`;
  }

  _headers() {
    return { Authorization: `Bearer ${this._token}` };
  }

  _objectUrl(relPath) {
    const encoded = relPath.split('/').map(encodeURIComponent).join('/');
    return `${this._baseUrl()}/${encoded}`;
  }

  async stat(relPath, apiPath) {
    if (!relPath) {
      return { path: apiPath, name: apiPath === '/' ? '/' : path.posix.basename(apiPath), type: 'directory', size: 0, mtime: null, etag: null };
    }

    // CF REST API does not support HEAD — use list with prefix for metadata
    const params = new URLSearchParams({ limit: '2', prefix: relPath });
    const res = await fetch(`${this._baseUrl()}?${params}`, { headers: this._headers() });
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new HttpError(res.status, body?.errors?.[0]?.message || `R2 stat error ${res.status}`);
    }
    const data = await res.json();
    const items = Array.isArray(data.result) ? data.result : [];

    const obj = items.find((i) => i.key === relPath);
    if (obj) {
      return {
        path: apiPath,
        name: path.posix.basename(apiPath),
        type: 'file',
        size: obj.size ?? 0,
        mtime: obj.last_modified || null,
        etag: (obj.etag || '').replace(/"/g, '') || null,
        contentType: obj.http_metadata?.contentType || 'application/octet-stream',
      };
    }

    if (items.some((i) => i.key.startsWith(`${relPath}/`))) {
      return { path: apiPath, name: path.posix.basename(apiPath), type: 'directory', size: 0, mtime: null, etag: null };
    }

    // Check with explicit dir prefix in case the 2-item window above missed it
    const dirParams = new URLSearchParams({ limit: '1', prefix: `${relPath}/` });
    const dirRes = await fetch(`${this._baseUrl()}?${dirParams}`, { headers: this._headers() });
    if (dirRes.ok) {
      const dirData = await dirRes.json();
      if ((Array.isArray(dirData.result) ? dirData.result : []).length > 0) {
        return { path: apiPath, name: path.posix.basename(apiPath), type: 'directory', size: 0, mtime: null, etag: null };
      }
    }

    throw new HttpError(404, `Not found: ${apiPath}`);
  }

  async list(relPath, apiPath, { depth = 1 } = {}) {
    const prefix = relPath ? `${relPath}/` : '';
    const useDelimiter = depth === 1;
    const entries = [];
    let cursor = null;
    let pageCount = 0;

    do {
      const params = new URLSearchParams({ limit: '1000' });
      if (prefix) params.set('prefix', prefix);
      if (useDelimiter) params.set('delimiter', '/');
      if (cursor) params.set('cursor', cursor);

      const res = await fetch(`${this._baseUrl()}?${params}`, { headers: this._headers() });
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new HttpError(res.status, body?.errors?.[0]?.message || `R2 list error ${res.status}`);
      }

      const data = await res.json();
      const items = Array.isArray(data.result) ? data.result : [];
      const delimited = Array.isArray(data.result_info?.delimited) ? data.result_info.delimited : [];

      for (const dp of delimited) {
        const rel = dp.slice(prefix.length).replace(/\/$/, '');
        if (!rel) continue;
        const entryApiPath = apiPath === '/' ? `/${rel}` : `${apiPath}/${rel}`;
        entries.push({ path: entryApiPath, name: rel, type: 'directory', size: 0, mtime: null, etag: null });
      }

      for (const obj of items) {
        if (obj.key === prefix.replace(/\/$/, '')) continue;
        const rel = obj.key.slice(prefix.length);
        if (!rel) continue;
        if (useDelimiter && rel.includes('/')) continue;
        const entryApiPath = apiPath === '/' ? `/${obj.key}` : (prefix ? `${apiPath}/${rel}` : `${apiPath}/${obj.key}`);
        entries.push({
          path: entryApiPath,
          name: path.posix.basename(obj.key),
          type: 'file',
          size: obj.size ?? 0,
          mtime: obj.last_modified || null,
          etag: (obj.etag || '').replace(/"/g, '') || null,
          contentType: obj.http_metadata?.contentType || 'application/octet-stream',
        });
      }

      cursor = data.result_info?.is_truncated ? (data.result_info.cursor || null) : null;
      pageCount++;
      if (pageCount > 20) { console.warn(`[ocean-bedrock] R2 list truncated after 20 pages for prefix "${prefix}"`); break; }
    } while (cursor);

    const root = await this.stat(relPath, apiPath).catch(() => ({
      path: apiPath, name: path.posix.basename(apiPath) || '/', type: 'directory', size: 0, mtime: null, etag: null,
    }));
    return { root, entries };
  }

  async read(relPath) {
    const res = await fetch(this._objectUrl(relPath), { method: 'GET', headers: this._headers() });
    if (res.status === 404) throw new HttpError(404, 'Not found.');
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new HttpError(res.status, body?.errors?.[0]?.message || `R2 read error ${res.status}`);
    }
    return { stream: res.body };
  }

  async write(relPath, tmpPath) {
    const stat = await fs.stat(tmpPath);
    const webStream = Readable.toWeb(createReadStream(tmpPath));
    const res = await fetch(this._objectUrl(relPath), {
      method: 'PUT',
      body: webStream,
      headers: { ...this._headers(), 'content-length': String(stat.size) },
      duplex: 'half',
    });
    try { await fs.unlink(tmpPath); } catch {}
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new HttpError(res.status, body?.errors?.[0]?.message || `R2 write error ${res.status}`);
    }
  }

  async delete(relPath, { recursive = false } = {}) {
    if (recursive) {
      const prefix = `${relPath}/`;
      const params = new URLSearchParams({ limit: '1000', prefix });
      let cursor = null;
      do {
        if (cursor) params.set('cursor', cursor);
        const listRes = await fetch(`${this._baseUrl()}?${params}`, { headers: this._headers() });
        if (!listRes.ok) break;
        const data = await listRes.json();
        const items = Array.isArray(data.result) ? data.result : [];
        await Promise.all(items.map((obj) => fetch(this._objectUrl(obj.key), { method: 'DELETE', headers: this._headers() })));
        cursor = data.result_info?.is_truncated ? data.result_info.cursor : null;
      } while (cursor);
      return;
    }

    const res = await fetch(this._objectUrl(relPath), { method: 'DELETE', headers: this._headers() });
    if (!res.ok && res.status !== 404) {
      const body = await res.json().catch(() => ({}));
      throw new HttpError(res.status, body?.errors?.[0]?.message || `R2 delete error ${res.status}`);
    }
  }

  async move(from, to, { overwrite = false } = {}) {
    await this.copy(from, to, { overwrite });
    await this.delete(from);
  }

  async copy(from, to, { overwrite = false } = {}) {
    if (!overwrite) {
      try { await this.stat(to, to); throw new HttpError(409, 'Destination exists. Pass overwrite=true to replace it.'); } catch (e) { if (e instanceof HttpError && e.status !== 404) throw e; }
    }
    const srcRes = await fetch(this._objectUrl(from), { method: 'GET', headers: this._headers() });
    if (!srcRes.ok) throw new HttpError(srcRes.status, 'R2 copy source not found.');
    const contentType = srcRes.headers.get('content-type') || 'application/octet-stream';
    const destRes = await fetch(this._objectUrl(to), {
      method: 'PUT',
      body: srcRes.body,
      headers: { ...this._headers(), 'content-type': contentType },
      duplex: 'half',
    });
    if (!destRes.ok) {
      const body = await destRes.json().catch(() => ({}));
      throw new HttpError(destRes.status, body?.errors?.[0]?.message || `R2 copy error ${destRes.status}`);
    }
  }

  async mkdir(relPath, apiPath) {
    return { path: apiPath, name: path.posix.basename(apiPath), type: 'directory', size: 0, mtime: null, etag: null };
  }
}
