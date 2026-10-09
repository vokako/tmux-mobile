// A→B→A through the REAL ws.ts facade (board #335 ①).
//
// `server-switch.test.ts` drives the #315 switch against a hand-written fake
// transport: it proves the ORDER and the storage rules, not this module. What
// is unproven there is the compatibility slot itself — that the facade's one
// memory-only Symbol key still behaves exactly like the module-global socket
// it replaced: one live socket at a time, every pending RPC of the server we
// left rejected, and the listener/refcount registries surviving the swap
// because they always did (a Terminal stays mounted in a hidden page layer,
// and `resubscribeActive()` is what brings its wire subscription back).
//
// Everything here goes through the facade's own exports. The plain-token path
// keeps the handshake out of the way; `ws.test.ts` and `connection.test.ts`
// own the crypto.
import test from 'node:test';
import assert from 'node:assert/strict';
import { MockWebSocket, installDoubles, settle } from './connection.fixture.ts';

installDoubles();
const ws = await import(`./ws.ts?facade=${Date.now()}`);

const A = 'ws://a.test/ws';
const B = 'ws://b.test/ws';

/** The facade's own connect, answered as `machineId`. */
async function arriveAt(url: string, machineId: string) {
  const connecting = ws.connect(url, 'tok');
  const socket = MockWebSocket.instances.at(-1)!;
  socket.readyState = MockWebSocket.OPEN;
  socket.message({ server_nonce: '00'.repeat(16) });
  socket.message({ result: { authenticated: true, machine_id: machineId, hostname: machineId } });
  await connecting;
  return socket;
}
const open = () => MockWebSocket.instances.filter(s => s.readyState === MockWebSocket.OPEN);
const subscribesOn = (socket: MockWebSocket) =>
  socket.texts().filter(m => m.method === 'subscribe').map(m => m.params.target);

test('A→B→A through the facade keeps one live socket and one connection object', async () => {
  let recoveries = 0;
  ws.setOnDisconnect(() => recoveries++);
  const seen: string[] = [];
  const listener = (_t: string, content?: string) => { seen.push(content!); };

  // ── On A, with a mounted pane and a call in flight ────────────────────
  const socketA = await arriveAt(A, 'machine-a');
  assert.equal(ws.isConnected(), true);
  assert.equal(ws.getMachineId(), 'machine-a');
  ws.addPaneOutputListener('app:0.0', listener);
  ws.subscribe('app:0.0');
  await settle(2);
  assert.deepEqual(subscribesOn(socketA), ['app:0.0']);
  const strandedOnA = ws.listSessions();
  socketA.message({ method: 'pane_output', params: { target: 'app:0.0', content: 'a-screen' } });
  await settle(2);
  assert.deepEqual(seen, ['a-screen']);

  // ── Switch to B ───────────────────────────────────────────────────────
  const socketB = await arriveAt(B, 'machine-b');
  await assert.rejects(strandedOnA, { code: 'DISCONNECTED' },
    "the server we left cannot answer, so its pending call fails rather than hanging");
  assert.equal(recoveries, 0, 'a deliberate switch is not a connection loss');
  assert.deepEqual(open(), [socketB], 'exactly one live socket, and it is B s');
  assert.equal(socketA.readyState, MockWebSocket.CLOSED);
  assert.equal(ws.getMachineId(), 'machine-b', 'one object, the new socket s identity');
  assert.equal(ws.isConnected(), true);
  // Nothing may be appended to the socket we left from here on.
  const aFramesAtSwitch = socketA.sent.length;

  // The registries survived the swap, exactly as the module-level maps did
  // before the split. In production App unmounts the server-bound tree on a
  // switch and the pages re-register; the transport neither forces that nor
  // prevents it.
  ws.resubscribeActive();
  await settle(2);
  assert.deepEqual(subscribesOn(socketB), ['app:0.0'], 'the refcount came back on B s wire');
  assert.equal(socketA.sent.length, aFramesAtSwitch, 'and nothing new went to the closed socket');

  // A fresh RPC goes out on B only.
  const onB = ws.listSessions();
  await settle(2);
  const reqB = socketB.texts().find(m => m.method === 'list_sessions');
  assert.ok(reqB, 'the call reached B');
  assert.equal(socketA.sent.length, aFramesAtSwitch, 'and not to the socket we left');
  socketB.message({ id: reqB.id, result: [{ name: 'on-b' }] });
  assert.deepEqual((await onB).map((s: any) => s.name), ['on-b']);

  // A push from B for the SAME pane name reaches the listener; a push from the
  // dead A socket reaches nobody.
  socketB.message({ method: 'pane_output', params: { target: 'app:0.0', content: 'b-screen' } });
  socketA.onmessage?.({ data: JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'late-from-a' } }) });
  await settle(2);
  assert.deepEqual(seen, ['a-screen', 'b-screen'], 'the replaced socket is mute');

  // ── Back to A ─────────────────────────────────────────────────────────
  const socketA2 = await arriveAt(A, 'machine-a');
  assert.deepEqual(open(), [socketA2]);
  assert.equal(ws.getMachineId(), 'machine-a');
  ws.resubscribeActive();
  await settle(2);
  assert.deepEqual(subscribesOn(socketA2), ['app:0.0']);
  assert.equal(socketA.sent.length, aFramesAtSwitch, 'the first socket never spoke again');
  assert.equal(MockWebSocket.instances.length, 3, 'three dials, three sockets — the slot opened nothing extra');

  ws.unsubscribe('app:0.0');
  ws.removePaneOutputListener('app:0.0', listener);
  ws.disconnect();
  assert.equal(recoveries, 0, 'no step of the round trip asked for recovery');
});

test('the facade is one connection object, so a reconnect to the same server keeps its registries', async () => {
  const seen: string[] = [];
  const listener = (_t: string, content?: string) => { seen.push(content!); };
  ws.addPaneOutputListener('app:0.1', listener);
  ws.subscribe('app:0.1');

  const first = await arriveAt(A, 'machine-a');
  ws.resubscribeActive();
  await settle(2);
  assert.deepEqual(subscribesOn(first).filter(t => t === 'app:0.1'), ['app:0.1']);

  // A dropped link, then the same server again: the subscription is restored
  // from the refcount the mounted cell still holds, never re-counted.
  first.readyState = MockWebSocket.CLOSED;
  first.onclose!({ code: 1006, reason: '', wasClean: false });
  const second = await arriveAt(A, 'machine-a');
  ws.resubscribeActive();
  await settle(2);
  assert.deepEqual(subscribesOn(second).filter(t => t === 'app:0.1'), ['app:0.1']);
  second.message({ method: 'pane_output', params: { target: 'app:0.1', content: 'resumed' } });
  await settle(2);
  assert.deepEqual(seen, ['resumed']);

  // One unsubscribe reaches 0 — the refcount was not doubled by the restore.
  ws.unsubscribe('app:0.1');
  await settle(2);
  assert.deepEqual(
    second.texts().filter(m => m.method === 'unsubscribe').map(m => m.params.target),
    ['app:0.1'],
  );
  ws.removePaneOutputListener('app:0.1', listener);
  ws.disconnect();
});
