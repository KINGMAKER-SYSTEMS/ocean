import path from 'node:path';
import { promises as fs } from 'node:fs';
import { HttpError } from '../adapter.mjs';

function etagFromStat(stat) {
  return `"${stat.size.toString(16)}-${Math.floor(stat.mtimeMs).toString(16)}"`;
}

function kindFromStat(stat) {
  if (stat.isDirectory()) return 'directory';
  if (stat.isFile()) return 'file';
  if (stat.isSymbolicLink()) return 'symlink';
  return 'other';
}

function fileInfo(apiPath, stat) {
  return {
    path: apiPath,
    name: apiPath === '/' ? '/' : path.posix.basename(apiPath),
    type: kindFromStat(stat),
    size: stat.size,
    mtime: stat.mtime.toISOString(),
    ctime: stat.ctime.toISOString(),
    etag: etagFromStat(stat),
  };
}

async function lstatNoSymlink(diskPath) {
  let stat;
  try {
    stat = await fs.lstat(diskPath);
  } catch (error) {
    if (error.code === 'ENOENT') throw new HttpError(404, 'Path not found.');
    throw error;
  }
  if (stat.isSymbolicLink()) throw new HttpError(403, 'Symlinks are not served by Ocean Bedrock.');
  return stat;
}

export class LocalStorageAdapter {
  constructor(mount, absRoot) {
    this.mount = mount;
    this.absRoot = absRoot;
    this.capabilities = mount.capabilities || { read: true, write: true, delete: true, move: true };
    this.diskRoot = absRoot;
  }

  _diskPath(relPath) {
    const resolved = relPath ? path.resolve(this.absRoot, relPath) : this.absRoot;
    if (resolved !== this.absRoot && !resolved.startsWith(`${this.absRoot}${path.sep}`)) {
      throw new HttpError(400, 'Resolved path escapes the mount root.');
    }
    return resolved;
  }

  async stat(relPath, apiPath) {
    const diskPath = this._diskPath(relPath);
    const stat = await lstatNoSymlink(diskPath);
    return fileInfo(apiPath, stat);
  }

  async list(relPath, apiPath, { depth = 1 } = {}) {
    const diskPath = this._diskPath(relPath);
    const stat = await lstatNoSymlink(diskPath);
    if (!stat.isDirectory()) throw new HttpError(400, `${apiPath} is not a directory.`);

    const entries = [];
    async function walk(currentApiPath, currentDiskPath, remainingDepth) {
      const dirents = await fs.readdir(currentDiskPath, { withFileTypes: true });
      for (const dirent of dirents) {
        const childApiPath = currentApiPath === '/' ? `/${dirent.name}` : `${currentApiPath}/${dirent.name}`;
        const childDiskPath = path.join(currentDiskPath, dirent.name);
        const childStat = await fs.lstat(childDiskPath);
        entries.push(fileInfo(childApiPath, childStat));
        if (childStat.isDirectory() && remainingDepth > 1) {
          await walk(childApiPath, childDiskPath, remainingDepth - 1);
        }
      }
    }

    await walk(apiPath, diskPath, depth);
    return { root: fileInfo(apiPath, stat), entries };
  }

  async read(relPath) {
    const diskPath = this._diskPath(relPath);
    return { diskPath };
  }

  async write(relPath, tmpPath) {
    const diskPath = this._diskPath(relPath);
    await fs.mkdir(path.dirname(diskPath), { recursive: true });
    await fs.rename(tmpPath, diskPath);
  }

  async delete(relPath, { recursive = false } = {}) {
    const diskPath = this._diskPath(relPath);
    await fs.rm(diskPath, { recursive, force: false });
  }

  async move(from, to, { overwrite = false } = {}) {
    const fromDisk = this._diskPath(from);
    const toDisk = this._diskPath(to);
    await lstatNoSymlink(fromDisk);
    let toExists = false;
    try { await fs.access(toDisk); toExists = true; } catch {}
    if (toExists && !overwrite) throw new HttpError(409, 'Destination exists. Pass overwrite=true to replace it.');
    await fs.mkdir(path.dirname(toDisk), { recursive: true });
    if (overwrite && toExists) await fs.rm(toDisk, { recursive: true, force: true });
    await fs.rename(fromDisk, toDisk);
  }

  async copy(from, to, { overwrite = false } = {}) {
    const fromDisk = this._diskPath(from);
    const toDisk = this._diskPath(to);
    await lstatNoSymlink(fromDisk);
    let toExists = false;
    try { await fs.access(toDisk); toExists = true; } catch {}
    if (toExists && !overwrite) throw new HttpError(409, 'Destination exists. Pass overwrite=true to replace it.');
    await fs.mkdir(path.dirname(toDisk), { recursive: true });
    await fs.cp(fromDisk, toDisk, { recursive: true, force: Boolean(overwrite), errorOnExist: !overwrite });
  }

  async mkdir(relPath, apiPath) {
    const diskPath = this._diskPath(relPath);
    await fs.mkdir(diskPath, { recursive: true });
    return this.stat(relPath, apiPath);
  }
}
