// Two servers held at once, and the identity rules that decide which is which
// (board #335 ②a-2). The transport doubles are phase ①'s fixture, so these
// tests exercise the real Connection and the real `servers.ts` writes — only
// the socket and the clock are fake.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  FakeE2eServer, MockWebSocket, handshake, handshakePlain, installDoubles, settle, trackTimers, until, withWebCrypto,
} from '../core/connection.fixture.ts';
import { createConnectionRegistry, type ConnectionSlot } from '../core/connection-registry.ts';
import { createServerFleet } from './server-fleet.ts';
import { createServerRuntime } from './server-runtime.ts';
import { loadServers, type ServerEntry } from './servers.ts';

installDoubles();

function mem(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => { m.set(k, String(v)); },
    removeItem: (k: string) => { m.delete(k); },
    dump: () => Object.fromEntries(m),
  };
}

const entry = (over: Partial<ServerEntry> & { id: string; address: string }): ServerEntry =>
  ({ name: over.name ?? 'srv', token: 'tok', ...over });

/** A storage holding two saved servers, each knowing its machine. */
function twoServers() {
  const a = entry({ id: 'a', name: 'alpha', address: 'ws://a:9899', token: 'tokA', machineId: 'm-a' });
  const b = entry({ id: 'b', name: 'beta', address: 'ws://b:9899', token: 'tokB', machineId: 'm-b' });
  const storage = mem({
    tmux_servers: JSON.stringify([a, b]),
    tmux_server_current: 'a',
  });
  return { storage, a, b };
}

test('two servers are online at the same time, each with its own transport', async () => {
  const timers = trackTimers();
  const { storage, a, b } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const rb = fleet.include(b);
    assert.notEqual(ra.connection, rb.connection, 'one connection object each');
    assert.deepEqual(fleet.ids(), ['a', 'b']);

    const sa = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    const sb = await handshakePlain(rb.connection, 'ws://b:9899', 'tokB', 'm-b');
    assert.ok(ra.connection.isConnected() && rb.connection.isConnected(), 'both live');
    assert.equal(ra.machineId(), 'm-a');
    assert.equal(rb.machineId(), 'm-b');

    // An RPC answered on one socket resolves only on its own runtime.
    const pa = ra.api.listSessions();
    const pb = rb.api.listSessions();
    await settle();
    const idOf = (s: MockWebSocket) => s.texts().at(-1).id;
    sa.message({ id: idOf(sa), result: [{ name: 'on-a' }] });
    sb.message({ id: idOf(sb), result: [{ name: 'on-b' }] });
    assert.deepEqual((await pa).map((s: any) => s.name), ['on-a']);
    assert.deepEqual((await pb).map((s: any) => s.name), ['on-b']);

    // Nothing of either server's clock survives the fleet: phase ①'s liveness
    // timers are per connection, so two runtimes must leave none behind.
    fleet.dropAll();
    assert.equal(timers.live(), 0, 'no interval or timeout outlives the fleet');
  } finally {
    fleet.dropAll();
    timers.restore();
  }
});

test('one entry is one runtime, and one machine is one runtime', async () => {
  const { storage, a } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const first = fleet.include(a);
    assert.equal(fleet.include(a), first, 'asking twice cannot mint a second client');
    // A caller holding a stale copy of the entry — a list read before a merge,
    // a different object with the same identity — must not open a second one.
    assert.equal(fleet.include({ ...a }), first, 'a copy of the entry is the same server');
    await handshakePlain(first.connection, 'ws://a:9899', 'tokA', 'm-a');
    assert.equal(
      fleet.include(entry({ id: 'stale-id', address: 'ws://a-other:9899', machineId: 'm-a' })),
      first,
      'a second entry id for a machine that already has a runtime folds into it',
    );
    assert.deepEqual(fleet.ids(), ['a'], 'still one runtime');
  } finally {
    fleet.dropAll();
  }
});

