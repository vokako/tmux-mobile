// Two REAL connections at once (board #335 ①).
//
// `ws.test.ts` proves the facade still behaves as it always did; this file
// proves the thing the facade now stands on: that two `createConnection()`
// objects share nothing. The interesting cases are all collisions — the same
// request id, the same pane target, the same room name, the same moment —
// because before the split those were not collisions, they were the single
// module's state. Every claim has a negative control: an assertion that would
// hold if the two were still sharing.
//
// Real Web Crypto, a real server half of the handshake per connection
// (`connection.fixture.ts`), no stubbing of our own transport.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  FakeE2eServer, MockWebSocket, handshake, handshakePlain,
  installDoubles, settle, until, withWebCrypto,
} from './connection.fixture.ts';

installDoubles();
const { createConnection } = await import('./connection.ts');
const { createWsApi } = await import('./ws-api.ts');

const A_URL = 'ws://a.test/ws';
const B_URL = 'ws://b.test/ws';

/** Two authenticated connections to two different servers. */
async function pair() {
  const serverA = new FakeE2eServer('tok-a', 2, 'machine-a', 'alpha');
  const serverB = new FakeE2eServer('tok-b', 2, 'machine-b', 'bravo');
  const a = createConnection();
  const b = createConnection();
  const socketA = await handshake(a, serverA, A_URL, 'tok-a');
  const socketB = await handshake(b, serverB, B_URL, 'tok-b');
  return {
    a, b, serverA, serverB, socketA, socketB,
    apiA: createWsApi(a), apiB: createWsApi(b),
    done: () => { a.dispose(); b.dispose(); },
  };
}

test('two connections authenticate at the same time and keep their own identity', () => withWebCrypto(async () => {
  const w = await pair();
  assert.equal(w.a.isConnected(), true);
  assert.equal(w.b.isConnected(), true);
  assert.equal(w.a.getMachineId(), 'machine-a');
  assert.equal(w.b.getMachineId(), 'machine-b');
  assert.equal(w.a.getHostname(), 'alpha');
  assert.equal(w.b.getHostname(), 'bravo');
  assert.equal(w.a.url(), A_URL);
  assert.equal(w.b.url(), B_URL);
  w.done();
}));

test('the same request id on both connections resolves on the connection that sent it', () => withWebCrypto(async () => {
  const w = await pair();
  const fromA = w.apiA.listSessions();
  const reqA = await w.serverA.next(w.socketA);
  // Each connection counts its own ids from 1, so the collision below is the
  // normal case, not a contrived one.
  assert.equal(reqA.id, 1);

  // Negative control: A's reply, delivered on B's socket, while B holds no
  // pending call at all. A shared pending map would have resolved A here.
  let aSettled = false;
  fromA.then(() => { aSettled = true; }, () => { aSettled = true; });
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqA.id, result: [{ name: 'wrong-way' }] })));
  await settle();
  assert.equal(aSettled, false, "A's reply cannot arrive through B");

  const fromB = w.apiB.listSessions();
  const reqB = await w.serverB.next(w.socketB);
  assert.equal(reqB.id, 1, 'B numbers its own requests from 1 too');

  // Replies out of order: B first, then A. Neither sees the other's payload.
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqB.id, result: [{ name: 'on-b' }] })));
  w.socketA.binary(await w.serverA.seal(JSON.stringify({ id: reqA.id, result: [{ name: 'on-a' }] })));
  assert.deepEqual((await fromA).map((s: any) => s.name), ['on-a']);
  assert.deepEqual((await fromB).map((s: any) => s.name), ['on-b']);
  w.done();
}));

