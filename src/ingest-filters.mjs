export const DEFAULT_NOISE_IGNORES = [
  '.git', '.hg', '.svn', 'node_modules', '.venv', 'venv', 'target', 'dist', 'build',
  '.next', '.turbo', '.cache', '__pycache__', '.DS_Store', 'Thumbs.db',
];

export const DEFAULT_SECRET_PATTERNS = [
  { id: 'env-file', match: (rel, name) => name === '.env' || name.startsWith('.env.') },
  { id: 'pem-key', match: (_rel, name) => /\.(pem|key|p12|pfx|keystore)$/i.test(name) },
  { id: 'ssh-private-key', match: (_rel, name) => /^(id_rsa|id_dsa|id_ecdsa|id_ed25519)$/i.test(name) },
  { id: 'secret-name', match: (_rel, name) => /(secret|credential|credentials)/i.test(name) },
  { id: 'service-account-json', match: (_rel, name) => /^service-account.*\.json$/i.test(name) },
  { id: 'terraform-state', match: (_rel, name) => /\.tfstate(\.backup)?$/i.test(name) },
  { id: 'tool-credential-file', match: (_rel, name) => ['.npmrc', '.pypirc', '.netrc', '.htpasswd'].includes(name) },
  { id: 'secret-directory', match: (rel, name) => ['.aws', '.ssh', '.gnupg'].includes(name) || /(^|\/)(\.aws|\.ssh|\.gnupg)(\/|$)/.test(rel) },
  { id: 'keepass-vault', match: (_rel, name) => /\.kdbx$/i.test(name) },
];

export const DEFAULT_IGNORES = [...DEFAULT_NOISE_IGNORES];

export function toPosixPath(value) {
  return String(value || '').replace(/\\/g, '/');
}

export function classifySensitivePath(relPath) {
  const rel = toPosixPath(relPath).replace(/^\/+/, '');
  const name = rel.split('/').filter(Boolean).at(-1) || rel;
  const matched = DEFAULT_SECRET_PATTERNS.find((pattern) => pattern.match(rel, name));
  return matched ? { sensitive: true, matchedPattern: matched.id } : { sensitive: false, matchedPattern: null };
}

export function shouldIgnoreName(name, ignores = DEFAULT_IGNORES) {
  return ignores.includes(name);
}

export function classifySkip(relPath, name, ignores = DEFAULT_IGNORES) {
  const sensitive = classifySensitivePath(relPath);
  if (sensitive.sensitive) return { skipped: true, reason: 'sensitive', matchedPattern: sensitive.matchedPattern };
  if (shouldIgnoreName(name, ignores)) return { skipped: true, reason: 'ignore', matchedPattern: null };
  return { skipped: false, reason: null, matchedPattern: null };
}
