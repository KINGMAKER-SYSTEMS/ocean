// Ocean ceremony WebSocket client.
//
// Resolves a ceremony run, opens /api/ceremony/ws, replays the snapshot, and
// streams live ceremony events to subscribers. Reconnects with capped backoff
// on close. Reduced-motion is honored at the UI layer; the client itself always
// streams events regardless of motion preference.

const REDUCED = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

function queryParam(name) {
  return new URL(location.href).searchParams.get(name);
}

async function postJSON(url, body) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body || {}),
  });
  const text = await res.text();
  let json = {};
  if (text) {
    try { json = JSON.parse(text); }
    catch { json = { ok: false, error: text }; }
  }
  if (!res.ok) {
    const err = new Error((json && json.error) || `HTTP ${res.status}`);
    err.status = res.status;
    err.body = json;
    throw err;
  }
  return json;
}

// Use a runId from the URL if present; otherwise create a ceremony run and
// reflect its id back into the URL so a refresh resumes the same run.
async function ensureRunId() {
  const existing = queryParam('runId');
  if (existing) return existing;
  const res = await postJSON('/api/ceremony/runs', {
    runner: { runtime: 'other', name: 'Ocean stage' },
    ownerName: queryParam('name') || 'ocean-coworker',
    mode: 'onboarding-import-v1',
  });
  const runId = res && res.run && res.run.runId;
  if (!runId) throw new Error('ceremony run was not created');
  const url = new URL(location.href);
  url.searchParams.set('runId', runId);
  history.replaceState({}, '', url.toString());
  return runId;
}

// Factory: a single ceremony client per page. Returns a small handle with
// subscribe(fn) / onStatus(fn) / start() / close().
export function createCeremonyClient() {
  const eventListeners = new Set();
  const statusListeners = new Set();

  let runId = null;
  let ws = null;
  let lastSeq = 0;
  let backoffMs = 800;
  let reconnectTimer = null;
  let pingTimer = null;
  let closedByUser = false;
  let fatal = false;
  let started = false;

  const emit = (event) => {
    for (const fn of eventListeners) {
      try { fn(event); } catch (err) { console.error('[ocean-ws] listener threw', err); }
    }
  };
  const status = (state, detail) => {
    for (const fn of statusListeners) {
      try { fn(state, detail); } catch { /* status is best-effort */ }
    }
  };

  const wsUrl = () => {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${location.host}/api/ceremony/ws?runId=${encodeURIComponent(runId)}&after=${lastSeq}`;
  };

  const sendPing = () => {
    if (ws && ws.readyState === 1) {
      try { ws.send(JSON.stringify({ kind: 'ping' })); } catch { /* socket closing */ }
    }
  };

  const handleFrame = (frame) => {
    if (!frame || typeof frame !== 'object') return;
    switch (frame.kind) {
      case 'hello':
        status('ready', { runId: frame.runId });
        break;
      case 'snapshot': {
        const events = Array.isArray(frame.events) ? frame.events : [];
        for (const e of events) {
          if (e && typeof e.seq === 'number' && e.seq > lastSeq) lastSeq = e.seq;
          emit(e);
        }
        status('snapshot', { count: events.length, lastSeq: frame.lastSeq });
        break;
      }
      case 'event': {
        const e = frame.event;
        if (!e) break;
        if (typeof e.seq === 'number' && e.seq > lastSeq) lastSeq = e.seq;
        emit(e);
        break;
      }
      case 'heartbeat':
        break;
      case 'error':
        // Fatal server-side frame (e.g. run not found). Stop reconnecting so
        // we do not loop on a runId the server will never accept.
        fatal = true;
        status('error', frame.error);
        break;
      default:
        break;
    }
  };

  const open = () => {
    try {
      ws = new WebSocket(wsUrl());
    } catch (err) {
      status('error', err.message);
      scheduleReconnect();
      return;
    }
    ws.addEventListener('open', () => {
      backoffMs = 800;
      status('connected', { runId });
      clearInterval(pingTimer);
      pingTimer = setInterval(sendPing, 25_000);
    });
    ws.addEventListener('message', (msg) => {
      let frame;
      try { frame = JSON.parse(msg.data); } catch { return; }
      handleFrame(frame);
    });
    ws.addEventListener('close', () => {
      clearInterval(pingTimer); pingTimer = null;
      if (closedByUser || fatal) { status(fatal ? 'error' : 'closed'); return; }
      status('reconnecting', { attemptInMs: backoffMs });
      scheduleReconnect();
    });
    ws.addEventListener('error', () => {
      // The close event follows with detail; surface a transitional status.
      status('error', 'websocket error');
    });
  };

  const scheduleReconnect = () => {
    if (closedByUser || fatal) return;
    if (reconnectTimer) return;
    backoffMs = Math.min(Math.round(backoffMs * 1.7), 12_000);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      open();
    }, backoffMs);
  };

  return {
    REDUCED,
    /** Register a listener that receives every ceremony event. Returns an unsubscribe. */
    subscribe(fn) { eventListeners.add(fn); return () => eventListeners.delete(fn); },
    /** Register a status listener: ('connected'|'ready'|'snapshot'|'reconnecting'|'error'|'closed', detail). */
    onStatus(fn) { statusListeners.add(fn); return () => statusListeners.delete(fn); },
    /** Current runId (null until start() resolves). */
    getRunId() { return runId; },
    /** Resolve the run and open the socket. Resolves with the runId. */
    async start() {
      if (started) return runId;
      started = true;
      status('connecting');
      try {
        runId = await ensureRunId();
      } catch (err) {
        status('error', err.message);
        throw err;
      }
      open();
      return runId;
    },
    /** Tear down. No reconnect after this. */
    close() {
      closedByUser = true;
      clearTimeout(reconnectTimer); reconnectTimer = null;
      clearInterval(pingTimer); pingTimer = null;
      if (ws) { try { ws.close(); } catch { /* already gone */ } }
    },
  };
}