test('dropping one server leaves the other working', async () => {
  const { storage, a, b } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const rb = fleet.include(b);
    const sa = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    const sb = await handshakePlain(rb.connection, 'ws://b:9899', 'tokB', 'm-b');

    fleet.drop('a');
    assert.equal(fleet.get('a'), undefined);
    assert.deepEqual(fleet.ids(), ['b']);
    assert.equal(sa.readyState, MockWebSocket.CLOSED, 'A s socket is gone');
    await assert.rejects(ra.api.listSessions(), /connection disposed/u, 'and A is terminal');

    // B never noticed.
    assert.ok(rb.connection.isConnected());
    const pb = rb.api.listSessions();
    await settle();
    sb.message({ id: sb.texts().at(-1).id, result: [{ name: 'still-b' }] });
    assert.deepEqual((await pb).map((s: any) => s.name), ['still-b']);
  } finally {
    fleet.dropAll();
  }
});

test('A going down and coming back does not touch B', async () => {
  const { storage, a, b } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const rb = fleet.include(b);
    const sa = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    const sb = await handshakePlain(rb.connection, 'ws://b:9899', 'tokB', 'm-b');

    // A's pending call must reject on A's own socket close, and B's must not.
    const lost = ra.api.listSessions();
    const kept = rb.api.listSessions();
    await settle();
    sa.readyState = MockWebSocket.CLOSED;
    sa.onclose?.({ code: 1006, reason: '', wasClean: false });
    await assert.rejects(lost, 'A s in-flight call fails with A');
    assert.equal(ra.connection.isConnected(), false);
    assert.equal(rb.connection.isConnected(), true, 'B stayed up');

    const back = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    assert.ok(ra.connection.isConnected(), 'A is back on a new socket');
    assert.notEqual(back, sa);
    // B's call, started before A fell over, still answers on B's socket.
    sb.message({ id: sb.texts().at(-1).id, result: [{ name: 'b-unmoved' }] });
    assert.deepEqual((await kept).map((s: any) => s.name), ['b-unmoved']);
  } finally {
    fleet.dropAll();
  }
});

test('dial records the machine that answered and adopts its hostname', async () => {
  // A DEFAULT name (still the URL host it was derived from) follows the
  // server's own hostname, board #310; a name the user typed does not.
  const def = entry({ id: 'd', name: 'n', address: 'ws://n:9899', token: 'tokN', machineId: 'm-d' });
  const typed = entry({ id: 't', name: 'my laptop', address: 'ws://t:9899', token: 'tokT', machineId: 'm-t', named: true });
  const storage = mem({ tmux_servers: JSON.stringify([def, typed]), tmux_server_current: 'd' });
  const fleet = createServerFleet({ storage });
  try {
    for (const [id, addr, machine, host] of [['d', 'ws://n:9899', 'm-d', 'new-host'], ['t', 'ws://t:9899', 'm-t', 'typed-host']] as const) {
      const rt = fleet.include(loadServers(storage).find((s) => s.id === id)!);
      const dialing = rt.dial();
      const socket = MockWebSocket.instances.at(-1)!;
      socket.readyState = MockWebSocket.OPEN;
      socket.message({ server_nonce: '00'.repeat(16) });
      socket.message({ result: { authenticated: true, machine_id: machine, hostname: host } });
      const result = await dialing;
      assert.equal(result.ok, true, `${id} authenticated at ${addr}`);
      assert.equal(result.ok && result.entry.id, id, 'published under its own id');
      assert.equal(rt.hostname(), host);
    }
    assert.equal(loadServers(storage).find((s) => s.id === 'd')?.name, 'new-host', 'a default name follows the server');
    assert.equal(loadServers(storage).find((s) => s.id === 't')?.name, 'my laptop', 'a typed name is never replaced');
    // Recording is not activating: CURRENT is untouched by a dial.
    assert.equal(storage.getItem('tmux_server_current'), 'd');
  } finally {
    fleet.dropAll();
  }
});

