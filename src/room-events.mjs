// room-events.mjs — Gate-2 S1C: authenticated durable SSE fanout.
// Frozen by gate2-s1c-freeze.md v3 (APPROVED) in ocean-surface
// .stitchpad/artifacts. This module is the PROTOCOL STATE MACHINE ONLY:
// auth, F3, permission, scope, membership, and row visibility are
// SERVER-OWNED and injected as adapter functions — no gate is implemented
// here, so gate behavior cannot fork from the route handlers'.
//
// Protocol (frozen): selected-cursor precedence → H (cursorless tail read) →
// hello → snapshot (selectedCursor, H] → ONE live poll from
// max(selectedCursor, H). Emitted room_event ids are strictly increasing
// with no eligible/visible room row omitted.

import { parseAfterSequence } from './ledger.mjs';

export const POLL_INTERVAL_MS = 1000;
export const POLL_JITTER_MS = 100;
export const PAGE_LIMIT = 500;
export const HEARTBEAT_INTERVAL_MS = 15000;
export const QUEUE_BOUND = 1024;

// Frozen cursor precedence (standard SSE reconnect semantics): a canonical
// Last-Event-ID wins when present — an EventSource-style reconnect retains
// its original URL, so a stale query param plus a newer header is normal.
// Else after_sequence; else '0'. The SELECTED value is validated strictly;
// a malformed selected cursor throws (server maps to 400).
export function selectCursor({ lastEventId, afterSequence } = {}) {
  if (lastEventId !== null && lastEventId !== undefined) {
    return parseAfterSequence(lastEventId);
  }
  if (afterSequence !== null && afterSequence !== undefined) {
    return parseAfterSequence(afterSequence);
  }
  return '0';
}

function bigMax(a, b) {
  return BigInt(a) >= BigInt(b) ? String(a) : String(b);
}

// Frame constructors — exact frozen payload shapes. Pinned-string fields are
// the SSE id and every control-frame cursor field; room_event data preserves
// the native GET /ledger/events row shape.
export function helloFrame(roomId, highWater) {
  return { event: 'hello', data: { room_id: roomId, snapshot_high_water: String(highWater) } };
}
export function roomEventFrame(row) {
  return { event: 'room_event', id: String(row.sequence), data: row };
}
export function heartbeatFrame(lastAcceptedId) {
  return { event: 'heartbeat', data: { sequence: String(lastAcceptedId) } };
}
export function resyncFrame(lastAcceptedId) {
  return { event: 'resync_required', data: { after_sequence: String(lastAcceptedId) } };
}
export function revokedFrame(reason) {
  return { event: 'revoked', data: { reason } };
}

/**
 * Create the per-subscriber protocol state machine.
 *
 * adapters (ALL server-owned; the machine never implements a gate):
 *   readHead()                      → Promise<string> — newest room sequence
 *                                     via ONE cursorless scoped tail read
 *                                     (limit=1, correlation+exact path);
 *                                     '0' for an empty room.
 *   readPage(afterSequence, limit)  → Promise<row[]> — the S1B strict cursor
 *                                     read with BOTH correlation_id and
 *                                     exact path filters, ascending.
 *   isRowVisible(row)               → boolean — ledgerEventVisibleToPrincipal
 *                                     (path scope + clearance).
 *   reauthenticate()                → Promise<{ok:true} |
 *                                     {ok:false, reason:'membership_revoked'|'token_invalid'}>
 *                                     — re-runs the FULL gate from the stored
 *                                     raw header bearer. Throwing = transport
 *                                     failure, NEVER a revocation.
 *   write(frame)                    → boolean — accepted/written to the
 *                                     socket in stream order (Node cannot
 *                                     prove remote flush); false = socket
 *                                     backpressure.
 *   waitDrain()                     → Promise<void> — resolves when the
 *                                     socket is writable again.
 *   close(kind)                     → void — kind: 'resync' | 'revoked' |
 *                                     'transport' | 'done'.
 *   timers                          → { setTimeout, clearTimeout } —
 *                                     injectable for deterministic tests.
 *   random                          → () => number in [0,1) for jitter.
 */