test('a same-named pane and a same-named room reach only their own connection', () => withWebCrypto(async () => {
  const w = await pair();
  const seenA: string[] = [], seenB: string[] = [];
  const closedA: string[] = [], closedB: string[] = [];
  const roomA: any[] = [], roomB: any[] = [];
  const onA = (_t: string, content?: string) => { seenA.push(content!); };
  const onB = (_t: string, content?: string) => { seenB.push(content!); };
  w.a.addPaneOutputListener('app:0.0', onA);
  w.b.addPaneOutputListener('app:0.0', onB);
  w.a.addPaneClosedListener('app:0.0', (t) => closedA.push(t));
  w.b.addPaneClosedListener('app:0.0', (t) => closedB.push(t));
  w.a.addTeamMessageListener((m) => roomA.push(m));
  w.b.addTeamMessageListener((m) => roomB.push(m));

  const push = (room: string, body: string) =>
    JSON.stringify({ method: 'team_message', params: { message: { room, body } } });
  w.socketA.binary(await w.serverA.seal(JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'from-a' } })));
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'from-b' } })));
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ method: 'pane_closed', params: { target: 'app:0.0' } })));
  w.socketA.binary(await w.serverA.seal(push('proj:app', 'hello from a')));
  w.socketB.binary(await w.serverB.seal(push('proj:app', 'hello from b')));
  await until(() => seenA.length === 1 && seenB.length === 1 && roomA.length === 1 && roomB.length === 1);
  await settle();

  assert.deepEqual(seenA, ['from-a'], 'A saw only its own pane snapshot');
  assert.deepEqual(seenB, ['from-b'], 'B saw only its own');
  assert.deepEqual(closedA, [], 'B closing app:0.0 says nothing about A\'s app:0.0');
  assert.deepEqual(closedB, ['app:0.0']);
  assert.deepEqual(roomA.map(m => m.body), ['hello from a'], 'the same room name on two servers is two rooms');
  assert.deepEqual(roomB.map(m => m.body), ['hello from b']);
  w.done();
}));

test('a pane push for a target only the other connection watches reaches nobody', () => withWebCrypto(async () => {
  const w = await pair();
  const seenA: string[] = [];
  w.a.addPaneOutputListener('only-on-a:0.0', (_t, c) => seenA.push(c!));
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ method: 'pane_output', params: { target: 'only-on-a:0.0', content: 'leak' } })));
  await settle();
  assert.deepEqual(seenA, [], 'B has no route to a listener registered on A');
  w.done();
}));

test('subscription refcounts are per connection, and a reconnect restores only its own targets', () => withWebCrypto(async () => {
  const w = await pair();
  const wire = async (server: FakeE2eServer, socket: MockWebSocket) =>
    (await server.drain(socket)).map(m => `${m.method} ${m.params.target}`);

  // Two subscribers on one connection are one wire subscription.
  w.a.subscribe('app:0.0');
  w.a.subscribe('app:0.0');
  w.b.subscribe('app:0.0');
  await until(() => w.socketA.sent.length >= 2 && w.socketB.sent.length >= 2);
  await settle();
  assert.deepEqual(await wire(w.serverA, w.socketA), ['subscribe app:0.0'], 'the 0→1 transition sends once');
  assert.deepEqual(await wire(w.serverB, w.socketB), ['subscribe app:0.0'], 'B subscribes for itself');

  // Dropping one of two subscribers must not cut the survivor's feed.
  w.a.unsubscribe('app:0.0');
  await settle();
  assert.deepEqual(await wire(w.serverA, w.socketA), [], '2→1 sends nothing');
  w.a.unsubscribe('app:0.0');
  await until(() => w.socketA.sent.length >= 3);
  await settle();
  assert.deepEqual(await wire(w.serverA, w.socketA), ['unsubscribe app:0.0'], '1→0 sends once');
  assert.deepEqual(await wire(w.serverB, w.socketB), [], "A's last unsubscribe is not B's");

  // A reconnect replaces A's socket inside the same object; the server forgot
  // the subscriptions, the mounted cells did not.
  w.a.subscribe('app:0.0');
  w.a.subscribe('app:0.1');
  await settle();
  await wire(w.serverA, w.socketA);
  const serverA2 = new FakeE2eServer('tok-a', 2, 'machine-a', 'alpha');
  const socketA2 = await handshake(w.a, serverA2, A_URL, 'tok-a');
  w.a.resubscribeActive();
  await until(() => socketA2.sent.length >= 3);
  await settle();
  assert.deepEqual((await wire(serverA2, socketA2)).sort(), ['subscribe app:0.0', 'subscribe app:0.1']);
  assert.deepEqual(await wire(w.serverB, w.socketB), [], "A's reconnect does not resubscribe B");

  // The refcounts did not double: one unsubscribe per target reaches 0.
  w.a.unsubscribe('app:0.0');
  w.a.unsubscribe('app:0.1');
  await until(() => socketA2.sent.length >= 5);
  await settle();
  assert.deepEqual((await wire(serverA2, socketA2)).sort(), ['unsubscribe app:0.0', 'unsubscribe app:0.1']);
  w.done();
}));