test('an address that now answers as another machine does not publish under the old entry', async () => {
  // The bug this rule exists for: a loopback tunnel or a reused LAN address
  // that used to reach A now reaches B. Nothing may route B's panes into A's
  // views, and A's saved entry must keep its own name, token and address.
  const { storage, a } = twoServers();
  const before = loadServers(storage);
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const dialing = ra.dial();
    const socket = MockWebSocket.instances.at(-1)!;
    socket.readyState = MockWebSocket.OPEN;
    socket.message({ server_nonce: '00'.repeat(16) });
    socket.message({ result: { authenticated: true, machine_id: 'm-b', hostname: 'beta-host' } });
    const result = await dialing;

    assert.equal(result.ok, false);
    assert.equal(!result.ok && result.reason, 'elsewhere');
    assert.equal(!result.ok && result.reason === 'elsewhere' && result.entry.id, 'b',
      'it reports the entry of the machine that DID answer');
    assert.equal(ra.connection.isConnected(), false, 'and holds no live link to it');
    const after = loadServers(storage);
    assert.deepEqual(after.find((s) => s.id === 'a'), before.find((s) => s.id === 'a'),
      'A s entry is untouched — name, token and address are still A s');
    assert.equal(after.find((s) => s.id === 'b')?.address, 'ws://a:9899',
      'B s entry learns the address that reached it');
  } finally {
    fleet.dropAll();
  }
});

test('an entry that does not know its machine is merged before it publishes', async () => {
  // A migrated address-history entry, or one recorded before the machine id
  // was learned. Once it authenticates as a machine that already has an entry,
  // that entry is the server and this id must not become a second one.
  const known = entry({ id: 'known', name: 'alpha', address: 'ws://a:9899', token: 'tokA', machineId: 'm-a' });
  const guess = entry({ id: 'guess', name: 'a-other', address: 'ws://a-other:9899', token: 'tokA' });
  const storage = mem({
    tmux_servers: JSON.stringify([known, guess]),
    tmux_server_current: 'known',
    tmux_machines: JSON.stringify({ 'm-a': ['ws://a:9899'] }),
  });
  const fleet = createServerFleet({ storage });
  try {
    const rg = fleet.include(guess);
    const dialing = rg.dial();
    const socket = MockWebSocket.instances.at(-1)!;
    socket.readyState = MockWebSocket.OPEN;
    socket.message({ server_nonce: '00'.repeat(16) });
    socket.message({ result: { authenticated: true, machine_id: 'm-a', hostname: 'alpha-host' } });
    const result = await dialing;

    assert.equal(result.ok, false);
    assert.equal(!result.ok && result.reason === 'elsewhere' && result.entry.id, 'known');
    assert.equal(rg.connection.isConnected(), false);
    assert.deepEqual(loadServers(storage).map((s) => s.id), ['known'], 'the guess was absorbed');
  } finally {
    fleet.dropAll();
  }
});

test('an entry whose machine is new keeps its own id and gets stamped', async () => {
  const fresh = entry({ id: 'fresh', name: 'ws://n:9899', address: 'ws://n:9899', token: 'tokN' });
  const storage = mem({ tmux_servers: JSON.stringify([fresh]), tmux_server_current: 'fresh' });
  const fleet = createServerFleet({ storage });
  try {
    const rt = fleet.include(fresh);
    const dialing = rt.dial();
    const socket = MockWebSocket.instances.at(-1)!;
    socket.readyState = MockWebSocket.OPEN;
    socket.message({ server_nonce: '00'.repeat(16) });
    socket.message({ result: { authenticated: true, machine_id: 'm-new', hostname: 'new-host' } });
    const result = await dialing;
    assert.equal(result.ok, true);
    assert.equal(result.ok && result.entry.id, 'fresh');
    assert.equal(loadServers(storage).find((s) => s.id === 'fresh')?.machineId, 'm-new');
  } finally {
    fleet.dropAll();
  }
});

