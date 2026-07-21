import test from 'node:test';
import assert from 'node:assert/strict';

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static instances = [];

  constructor(url) {
    this.url = url;
    this.readyState = MockWebSocket.CONNECTING;
    this.sent = [];
    MockWebSocket.instances.push(this);
  }

  send(data) {
    if (this.readyState !== MockWebSocket.OPEN) throw new Error('socket is not open');
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
  }

  message(data) {
    this.onmessage?.({ data: JSON.stringify(data) });
  }
}

globalThis.window = {
  __dbg: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
};
globalThis.WebSocket = MockWebSocket;
Object.defineProperty(globalThis, 'crypto', {
  configurable: true,
  value: { subtle: null },
});

const wsClient = await import(`./ws.js?lifecycle=${Date.now()}`);

async function authenticate() {
  const connecting = wsClient.connect('ws://test', 'token');
  const socket = MockWebSocket.instances.at(-1);
  socket.readyState = MockWebSocket.OPEN;
  socket.message({ server_nonce: '00'.repeat(16) });
  socket.message({ result: { authenticated: true, machine_id: 'machine', hostname: 'host' } });
  await connecting;
  return socket;
}

test('an unavailable authenticated socket requests recovery exactly once', async () => {
  let disconnects = 0;
  wsClient.setOnDisconnect(() => disconnects++);
  const socket = await authenticate();
  socket.readyState = MockWebSocket.CLOSED;

  await assert.rejects(wsClient.sendKeys('s:0.0', 'a'), { code: 'DISCONNECTED' });
  await assert.rejects(wsClient.sendKeys('s:0.0', 'b'), { code: 'DISCONNECTED' });
  assert.equal(disconnects, 1);

  wsClient.disconnect();
});

test('manual disconnect never requests recovery', async () => {
  let disconnects = 0;
  wsClient.setOnDisconnect(() => disconnects++);
  await authenticate();

  wsClient.disconnect();
  await assert.rejects(wsClient.sendKeys('s:0.0', 'a'), { code: 'DISCONNECTED' });
  assert.equal(disconnects, 0);
});

test('a stale close handler cannot clear a replacement connection', async () => {
  let disconnects = 0;
  wsClient.setOnDisconnect(() => disconnects++);
  const first = await authenticate();
  const staleClose = first.onclose;
  await authenticate();

  staleClose({ code: 1006, reason: '', wasClean: false });
  assert.equal(wsClient.isConnected(), true);
  assert.equal(disconnects, 0);

  wsClient.disconnect();
});

test('an async send from an old socket is discarded after replacement', async () => {
  const first = await authenticate();
  wsClient.subscribe('s:0.0');

  const reconnecting = wsClient.connect('ws://test', 'token');
  const second = MockWebSocket.instances.at(-1);
  second.readyState = MockWebSocket.OPEN;
  second.message({ server_nonce: '00'.repeat(16) });
  second.message({ result: { authenticated: true, machine_id: 'machine', hostname: 'host' } });
  await reconnecting;
  await Promise.resolve();

  assert.equal(first.sent.some(frame => String(frame).includes('subscribe')), false);
  assert.equal(second.sent.some(frame => String(frame).includes('subscribe')), false);

  wsClient.unsubscribe('s:0.0');
  wsClient.disconnect();
});

test('concurrent RPC frames preserve request order on one socket', async () => {
  const socket = await authenticate();
  const first = wsClient.sendKeys('s:0.0', 'a');
  const second = wsClient.sendKeys('s:0.0', 'b');
  for (let i = 0; i < 5 && socket.sent.filter(frame => typeof frame === 'string' && frame.includes('send_keys')).length < 2; i++) {
    await new Promise(resolve => setImmediate(resolve));
  }

  const requests = socket.sent
    .map(frame => typeof frame === 'string' ? JSON.parse(frame) : null)
    .filter(message => message?.method === 'send_keys');
  assert.deepEqual(requests.map(message => message.params.keys), ['a', 'b']);

  for (const request of requests) socket.message({ id: request.id, result: { ok: true } });
  await Promise.all([first, second]);
  wsClient.disconnect();
});