test('an RPC timeout on one connection leaves the other pending call alone', () => withWebCrypto(async () => {
  const w = await pair();
  const slowB = w.apiB.listSessions();
  const reqB = await w.serverB.next(w.socketB);
  const timingOut = w.a.call('list_sessions', {}, 20);
  await assert.rejects(timingOut, /request timeout/u);
  assert.equal(w.b.isConnected(), true, "A's timeout is not B's problem");

  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqB.id, result: [{ name: 'still-here' }] })));
  assert.deepEqual((await slowB).map((s: any) => s.name), ['still-here']);
  w.done();
}));

test('a lost socket notifies only its own recovery callback, and the survivor still calls', () => withWebCrypto(async () => {
  const w = await pair();
  let recoverA = 0, recoverB = 0;
  w.a.setOnDisconnect(() => recoverA++);
  w.b.setOnDisconnect(() => recoverB++);
  const strandedA = w.apiA.listSessions();
  await w.serverA.next(w.socketA);

  w.socketA.readyState = MockWebSocket.CLOSED;
  w.socketA.onclose!({ code: 1006, reason: '', wasClean: false });
  await assert.rejects(strandedA, { code: 'DISCONNECTED' });
  assert.equal(recoverA, 1);
  assert.equal(recoverB, 0, 'B never lost anything');
  assert.equal(w.a.isConnected(), false);
  assert.equal(w.b.isConnected(), true);

  const fromB = w.apiB.listSessions();
  const reqB = await w.serverB.next(w.socketB);
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqB.id, result: [] })));
  assert.deepEqual(await fromB, [], 'B keeps working through A s outage');
  assert.equal(recoverB, 0);
  w.done();
}));

test('a failed authentication on a third connection disturbs neither live one', () => withWebCrypto(async () => {
  const w = await pair();
  let recoverA = 0, recoverB = 0;
  w.a.setOnDisconnect(() => recoverA++);
  w.b.setOnDisconnect(() => recoverB++);

  const c = createConnection();
  const serverC = new FakeE2eServer('tok-c', 2, 'machine-c', 'charlie');
  const dialing = c.connect('ws://c.test/ws', 'tok-c');
  const socketC = MockWebSocket.instances.at(-1)!;
  socketC.readyState = MockWebSocket.OPEN;
  socketC.message(serverC.nonceFrame());
  await until(() => socketC.sent.length >= 1);
  // A text frame where the encrypted auth response belongs: refused.
  socketC.message({ error: { message: 'bad token' } });
  await assert.rejects(dialing, /auth failed/u);

  assert.equal(w.a.isConnected(), true);
  assert.equal(w.b.isConnected(), true);
  assert.equal(recoverA + recoverB, 0, 'a never-authenticated connection asks nobody to recover');
  c.dispose();
  w.done();
}));

test('dispose releases its own connection and nothing else', () => withWebCrypto(async () => {
  const w = await pair();
  const strandedA = w.apiA.listSessions();
  await w.serverA.next(w.socketA);
  const seenB: string[] = [];
  w.b.addPaneOutputListener('app:0.0', (_t, c) => seenB.push(c!));
  w.b.subscribe('app:0.0');
  await settle();
  await w.serverB.drain(w.socketB);

  w.a.dispose();
  await assert.rejects(strandedA, { code: 'DISCONNECTED' });
  assert.equal(w.a.isConnected(), false);
  assert.equal(w.socketA.closed, 1);
  assert.equal(w.b.isConnected(), true);
  assert.equal(w.socketB.closed, 0);

  w.socketB.binary(await w.serverB.seal(JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'alive' } })));
  await until(() => seenB.length === 1);
  assert.deepEqual(seenB, ['alive'], "B's listeners and cipher survived A's dispose");
  const fromB = w.apiB.listSessions();
  const reqB = await w.serverB.next(w.socketB);
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqB.id, result: [] })));
  assert.deepEqual(await fromB, []);
  w.b.dispose();
}));