test('a removed entry is not dialled from a stale address', async () => {
  const { storage, a } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    storage.setItem('tmux_servers', JSON.stringify(loadServers(storage).filter((s) => s.id !== 'a')));
    const sockets = MockWebSocket.instances.length;
    const result = await ra.dial();
    assert.equal(!result.ok && result.reason, 'gone');
    assert.equal(MockWebSocket.instances.length, sockets, 'no socket was opened');
    // The name is still available to whoever has to tear the runtime down.
    assert.equal(ra.entry().name, 'alpha');
  } finally {
    fleet.dropAll();
  }
});

test('caps are the answer of the server that was asked', async () => {
  const { storage, a, b } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const rb = fleet.include(b);
    const sa = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    const sb = await handshakePlain(rb.connection, 'ws://b:9899', 'tokB', 'm-b');
    assert.deepEqual(ra.caps(), { hub: null, backends: null }, 'unasked is not "no"');

    const probing = Promise.all([ra.probeCaps(), rb.probeCaps()]);
    await settle();
    const answer = (s: MockWebSocket, method: string, body: unknown) => {
      const req = s.texts().findLast((t: any) => t.method === method);
      assert.ok(req, `${method} was asked`);
      s.message({ id: req.id, ...(body as object) });
    };
    // A has the Hub and two backends; B is a plain tmux server.
    answer(sa, 'hub_rooms', { result: { rooms: {} } });
    answer(sa, 'backends_list', { result: { backends: [{ name: 'kiro' }, { name: 'codex' }] } });
    answer(sb, 'hub_rooms', { error: { code: -32601, message: 'no such method' } });
    answer(sb, 'backends_list', { error: { code: -32601, message: 'no such method' } });
    await probing;

    assert.equal(ra.caps().hub, true);
    assert.deepEqual(ra.caps().backends?.map((x) => x.name), ['kiro', 'codex']);
    assert.equal(rb.caps().hub, false, 'a definitive "no method" is a no');
    assert.equal(rb.caps().backends, null);
  } finally {
    fleet.dropAll();
  }
});

test('a transient failure leaves the Hub flag alone; an empty list is unknown', async () => {
  const { storage, a } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    const sa = await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    // findLast, not find: the second probe sends new requests, and answering
    // the first probe's stale id would leave these pending until the RPC
    // timeout — the test would then pass through the timeout's catch branch
    // and prove nothing about either rule.
    const answer = (method: string, body: unknown) => {
      const req = sa.texts().findLast((t: any) => t.method === method);
      assert.ok(req, `${method} was asked`);
      sa.message({ id: req.id, ...(body as object) });
    };
    const first = ra.probeCaps();
    await settle();
    answer('hub_rooms', { result: { rooms: {} } });
    answer('backends_list', { result: { backends: [{ name: 'kiro' }] } });
    await first;
    assert.equal(ra.caps().hub, true);
    assert.deepEqual(ra.caps().backends?.map((x) => x.name), ['kiro']);

    // A reconnect blip must not unmount the Hub and destroy its state.
    const again = ra.probeCaps();
    await settle();
    answer('hub_rooms', { error: { code: -32000, message: 'timeout' } });
    // A server that answers with an EMPTY list has told us nothing useful —
    // `agents.ts` treats that as unknown, and so does this.
    answer('backends_list', { result: { backends: [] } });
    await again;
    assert.equal(ra.caps().hub, true, 'only a definitive no flips it off');
    assert.equal(ra.caps().backends, null, 'an empty list is unknown, not "this server has none"');
  } finally {
    fleet.dropAll();
  }
});

test('a probe in flight when the runtime is dropped settles without publishing', async () => {
  const { storage, a } = twoServers();
  const fleet = createServerFleet({ storage });
  try {
    const ra = fleet.include(a);
    await handshakePlain(ra.connection, 'ws://a:9899', 'tokA', 'm-a');
    const probing = ra.probeCaps();
    await settle();
    fleet.drop('a');
    const caps = await probing;
    assert.deepEqual(caps, { hub: null, backends: null }, 'a dropped runtime learns nothing');
    assert.deepEqual(ra.caps(), { hub: null, backends: null });
  } finally {
    fleet.dropAll();
  }
});

