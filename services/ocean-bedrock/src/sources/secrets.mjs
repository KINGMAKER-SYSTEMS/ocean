import crypto from 'node:crypto';

const SECRET_KEY_PATTERN = /(authorization|bearer|token|secret|password|credential|api[_-]?key|access[_-]?key|private[_-]?key|webhook[_-]?secret)/i;
const ENV_SECRET_REF_PATTERN = /^env:[A-Za-z_][A-Za-z0-9_]*$/;
const KV_SECRET_REF_PATTERN = /^kv:[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/;

export function looksLikeRawSecret(value) {
  if (value === undefined || value === null) return false;
  const text = String(value).trim();
  if (!text) return false;
  if (/^Bearer\s+\S+/i.test(text)) return true;
  if (/^(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{20,}$/.test(text)) return true;
  if (/^github_pat_[A-Za-z0-9_]{40,}$/.test(text)) return true;
  if (/^xox[abprs]-[A-Za-z0-9-]{20,}$/.test(text)) return true;
  if (/^\d{6,}:[A-Za-z0-9_-]{20,}$/.test(text)) return true; // Telegram bot token shape.
  if (text.length >= 40 && /^[A-Za-z0-9._~+/=-]+$/.test(text) && /[A-Za-z]/.test(text) && /\d/.test(text)) return true;
  return false;
}

export function normalizeSecretRef(secretRef, { allowNull = true } = {}) {
  if (secretRef === undefined || secretRef === null || secretRef === '') {
    if (allowNull) return null;
    throw new Error('secret_ref is required.');
  }

  const text = String(secretRef).trim();
  if (!text) {
    if (allowNull) return null;
    throw new Error('secret_ref is required.');
  }

  if (looksLikeRawSecret(text)) {
    throw new Error('secret_ref looks like a raw secret. Store a pointer such as env:GITHUB_TOKEN instead.');
  }

  if (ENV_SECRET_REF_PATTERN.test(text) || KV_SECRET_REF_PATTERN.test(text)) return text;

  throw new Error('secret_ref must be a pointer in the form env:VAR_NAME (or future kv:key), not a raw token.');
}

export function validateSecretRef(secretRef, options = {}) {
  return normalizeSecretRef(secretRef, options);
}

export function secretFingerprint(secret) {
  if (!secret) return null;
  return crypto.createHash('sha256').update(String(secret)).digest('hex').slice(0, 16);
}

export function resolveSecret(secretRef, { required = false } = {}) {
  const normalized = normalizeSecretRef(secretRef, { allowNull: !required });
  if (!normalized) return null;

  if (normalized.startsWith('env:')) {
    const name = normalized.slice(4);
    const value = process.env[name];
    if (!value) throw new Error(`Secret environment variable ${name} is not set.`);
    return value;
  }

  if (normalized.startsWith('kv:')) {
    throw new Error('kv: secret references are reserved for a future secret manager and are not implemented yet.');
  }

  throw new Error('Unsupported secret_ref scheme.');
}

export function redactSecrets(value) {
  if (Array.isArray(value)) return value.map((item) => redactSecrets(item));
  if (!value || typeof value !== 'object') return looksLikeRawSecret(value) ? '[redacted]' : value;

  const out = {};
  for (const [key, child] of Object.entries(value)) {
    if (SECRET_KEY_PATTERN.test(key)) {
      out[key] = child ? '[redacted]' : child;
    } else {
      out[key] = redactSecrets(child);
    }
  }
  return out;
}
