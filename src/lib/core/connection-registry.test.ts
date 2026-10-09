// The registry's whole job is object lifetime (board #335 ①), so that is all
// this file asserts: one object per key, nothing dialled on its behalf, one
// removal disposing exactly one connection, and a teardown that leaves no
// socket and no timer behind.
import test from 'node:test';
import assert from 'node:assert/strict';
import { MockWebSocket, handshakePlain, installDoubles, trackTimers } from './connection.fixture.ts';

installDoubles();
const { createConnectionRegistry } = await import('./connection-registry.ts');

test('ensure is idempotent per key and never dials', () => {
  const registry = createConnectionRegistry();
  const before = MockWebSocket.instances.length;
  const first = registry.ensure('a');
  const again = registry.ensure('a');
  const other = registry.ensure('b');

  assert.equal(again, first, 'the same key is the same slot');
  assert.equal(again.connection, first.connection, 'and the same connection object');
  assert.equal(again.api, first.api, 'and the same bound API');
  assert.notEqual(other.connection, first.connection, 'a different key is a different connection');
  assert.equal(MockWebSocket.instances.length, before, 'ensure opened no socket');
  assert.equal(first.connection.isConnected(), false);
  assert.equal(first.connection.url(), null, 'an idle connection has no address yet');
  assert.equal(registry.get('a'), first);
  assert.equal(registry.get('nope'), undefined);
  registry.disposeAll();
});

test('a Symbol key works, which is what the ws.ts facade uses', () => {
  const registry = createConnectionRegistry();
  const key = Symbol('single-current');
  assert.equal(registry.ensure(key), registry.ensure(key));
  assert.notEqual(registry.ensure(key), registry.ensure(Symbol('single-current')),
    'two Symbols of the same description are two keys');
  registry.disposeAll();
});

test('remove disposes exactly one connection', async () => {
  const registry = createConnectionRegistry();
  const a = registry.ensure('a');
  const b = registry.ensure('b');
  const socketA = await handshakePlain(a.connection, 'ws://a.test/ws', 'tok', 'machine-a');
  const socketB = await handshakePlain(b.connection, 'ws://b.test/ws', 'tok', 'machine-b');
  const strandedA = a.api.listSessions();

  registry.remove('a');
  await assert.rejects(strandedA, { code: 'DISCONNECTED' });
  assert.equal(registry.get('a'), undefined);
  assert.equal(socketA.readyState, MockWebSocket.CLOSED);
  assert.equal(a.connection.isConnected(), false);

  assert.equal(registry.get('b'), b, 'B is still registered');
  assert.equal(socketB.readyState, MockWebSocket.OPEN);
  assert.equal(b.connection.isConnected(), true);
  assert.equal(b.connection.getMachineId(), 'machine-b');

  registry.remove('never-registered'); // a no-op, not a throw
  assert.equal(registry.get('b'), b);
  registry.disposeAll();
});

test('disposeAll empties this registry and leaves no socket or timer behind', async () => {
  const timers = trackTimers();
  try {
    const registry = createConnectionRegistry();
    const other = createConnectionRegistry();
    const a = registry.ensure('a');
    const b = registry.ensure('b');
    const outsider = other.ensure('a');
    const sockets = [
      await handshakePlain(a.connection, 'ws://a.test/ws', 'tok', 'machine-a'),
      await handshakePlain(b.connection, 'ws://b.test/ws', 'tok', 'machine-b'),
    ];
    const outsiderSocket = await handshakePlain(outsider.connection, 'ws://c.test/ws', 'tok', 'machine-c');
    assert.equal(timers.intervals(), 3, 'each authenticated connection runs one idle probe');
    assert.equal(timers.timeouts(), 0, 'no dial is left unfinished');

    registry.disposeAll();
    assert.equal(registry.get('a'), undefined);
    assert.equal(registry.get('b'), undefined);
    for (const socket of sockets) assert.equal(socket.readyState, MockWebSocket.CLOSED);
    assert.equal(timers.live(), 1, 'only the other registry s probe is still running');
    assert.equal(timers.timeouts(), 0);
    assert.equal(outsiderSocket.readyState, MockWebSocket.OPEN, 'another registry is not ours to tear down');
    assert.equal(other.get('a'), outsider);

    // Idempotent, and a later ensure is a FRESH object, not a disposed one.
    registry.disposeAll();
    assert.notEqual(registry.ensure('a').connection, a.connection);
    registry.disposeAll();
    other.disposeAll();
    assert.equal(timers.live(), 0);
  } finally {
    timers.restore();
  }
});