export function createRoomEventStream({ roomId, selectedCursor, adapters }) {
  const {
    readHead, readPage, isRowVisible, reauthenticate, write, waitDrain, close,
    timers = { setTimeout: globalThis.setTimeout.bind(globalThis), clearTimeout: globalThis.clearTimeout.bind(globalThis) },
    random = Math.random,
  } = adapters;

  const state = {
    phase: 'idle',            // idle → hello → snapshot → live → closed
    readCursor: selectedCursor,
    enqueuedCursor: selectedCursor,
    lastAcceptedId: selectedCursor,
    queue: [],
    inFlightRead: false,
    closed: false,
    pollTimer: null,
    heartbeatTimer: null,
    drainWaiter: null,
  };

  function clearTimers() {
    if (state.pollTimer !== null) { timers.clearTimeout(state.pollTimer); state.pollTimer = null; }
    if (state.heartbeatTimer !== null) { timers.clearTimeout(state.heartbeatTimer); state.heartbeatTimer = null; }
  }

  function shutdown(kind) {
    if (state.closed) return;
    state.closed = true;
    state.phase = 'closed';
    clearTimers();
    state.queue.length = 0;
    // Release any drain wait so no promise dangles past close.
    if (state.drainWaiter) { state.drainWaiter(); state.drainWaiter = null; }
    close(kind);
  }

  // Emit one frame with backpressure. Only room_event frames advance
  // lastAcceptedId — control frames never move a cursor.
  async function emit(frame) {
    if (state.closed) return false;
    const accepted = write(frame);
    if (frame.event === 'room_event') state.lastAcceptedId = frame.id;
    if (!accepted && !state.closed) {
      await new Promise((resolve) => {
        state.drainWaiter = resolve;
        Promise.resolve(waitDrain()).then(() => {
          if (state.drainWaiter === resolve) { state.drainWaiter = null; resolve(); }
        }, () => {
          if (state.drainWaiter === resolve) { state.drainWaiter = null; resolve(); }
        });
      });
    }
    return !state.closed;
  }

  // Independent queue pump: drains to the socket in order, awaiting drain on
  // backpressure. Reads NEVER await the pump — they keep enqueueing while
  // the socket is blocked (frozen queue model), which is exactly what makes
  // the overflow bound reachable.
  let pumpRunning = false;
  function kickPump() {
    if (pumpRunning || state.closed) return;
    pumpRunning = true;
    (async () => {
      try {
        while (!state.closed && state.queue.length) {
          const frame = state.queue.shift();
          // eslint-disable-next-line no-await-in-loop
          if (!(await emit(frame))) return;
        }
      } catch {
        shutdown('transport');   // a socket write throw is transport, and never an unhandled rejection
      } finally {
        pumpRunning = false;
      }
    })();
  }

  // Enqueue a VISIBLE row. Frozen overflow: the queue already holds
  // QUEUE_BOUND entries AND this next visible row exists (this call IS the
  // headroom probe finding it) → resync_required with lastAcceptedId, close.
  function enqueueVisible(row) {
    if (state.queue.length >= QUEUE_BOUND) {
      write(resyncFrame(state.lastAcceptedId));   // best-effort under blocked socket
      shutdown('resync');
      return false;
    }
    state.queue.push(roomEventFrame(row));
    state.enqueuedCursor = String(row.sequence);
    return true;
  }

  // One live read from readCursor. Filtered rows advance readCursor ONLY.
  // Returns whether the page was full (catch-up should continue).
  async function readOnce() {
    if (state.inFlightRead || state.closed) return false;
    state.inFlightRead = true;
    try {
      const rows = await readPage(state.readCursor, PAGE_LIMIT);
      if (state.closed) return false;   // discard in-flight result after close
      for (const row of rows) {
        state.readCursor = String(row.sequence);
        if (!isRowVisible(row)) continue;
        if (!enqueueVisible(row)) return false;
      }
      kickPump();
      return rows.length === PAGE_LIMIT;
    } finally {
      state.inFlightRead = false;
    }
  }

  function scheduleNextPoll() {
    if (state.closed) return;
    const jitter = Math.floor((random() * 2 - 1) * POLL_JITTER_MS);
    state.pollTimer = timers.setTimeout(pollTick, POLL_INTERVAL_MS + jitter);
  }

  async function pollTick() {
    if (state.closed) return;
    try {
      let full = await readOnce();
      // Catch-up: keep reading while pages come back full, yielding a
      // macrotask between pages so the heartbeat timer cannot be starved.
      while (full && !state.closed) {
        // eslint-disable-next-line no-await-in-loop
        await new Promise((resolve) => timers.setTimeout(resolve, 0));
        // eslint-disable-next-line no-await-in-loop
        full = await readOnce();
      }
    } catch {
      shutdown('transport');
      return;
    }
    scheduleNextPoll();
  }

  function scheduleHeartbeat() {
    if (state.closed) return;
    state.heartbeatTimer = timers.setTimeout(heartbeatTick, HEARTBEAT_INTERVAL_MS);
  }

  async function heartbeatTick() {
    if (state.closed) return;
    try {
      await emit(heartbeatFrame(state.lastAcceptedId));
      const verdict = await reauthenticate();
      if (state.closed) return;
      if (!verdict.ok) {
        // Authz DENIAL (membership/scope/permission/token) → terminal revoked.
        write(revokedFrame(verdict.reason || 'token_invalid'));
        shutdown('revoked');
        return;
      }
    } catch {
      // Unexpected DB/I/O failure → transport close, NEVER falsely revoked.
      shutdown('transport');
      return;
    }
    scheduleHeartbeat();
  }

  async function start() {
    if (state.phase !== 'idle') throw new Error('stream already started');
    state.phase = 'hello';
    let highWater;
    try {
      highWater = await readHead();
    } catch {
      shutdown('transport');
      return;
    }
    if (state.closed) return;
    await emit(helloFrame(roomId, highWater));
    scheduleHeartbeat();

    // Snapshot (selectedCursor, H] on its own raw cursor; stops past H.
    state.phase = 'snapshot';
    const H = BigInt(highWater);
    let snapshotCursor = selectedCursor;
    try {
      while (!state.closed && BigInt(snapshotCursor) < H) {
        // eslint-disable-next-line no-await-in-loop
        const rows = await readPage(snapshotCursor, PAGE_LIMIT);
        if (state.closed || !rows.length) break;
        let sawPastH = false;
        for (const row of rows) {
          const seq = BigInt(row.sequence);
          if (seq > H) { sawPastH = true; break; }   // live owns > H
          snapshotCursor = String(row.sequence);
          state.readCursor = bigMax(state.readCursor, snapshotCursor);
          if (!isRowVisible(row)) continue;
          if (!enqueueVisible(row)) return;
        }
        kickPump();
        if (sawPastH || rows.length < PAGE_LIMIT) break;
        // Yield between snapshot pages — same starvation rule as catch-up.
        // eslint-disable-next-line no-await-in-loop
        await new Promise((resolve) => timers.setTimeout(resolve, 0));
      }
    } catch {
      shutdown('transport');
      return;
    }
    if (state.closed) return;

    // ONE live poll from max(selectedCursor, H). No pre-snapshot subscriber:
    // appends are durable, so rows landing after H during the snapshot are
    // picked up by the first > cursor live read.
    state.phase = 'live';
    state.readCursor = bigMax(state.readCursor, highWater);
    await pollTick();
  }

  return {
    start,
    abort: () => shutdown('done'),
    // test/inspection surface — read-only view of positions and phase
    inspect: () => ({
      phase: state.phase,
      readCursor: state.readCursor,
      enqueuedCursor: state.enqueuedCursor,
      lastAcceptedId: state.lastAcceptedId,
      queueLength: state.queue.length,
      closed: state.closed,
      timers: { poll: state.pollTimer !== null, heartbeat: state.heartbeatTimer !== null },
    }),
  };
}
