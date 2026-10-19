// Private host.rs adapter for Tauri 2's event plugin; no global Tauri API needed.
export function listenNativeEvent(event, handler) {
  const internals = window.__TAURI_INTERNALS__;
  let active = true;
  let eventId;
  const callbackId = internals.transformCallback((raw) => {
    if (active) handler(raw.payload);
  });
  const unlisten = (id) => {
    // Disposal must never throw past the Rust closure's destructor.
    try {
      Promise.resolve(internals.invoke("plugin:event|unlisten", { event, eventId: id }))
        .catch(() => {});
    } catch (_) {}
  };
  const dispose = () => {
    if (!active) return;
    active = false;
    internals.unregisterCallback(callbackId);
    if (eventId !== undefined) unlisten(eventId);
  };
  try {
    Promise.resolve(internals.invoke("plugin:event|listen", {
      event, target: { kind: "Any" }, handler: callbackId,
    })).then((id) => {
      eventId = id;
      if (!active) unlisten(id);
    }, dispose);
  } catch (error) {
    dispose();
    throw error;
  }
  return dispose;
}
