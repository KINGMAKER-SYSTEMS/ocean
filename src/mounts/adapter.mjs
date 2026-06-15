export class HttpError extends Error {
  constructor(status, message, details = undefined) {
    super(message);
    this.status = status;
    this.details = details;
  }
}

export function createUnimplementedAdapter(mount) {
  const stub = (name) => () => {
    throw new HttpError(501, `Mount type "${mount.type}" is not yet implemented (${name}).`);
  };
  return {
    capabilities: mount.capabilities || {},
    diskRoot: null,
    stat: stub('stat'),
    list: stub('list'),
    read: stub('read'),
    write: stub('write'),
    delete: stub('delete'),
    move: stub('move'),
    copy: stub('copy'),
    mkdir: stub('mkdir'),
  };
}
