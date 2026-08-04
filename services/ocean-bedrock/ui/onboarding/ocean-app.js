// Ocean onboarding stage bootstrap + event bus.
//
// Imports the globe (it boots itself on import - see the auto-init guard at
// the tail of ocean-three.js), starts the ceremony WebSocket client, and wires
// every ceremony event into a tiny typed bus that the UI layer subscribes to.
// As agent.source_detected events arrive, globe turbulence rises.

import { init, setIntensity, absorbAsset } from './ocean-three.js';
import { createCeremonyClient } from './ocean-ws.js';
import { mount } from './ocean-ui.js';

// ocean-three.js initializes the WebGL globe when this module loads (its tail
// runs init() on DOMContentLoaded or immediately). We import init for the
// contract but must NOT call it again - a second init() would attach a second
// renderer + render loop to the same canvas.
void init;

const REDUCED = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

// --- Tiny event bus: dispatch by ceremony event.type ----------------------
const listeners = new Map();    // event type -> Set<fn>
const wildcard = new Set();     // '*' receives every event

export function on(type, fn) {
  if (type === '*') { wildcard.add(fn); return () => wildcard.delete(fn); }
  let set = listeners.get(type);
  if (!set) { set = new Set(); listeners.set(type, set); }
  set.add(fn);
  return () => set.delete(fn);
}

export function emit(event) {
  if (!event || typeof event.type !== 'string') return;
  const set = listeners.get(event.type);
  if (set) {
    for (const fn of set) {
      try { fn(event); } catch (err) { console.error('[ocean] listener threw', err); }
    }
  }
  for (const fn of wildcard) {
    try { fn(event); } catch (err) { console.error('[ocean] wildcard listener threw', err); }
  }
}

// --- Globe turbulence: rises with detected sources, capped at ~60 ---------
let sourceCount = 0;
on('agent.source_detected', (event) => {
  sourceCount += 1;
  const intensity = Math.min(60, 12 + 4 * sourceCount);
  setIntensity(intensity);
  const logo = event?.payload?.logo;
  if (logo) absorbAsset(`/assets/integrations/${logo}.svg`);
});

// --- Boot -----------------------------------------------------------------
function start() {
  mount({ on, emit, reduced: REDUCED });

  const client = createCeremonyClient();
  // Every ceremony event flows onto the bus; the UI subscribes by type.
  client.subscribe(emit);
  client.onStatus((state, detail) => {
    emit({ type: '__status', phase: 'connection', status: state, message: typeof detail === 'string' ? detail : state, payload: { state, detail } });
  });

  client.start().catch((err) => {
    console.error('[ocean] ceremony client failed to start', err);
    emit({ type: '__status', phase: 'connection', status: 'error', message: err && err.message });
  });
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', start, { once: true });
} else {
  start();
}