test('encryption that finishes after a reconnect reaches neither socket', () => withWebCrypto(async () => {
  const w = await pair();
  // ≥256 bytes goes through CompressionStream, so the encrypt resolves several
  // microtasks after the call — long enough for a reconnect to land first.
  const big = 'x'.repeat(4000);
  const doomed = w.apiA.sendKeys('app:0.0', big);
  const rejected = assert.rejects(doomed, { code: 'DISCONNECTED' });
  const beforeA = w.socketA.sent.length;
  const beforeB = w.socketB.sent.length;

  const serverA2 = new FakeE2eServer('tok-a', 2, 'machine-a', 'alpha');
  const socketA2 = await handshake(w.a, serverA2, A_URL, 'tok-a');
  await rejected;
  await settle();

  assert.equal(w.socketA.sent.length, beforeA, 'the replaced socket was already closed');
  assert.equal(socketA2.sent.length, 1, "only A's own handshake went out on the new socket");
  assert.equal(w.socketB.sent.length, beforeB, "A's late ciphertext never reaches B");
  w.a.dispose();
  w.b.dispose();
}));

test('a signed download URL is resolved against the origin of the connection that asked', () => withWebCrypto(async () => {
  const w = await pair();
  const fromA = w.apiA.fsDownloadHttp('/p/same-name.png');
  const fromB = w.apiB.fsDownloadHttp('/p/same-name.png');
  const reqA = await w.serverA.next(w.socketA);
  const reqB = await w.serverB.next(w.socketB);
  assert.equal(reqA.method, 'fs_download_url');
  assert.equal(reqB.method, 'fs_download_url');

  // B answers first and becomes the last connection anyone heard from.
  w.socketB.binary(await w.serverB.seal(JSON.stringify({ id: reqB.id, result: { url: '/dl?sig=b', name: 'same-name.png' } })));
  await fromB;
  w.socketA.binary(await w.serverA.seal(JSON.stringify({ id: reqA.id, result: { url: '/dl?sig=a', name: 'same-name.png' } })));

  const a = await fromA;
  const b = await fromB;
  assert.equal(a.url, 'http://a.test/dl?sig=a');
  assert.equal(b.url, 'http://b.test/dl?sig=b');
  // The control: one shared "current URL" would have given both the same base.
  assert.notEqual(new URL(a.url).host, new URL(b.url).host);
  w.done();
}));

// ─── Lifetime: disconnect keeps the room, dispose empties it ─────────────
// The plain-token path, because what is under test is bookkeeping, not crypto.

test('disconnect keeps listeners and refcounts; dispose is what releases them', async () => {
  const conn = createConnection();
  const api = createWsApi(conn);
  const first = await handshakePlain(conn, A_URL, 'tok', 'machine-a');
  const seen: string[] = [];
  const listener = (_t: string, c?: string) => { seen.push(c!); };
  conn.addPaneOutputListener('app:0.0', listener);
  conn.subscribe('app:0.0');
  await settle(2); // sendOnSocket queues behind the socket's send chain
  assert.equal(first.texts().filter(m => m.method === 'subscribe').length, 1);

  // A deliberate disconnect: pending rejects, the registries stay.
  const stranded = api.listSessions();
  conn.disconnect();
  await assert.rejects(stranded, { code: 'DISCONNECTED' });

  const second = await handshakePlain(conn, A_URL, 'tok', 'machine-a');
  conn.resubscribeActive();
  await settle(2);
  assert.deepEqual(
    second.texts().filter(m => m.method === 'subscribe').map(m => m.params.target),
    ['app:0.0'],
    'the refcount survived, so the wire subscription came back',
  );
  second.onmessage!({ data: JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'back' } }) });
  await settle(2);
  assert.deepEqual(seen, ['back'], 'the listener survived too');

  // dispose: the registries are emptied, so nothing is left to restore.
  conn.dispose();
  const third = await handshakePlain(conn, A_URL, 'tok', 'machine-a');
  conn.resubscribeActive();
  await settle(2);
  assert.deepEqual(third.texts().filter(m => m.method === 'subscribe'), [], 'dispose dropped the refcounts');
  third.onmessage!({ data: JSON.stringify({ method: 'pane_output', params: { target: 'app:0.0', content: 'gone' } }) });
  await settle(2);
  assert.deepEqual(seen, ['back'], 'dispose dropped the listener');
  conn.dispose();
});

test('dispose is idempotent and a disposed connection still refuses RPCs cleanly', async () => {
  const conn = createConnection();
  const api = createWsApi(conn);
  await handshakePlain(conn, A_URL, 'tok', 'machine-a');
  let recover = 0;
  conn.setOnDisconnect(() => recover++);
  conn.dispose();
  conn.dispose();
  conn.dispose();
  await assert.rejects(api.listSessions(), { code: 'DISCONNECTED' });
  assert.equal(recover, 0, 'a deliberate teardown never asks for recovery');
});
