import crypto from 'node:crypto';
import path from 'node:path';
import { promises as fs } from 'node:fs';

export const AUTH_VERSION = 1;
export const VALID_ROLES = new Set(['readonly', 'contributor', 'readwrite', 'agent', 'admin']);

export function nowIso() {
  return new Date().toISOString();
}

export function generateToken(bytes = 32) {
  return crypto.randomBytes(bytes).toString('base64url');
}

export function hashToken(token) {
  return `sha256:${crypto.createHash('sha256').update(String(token), 'utf8').digest('hex')}`;
}

function timingSafeEqualString(a, b) {
  const left = Buffer.from(String(a));
  const right = Buffer.from(String(b));
  if (left.length !== right.length) return false;
  return crypto.timingSafeEqual(left, right);
}

export function normalizeScopes(scopes = ['/']) {
  const input = Array.isArray(scopes) && scopes.length > 0 ? scopes : ['/'];
  return [...new Set(input.map((scope) => normalizeScope(scope)))];
}

function normalizeScope(scope) {
  if (typeof scope !== 'string') return '/';
  let s = scope.trim();
  if (!s) return '/';
  if (!s.startsWith('/')) s = `/${s}`;
  s = s.replace(/\/+/g, '/');
  const parts = s.split('/').filter((part) => part && part !== '.');
  if (parts.includes('..')) {
    throw new Error(`Invalid token scope: ${scope}`);
  }
  return parts.length ? `/${parts.join('/')}` : '/';
}

export async function ensureAuthFile(authFile) {
  await fs.mkdir(path.dirname(authFile), { recursive: true });
  try {
    await fs.access(authFile);
  } catch {
    await saveAuthFile(authFile, { version: AUTH_VERSION, tokens: [] });
  }
}

export async function loadAuthFile(authFile) {
  await ensureAuthFile(authFile);
  const raw = await fs.readFile(authFile, 'utf8');
  const parsed = raw.trim() ? JSON.parse(raw) : { version: AUTH_VERSION, tokens: [] };
  if (!Array.isArray(parsed.tokens)) parsed.tokens = [];
  if (!Array.isArray(parsed.invites)) parsed.invites = [];
  parsed.version = parsed.version || AUTH_VERSION;
  return parsed;
}

export async function saveAuthFile(authFile, auth) {
  await fs.mkdir(path.dirname(authFile), { recursive: true });
  const tmp = `${authFile}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(tmp, `${JSON.stringify(auth, null, 2)}\n`, { mode: 0o600 });
  await fs.rename(tmp, authFile);
}

export function publicTokenRecord(record) {
  const { hash, ...safe } = record;
  return safe;
}

export function isTokenExpired(record, at = Date.now()) {
  return Boolean(record.expiresAt && Date.parse(record.expiresAt) <= at);
}

export function findToken(auth, token) {
  const wanted = hashToken(token);
  return auth.tokens.find((record) => {
    if (!record || !record.hash || record.revokedAt || isTokenExpired(record)) return false;
    return timingSafeEqualString(record.hash, wanted);
  });
}

export async function addToken(authFile, options = {}) {
  const auth = await loadAuthFile(authFile);
  const token = options.token || generateToken();
  const hash = hashToken(token);

  if (auth.tokens.some((record) => record.hash && timingSafeEqualString(record.hash, hash))) {
    throw new Error('A token with this secret already exists.');
  }

  const role = options.role || 'contributor';
  if (!VALID_ROLES.has(role)) {
    throw new Error(`Invalid role "${role}". Use one of: ${[...VALID_ROLES].join(', ')}`);
  }

  const id = `tok_${crypto.createHash('sha256').update(`${hash}:${Date.now()}:${Math.random()}`).digest('hex').slice(0, 12)}`;
  const record = {
    id,
    name: options.name || id,
    hash,
    role,
    scopes: normalizeScopes(options.scopes || ['/']),
    createdAt: nowIso(),
    createdBy: options.createdBy || null,
    expiresAt: options.expiresAt || null,
    revokedAt: null,
  };

  auth.tokens.push(record);
  await saveAuthFile(authFile, auth);
  return { token, record: publicTokenRecord(record) };
}

export async function revokeToken(authFile, tokenId, revokedBy = null) {
  const auth = await loadAuthFile(authFile);
  const record = auth.tokens.find((candidate) => candidate.id === tokenId);
  if (!record) return null;
  record.revokedAt = nowIso();
  record.revokedBy = revokedBy;
  await saveAuthFile(authFile, auth);
  return publicTokenRecord(record);
}

export function publicInviteRecord(record) {
  const { hash, ...safe } = record;
  return safe;
}

export async function createInvite(authFile, options = {}) {
  const auth = await loadAuthFile(authFile);
  const code = options.code || generateToken(24);
  const hash = hashToken(code);
  if (auth.invites.some((record) => record.hash && timingSafeEqualString(record.hash, hash))) {
    throw new Error('An invite with this code already exists.');
  }

  const role = options.role || 'contributor';
  if (!VALID_ROLES.has(role)) throw new Error(`Invalid role "${role}". Use one of: ${[...VALID_ROLES].join(', ')}`);
  if (role === 'admin' && !options.allowAdminInvite) throw new Error('Admin invites are not allowed by default.');

  const ttlMinutes = Math.max(1, Number(options.ttlMinutes || 60));
  const id = `inv_${crypto.createHash('sha256').update(`${hash}:${Date.now()}:${Math.random()}`).digest('hex').slice(0, 12)}`;
  const record = {
    id,
    name: options.name || id,
    hash,
    role,
    scopes: normalizeScopes(options.scopes || ['/']),
    createdAt: nowIso(),
    createdBy: options.createdBy || null,
    expiresAt: options.expiresAt || new Date(Date.now() + ttlMinutes * 60 * 1000).toISOString(),
    consumedAt: null,
    consumedByTokenId: null,
  };
  auth.invites.push(record);
  await saveAuthFile(authFile, auth);
  return { code, invite: publicInviteRecord(record) };
}

export async function redeemInvite(authFile, code, options = {}) {
  const auth = await loadAuthFile(authFile);
  const wanted = hashToken(code || '');
  const invite = auth.invites.find((record) => record.hash && timingSafeEqualString(record.hash, wanted));
  if (!invite || invite.consumedAt || isTokenExpired(invite)) return null;

  const token = options.token || generateToken();
  const tokenHash = hashToken(token);
  if (auth.tokens.some((record) => record.hash && timingSafeEqualString(record.hash, tokenHash))) {
    throw new Error('A token with this secret already exists.');
  }
  const tokenRecord = {
    id: `tok_${crypto.createHash('sha256').update(`${tokenHash}:${Date.now()}:${Math.random()}`).digest('hex').slice(0, 12)}`,
    name: invite.name,
    hash: tokenHash,
    role: invite.role,
    scopes: normalizeScopes(invite.scopes || ['/']),
    createdAt: nowIso(),
    createdBy: invite.id,
    expiresAt: options.expiresAt || null,
    revokedAt: null,
  };
  auth.tokens.push(tokenRecord);
  invite.consumedAt = nowIso();
  invite.consumedByTokenId = tokenRecord.id;
  await saveAuthFile(authFile, auth);
  return { token, invite: publicInviteRecord(invite), record: publicTokenRecord(tokenRecord) };
}
