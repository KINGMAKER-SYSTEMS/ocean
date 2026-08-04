import { loadMounts } from './registry.mjs';
import { HttpError } from './adapter.mjs';

export async function resolveMount(state, apiPath) {
  const mounts = await loadMounts(state);

  const entry = mounts.find(({ mount }) => {
    const p = mount.prefix;
    return p === '/' || apiPath === p || apiPath.startsWith(`${p}/`);
  });

  if (!entry) throw new HttpError(500, `No mount found for path: ${apiPath}`);

  const { mount, adapter } = entry;
  const relPath = mount.prefix === '/'
    ? apiPath.slice(1)
    : (apiPath === mount.prefix ? '' : apiPath.slice(mount.prefix.length + 1));

  return { mount, adapter, relPath };
}