test('an answer that RESOLVES after the drop is still not published', async () => {
  // The race the `dead` guard exists for, with the timing made exact: the RPC
  // resolves successfully, and only then does the runtime go away. Through a
  // socket the drop rejects the pending call instead, so that path can only
  // ever exercise the catch branch — this one drives the success branch.
  const { storage, a } = twoServers();
  let answerHub: (v: unknown) => void = () => {};
  let answerBackends: (v: unknown) => void = () => {};
  let disposed = 0;
  const slot = {
    connection: {
      dispose: () => { disposed++; },
      getMachineId: () => 'm-a',
      getHostname: () => 'alpha-host',
    },
    api: {
      hubRooms: () => new Promise((resolve) => { answerHub = resolve; }),
      backendsList: () => new Promise((resolve) => { answerBackends = resolve; }),
    },
  } as unknown as ConnectionSlot;
  const runtime = createServerRuntime(a, { storage, slot });

  const probing = runtime.probeCaps();
  runtime.dispose();
  assert.equal(disposed, 1, 'the connection is released');
  answerHub({ rooms: {} });
  answerBackends({ backends: [{ name: 'kiro' }] });
  assert.deepEqual(await probing, { hub: null, backends: null },
    'both answers are dropped, not written onto a runtime nobody holds');
  // Terminal, like Connection.dispose: a second drop is a no-op.
  runtime.dispose();
  assert.equal(disposed, 2, 'and it stays idempotent at the connection');
});

test('the registry is the lifetime, the fleet is the membership', async () => {
  const { storage, a, b } = twoServers();
  const registry = createConnectionRegistry();
  const fleet = createServerFleet({ storage, registry });
  try {
    const ra = fleet.include(a);
    fleet.include(b);
    assert.equal(registry.get('a')?.connection, ra.connection, 'the runtime uses the registry s slot');
    fleet.drop('a');
    assert.equal(registry.get('a'), undefined, 'and dropping it clears both maps');
    assert.ok(registry.get('b'), 'B is still registered');
    fleet.dropAll();
    assert.equal(registry.get('b'), undefined);
    assert.deepEqual(fleet.ids(), []);
  } finally {
    fleet.dropAll();
  }
});

test('a frame sealed for one server resolves nothing on the other', async () => {
  // End to end with real Web Crypto: the two runtimes negotiate separate
  // session keys, so a frame sealed by A is undecryptable by B. What must NOT
  // happen is B resolving A's call — and by the baseline's own rule a frame
  // that fails to decrypt costs that socket its link, because the receive
  // counter has already moved past it. So the cross-delivery costs B its
  // socket and leaves A's call still waiting for A.
  await withWebCrypto(async () => {
    const { storage, a, b } = twoServers();
    const fleet = createServerFleet({ storage });
    const srvA = new FakeE2eServer('tokA', 2, 'm-a', 'alpha-host');
    const srvB = new FakeE2eServer('tokB', 2, 'm-b', 'beta-host');
    try {
      const ra = fleet.include(a);
      const rb = fleet.include(b);
      const sa = await handshake(ra.connection, srvA, 'ws://a:9899', 'tokA');
      const sb = await handshake(rb.connection, srvB, 'ws://b:9899', 'tokB');
      assert.equal(ra.machineId(), 'm-a');
      assert.equal(rb.machineId(), 'm-b');

      const pa = ra.api.listSessions();
      let settled = false;
      void pa.then(() => { settled = true; }, () => { settled = true; });
      const reqA = await srvA.next(sa);
      const forA = await srvA.seal(JSON.stringify({ id: reqA.id, result: [{ name: 'a-only' }] }));

      sb.binary(forA);
      await until(() => !rb.connection.isConnected());
      assert.equal(settled, false, 'B could not answer a call it never carried');
      assert.equal(ra.connection.isConnected(), true, 'and A is untouched by B s loss');

      sa.binary(forA);
      assert.deepEqual((await pa).map((s: any) => s.name), ['a-only'], 'A s own socket answers it');
    } finally {
      fleet.dropAll();
    }
  });
});
