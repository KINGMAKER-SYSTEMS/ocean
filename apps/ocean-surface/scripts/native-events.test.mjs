import assert from 'node:assert/strict';
import { test } from 'node:test';
import { listenNativeEvent } from '../crates/ocean-surface-ui/src/host/native_events.mjs';

function fixture({ failure, synchronous = false } = {}) {
  const callbacks = new Map();
  const calls = [];
  let resolve;
  let reject;
  const pending = new Promise((yes, no) => { resolve = yes; reject = no; });
  globalThis.window = { __TAURI_INTERNALS__: {
    transformCallback(callback, once = false) {
      assert.equal(once, false);
      callbacks.set(17, callback);
      return 17;
    },
    unregisterCallback(id) { calls.push(['unregister', id]); callbacks.delete(id); },
    invoke(command, args) {
      calls.push([command, args]);
      if (command === 'plugin:event|listen') {
        assert.deepEqual(args, { event: 'path-changed', target: { kind: 'Any' }, handler: 17 });
        if (synchronous) throw failure;
        return pending;
      }
      assert.equal(command, 'plugin:event|unlisten');
      assert.deepEqual(args, { event: 'path-changed', eventId: 23 });
      return Promise.resolve();
    },
  } };
  return { callbacks, calls, resolve, reject };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test('delivers payloads repeatedly, then cancels callback before native unlisten', async () => {
  const f = fixture();
  const received = [];
  const dispose = listenNativeEvent('path-changed', payload => received.push(payload));
  const queuedCallback = f.callbacks.get(17);
  f.resolve(23);
  await flush();
  queuedCallback({ payload: { path: '/root/a' } });
  queuedCallback({ payload: { path: '/root/b' } });
  assert.deepEqual(received, [{ path: '/root/a' }, { path: '/root/b' }]);
  dispose();
  assert.equal(f.callbacks.size, 0);
  queuedCallback({ payload: { path: '/root/late' } });
  dispose();
  assert.equal(received.length, 2);
  assert.deepEqual(f.calls.slice(1), [
    ['unregister', 17],
    ['plugin:event|unlisten', { event: 'path-changed', eventId: 23 }],
  ]);
});

test('cancellation before admission rejects queued delivery and unlistens late admission', async () => {
  const f = fixture();
  const dispose = listenNativeEvent('path-changed', () => assert.fail('retired callback ran'));
  const queuedCallback = f.callbacks.get(17);
  dispose();
  assert.equal(f.callbacks.size, 0);
  queuedCallback({ payload: {} });
  f.resolve(23);
  await flush();
  queuedCallback({ payload: {} });
  assert.deepEqual(f.calls.slice(1), [
    ['unregister', 17],
    ['plugin:event|unlisten', { event: 'path-changed', eventId: 23 }],
  ]);
});

test('failed admission unregisters the callback without a nonexistent native listener', async () => {
  const f = fixture();
  const dispose = listenNativeEvent('path-changed', () => assert.fail('failed callback ran'));
  const queuedCallback = f.callbacks.get(17);
  f.reject(new Error('admission rejected'));
  await flush();
  assert.equal(f.callbacks.size, 0);
  queuedCallback({ payload: {} });
  dispose();
  assert.deepEqual(f.calls.slice(1), [['unregister', 17]]);
});

test('synchronous IPC failure also unregisters the Rust callback', () => {
  const f = fixture({ failure: new Error('IPC unavailable'), synchronous: true });
  assert.throws(() => listenNativeEvent('path-changed', () => {}), /IPC unavailable/);
  assert.equal(f.callbacks.size, 0);
  assert.deepEqual(f.calls.slice(1), [['unregister', 17]]);
});
