import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  CURRENT_KEY, MACHINE_PREFIX, MAX_SERVERS, SERVERS_KEY, STATE_PREFIX,
  activateConnected, applySwitch, currentServerId, hostLabel, loadServers,
  migrateServers, recordServer, removeServer, renameServer,
} from './servers.ts';

function mem(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => { m.set(k, String(v)); },
    removeItem: (k: string) => { m.delete(k); },
  };
}

test('hostLabel names a server by its host, keeping typed fallbacks', () => {
  assert.equal(hostLabel('ws://192.168.1.5:9899'), '192.168.1.5');
  assert.equal(hostLabel('wss://mac.tail.ts.net/ws'), 'mac.tail.ts.net');
  assert.equal(hostLabel('  wss://host:443  '), 'host');
  assert.equal(hostLabel('not-an-address'), 'not-an-address');
});

test('migration: the current connection becomes the first, CURRENT entry; history follows', () => {
  const s = mem({
    tmux_address: 'ws://10.0.0.2:9899',
    tmux_token: 'tokA',
    tmux_socket: '/tmp/sockA',
    tmux_address_history: JSON.stringify([
      { address: 'ws://10.0.0.2:9899', token: 'stale' },   // covered by current — skipped
      { address: 'wss://other.example/ws', token: 'tokB' },
      'ws://legacy-string:9899',                            // old string format
    ]),
  });
  const servers = migrateServers(s);
  assert.equal(servers.length, 3);
  assert.equal(servers[0]!.address, 'ws://10.0.0.2:9899');
  assert.equal(servers[0]!.token, 'tokA');
  assert.equal(servers[0]!.socket, '/tmp/sockA');
  assert.equal(servers[0]!.name, '10.0.0.2');
  assert.equal(currentServerId(s), servers[0]!.id, 'the current user is not lost');
  assert.equal(servers[1]!.address, 'wss://other.example/ws');
  assert.equal(servers[2]!.token, '');
  // The mirror keys are untouched: the boot path still reads them.
  assert.equal(s.getItem('tmux_address'), 'ws://10.0.0.2:9899');
});

test('migration is idempotent — an existing registry is returned, never rebuilt', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: 't' });
  const first = migrateServers(s);
  s.setItem('tmux_address', 'ws://changed:2'); // later drift must not re-run it
  const second = migrateServers(s);
  assert.deepEqual(second, first);
});

test('migration with no stored connection yields an empty registry and no current', () => {
  const s = mem();
  assert.deepEqual(migrateServers(s), []);
  assert.equal(currentServerId(s), '');
  assert.equal(s.getItem(SERVERS_KEY), '[]', 'the marker exists so it never re-runs');
});

test('loadServers drops malformed rows, duplicate ids, and respects the cap', () => {
  const good = { id: 'a', name: 'A', address: 'ws://a:1', token: 't' };
  const s = mem({
    [SERVERS_KEY]: JSON.stringify([
      good, { id: 'a', name: 'dup', address: 'ws://dup:1', token: '' },
      { id: 'b', address: 'ws://b:1' },            // name/token missing → defaulted
      { id: 'c' }, 'junk', null,                   // dropped
    ]),
  });
  const servers = loadServers(s);
  assert.equal(servers.length, 2);
  assert.equal(servers[1]!.name, 'b', 'name defaults to the host label');
  const many = Array.from({ length: 30 }, (_, i) => ({ id: `s${i}`, name: `${i}`, address: `ws://h${i}:1`, token: '' }));
  s.setItem(SERVERS_KEY, JSON.stringify(many));
  assert.equal(loadServers(s).length, MAX_SERVERS);
  s.setItem(SERVERS_KEY, '{not json');
  assert.deepEqual(loadServers(s), []);
});

test('same machine, three addresses = ONE config (lead review, board #55)', () => {
  // tmux_machines is the identity authority: LAN/Tailscale/WAN alternates of
  // the CURRENT machine must not re-materialize from history as separate
  // "servers" — they are the failover set the switch hands back to.
  const s = mem({
    tmux_address: 'ws://192.168.1.5:9899',
    tmux_token: 'tok',
    tmux_machine_id: 'm1',
    tmux_machines: JSON.stringify({ m1: ['ws://192.168.1.5:9899', 'wss://mac.tail.ts.net/ws', 'wss://wan.example:443'] }),
    tmux_address_history: JSON.stringify([
      { address: 'wss://mac.tail.ts.net/ws', token: 'tok' },
      { address: 'wss://wan.example:443', token: 'tok' },
      { address: 'ws://192.168.1.5:9899', token: 'tok' },
    ]),
  });
  const servers = migrateServers(s);
  assert.equal(servers.length, 1, 'the machine is one entry, not three');
  assert.equal(servers[0]!.machineId, 'm1');
  assert.equal(servers[0]!.address, 'ws://192.168.1.5:9899', 'the live address is the active one');
});

test('different machines never merge, however alike their addresses look', () => {
  const s = mem({
    tmux_address: 'ws://10.0.0.2:9899',
    tmux_token: 'ta',
    tmux_machine_id: 'm1',
    tmux_machines: JSON.stringify({ m1: ['ws://10.0.0.2:9899'], m2: ['ws://10.0.0.2:9900'] }),
    tmux_address_history: JSON.stringify([{ address: 'ws://10.0.0.2:9900', token: 'tb' }]),
  });
  const servers = migrateServers(s);
  assert.equal(servers.length, 2, 'same host shape, different machineId — separate entries');
  assert.equal(servers[0]!.machineId, 'm1');
  assert.equal(servers[1]!.machineId, 'm2');
  // hostLabel collides ('10.0.0.2' both) — identity is the machine, not the label.
  assert.equal(servers[0]!.name, servers[1]!.name);
});

test('a history machine seen for the first time yields ONE entry across its addresses', () => {
  const s = mem({
    tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_machine_id: 'm1',
    tmux_machines: JSON.stringify({ m1: ['ws://a:1'], m2: ['ws://b-lan:2', 'wss://b-wan:443'] }),
    tmux_address_history: JSON.stringify([
      { address: 'ws://b-lan:2', token: 'tb' },
      { address: 'wss://b-wan:443', token: 'tb' },
    ]),
  });
  const servers = migrateServers(s);
  assert.equal(servers.length, 2);
  assert.equal(servers[1]!.machineId, 'm2');
  assert.equal(servers[1]!.address, 'ws://b-lan:2', 'first-seen address is the active one');
});

test('recordServer NEVER moves CURRENT — recording and activating are different acts', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: 'ta' });
  migrateServers(s);
  const aId = currentServerId(s);
  const { entry } = recordServer(s, { address: 'ws://b:2', token: 'tb', machineId: 'm-b' });
  assert.equal(loadServers(s).length, 2, 'the new server is recorded');
  assert.notEqual(entry.id, aId);
  assert.equal(currentServerId(s), aId, 'CURRENT did not move (lead blocker #2)');
});

test('recordServer merges by machine identity across addresses and keeps renames', () => {
  const s = mem();
  migrateServers(s);
  recordServer(s, { address: 'ws://192.168.1.5:9899', token: 't', machineId: 'm1' });
  renameServer(s, loadServers(s)[0]!.id, 'my mac');
  const { servers } = recordServer(s, { address: 'wss://mac.tail.ts.net/ws', token: 't2', machineId: 'm1' });
  assert.equal(servers.length, 1, 'machineId match merges, address difference notwithstanding');
  assert.equal(servers[0]!.address, 'wss://mac.tail.ts.net/ws', 'active address follows the connect');
  assert.equal(servers[0]!.token, 't2');
  assert.equal(servers[0]!.name, 'my mac', 'rename survives reconnects');
  // A pre-connect entry (no machineId yet) gets stamped by an address match:
  recordServer(s, { address: 'ws://new:1', token: 'x' });
  const { servers: stamped } = recordServer(s, { address: 'ws://new:1', token: 'x', machineId: 'm2' });
  assert.equal(stamped.length, 2);
  assert.equal(stamped[1]!.machineId, 'm2', 'address match stamps the learned machineId');
});

test('recordServer attributes an address through tmux_machines when no machineId is given', () => {
  // A deep link carries no machineId; the failover map still knows whose
  // address it is — an alternate of a known machine folds into that entry.
  const s = mem({
    tmux_machines: JSON.stringify({ 'm-a': ['ws://a-lan:1', 'wss://a-wan:443'] }),
  });
  recordServer(s, { address: 'ws://a-lan:1', token: 't' });
  const { servers, entry } = recordServer(s, { address: 'wss://a-wan:443', token: 't' });
  assert.equal(servers.length, 1, 'the map attribution merged the alternate');
  assert.equal(entry.machineId, 'm-a', 'and stamped the identity');
});

test('activateConnected: the SAME server (machine alternate) needs no reboot', () => {
  const s = mem({
    tmux_address: 'ws://a-lan:1', tmux_token: 'ta', tmux_machine_id: 'm-a',
    tmux_machines: JSON.stringify({ 'm-a': ['ws://a-lan:1', 'wss://a-wan:443'] }),
    tmux_state: '{"page":"hub"}',
  });
  migrateServers(s);
  const aId = currentServerId(s);
  const act = activateConnected(s, { address: 'wss://a-wan:443', token: 'ta', machineId: 'm-a' });
  assert.equal(act.reload, false, 'same machine — the socket swap was the whole event');
  assert.equal(currentServerId(s), aId, 'CURRENT unchanged');
  assert.equal(s.getItem('tmux_state'), '{"page":"hub"}', 'live state untouched');
  assert.equal(loadServers(s).length, 1, 'still one entry');
});

test('activateConnected: a DIFFERENT server parks the old live state and demands a reboot', () => {
  // The lead blocker: a Settings connect to a new machine bypassed
  // applySwitch — old tmux_state was never parked under the old id and the
  // in-memory caches (Hub rooms, terminals, Files cwds) stayed old-server.
  // activateConnected does the applySwitch bookkeeping and returns
  // reload:true so the caller reboots through the one boot path.
  const s = mem({
    tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_machine_id: 'm-a',
    tmux_state: '{"page":"board"}',
  });
  migrateServers(s);
  const aId = currentServerId(s);
  s.setItem('tmux_address', 'ws://b:2');       // the connect form wrote the mirror keys pre-dial
  s.setItem('tmux_token', 'tb');
  s.setItem('tmux_disconnected', '1');
  const act = activateConnected(s, { address: 'ws://b:2', token: 'tb', machineId: 'm-b' });
  assert.equal(act.reload, true, 'different machine — the caller must location.reload()');
  assert.equal(s.getItem(STATE_PREFIX + aId), '{"page":"board"}', 'A’s live state parked under A');
  assert.equal(s.getItem(MACHINE_PREFIX + aId), 'm-a', 'A’s machine id parked too');
  assert.equal(s.getItem('tmux_state'), null, 'B starts fresh — never on A’s restore targets');
  assert.equal(s.getItem('tmux_machine_id'), 'm-b', 'B’s identity is live');
  const bEntry = loadServers(s).find((x) => x.address === 'ws://b:2')!;
  assert.equal(currentServerId(s), bEntry.id, 'B is current');
  assert.equal(s.getItem('tmux_disconnected'), null, 'connecting IS the intent');
  // And a later switch back restores A intact.
  assert.equal(applySwitch(s, aId), true);
  assert.equal(s.getItem('tmux_state'), '{"page":"board"}');
  assert.equal(s.getItem('tmux_machine_id'), 'm-a');
});

test('removeServer refuses the current entry and clears parked state for others', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: '' });
  migrateServers(s);
  const a = loadServers(s)[0]!;
  const { entry: b } = recordServer(s, { address: 'ws://b:2', token: '' });
  s.setItem(STATE_PREFIX + b.id, '{"page":"hub"}');
  s.setItem(MACHINE_PREFIX + b.id, 'mid-b');
  assert.equal(removeServer(s, a.id).length, 2, 'current is not removable');
  const left = removeServer(s, b.id);
  assert.equal(left.length, 1);
  assert.equal(s.getItem(STATE_PREFIX + b.id), null, 'parked state goes with the entry');
  assert.equal(s.getItem(MACHINE_PREFIX + b.id), null);
});

test('applySwitch parks the leaving server’s state and restores the target’s', () => {
  const s = mem({
    tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_socket: '/sa',
    tmux_state: '{"page":"terminal","terminalTarget":"projA:1.1"}',
    tmux_machine_id: 'mid-a',
    tmux_disconnected: '1',
  });
  migrateServers(s);                                     // a = current
  const a = loadServers(s)[0]!;
  const { entry: b } = recordServer(s, { address: 'ws://b:2', token: 'tb' });
  s.setItem(STATE_PREFIX + b.id, '{"page":"board"}');
  s.setItem(MACHINE_PREFIX + b.id, 'mid-b');

  assert.equal(applySwitch(s, 'nope'), false, 'unknown target is refused');
  assert.equal(applySwitch(s, a.id), false, 'already current is refused');
  assert.equal(applySwitch(s, b.id), true);

  assert.equal(s.getItem(STATE_PREFIX + a.id), '{"page":"terminal","terminalTarget":"projA:1.1"}');
  assert.equal(s.getItem(MACHINE_PREFIX + a.id), 'mid-a');
  assert.equal(s.getItem('tmux_state'), '{"page":"board"}');
  assert.equal(s.getItem('tmux_machine_id'), 'mid-b');
  assert.equal(s.getItem('tmux_address'), 'ws://b:2');
  assert.equal(s.getItem('tmux_token'), 'tb');
  assert.equal(s.getItem('tmux_socket'), null, 'no socket on b — the key is cleared, not inherited');
  assert.equal(currentServerId(s), b.id);
  assert.equal(s.getItem('tmux_disconnected'), null, 'switching is the intent to connect');
});

test('applySwitch to a first-visit target clears the live pair instead of inheriting', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_state: '{"page":"hub"}', tmux_machine_id: 'mid-a' });
  migrateServers(s);
  const a = loadServers(s)[0]!;
  const { entry: b } = recordServer(s, { address: 'ws://b:2', token: 'tb' });
  assert.equal(applySwitch(s, b.id), true);
  assert.equal(s.getItem('tmux_state'), null, 'no parked state — B starts fresh, never on A’s targets');
  assert.equal(s.getItem('tmux_machine_id'), null, 'B’s machine id arrives on connect, never A’s');
  assert.equal(applySwitch(s, a.id), true);
  assert.equal(s.getItem('tmux_state'), '{"page":"hub"}');
  assert.equal(s.getItem('tmux_machine_id'), 'mid-a');
  assert.equal(s.getItem('tmux_address'), 'ws://a:1');
});

test('applySwitch prefers the entry’s own machineId for the failover set', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_machine_id: 'm-a' });
  migrateServers(s);
  const a = loadServers(s)[0]!;
  const { entry: b } = recordServer(s, { address: 'ws://b:2', token: 'tb', machineId: 'm-b' });
  assert.equal(applySwitch(s, b.id), true);
  assert.equal(s.getItem('tmux_machine_id'), 'm-b', 'B’s identity, even with no parked key');
  assert.equal(applySwitch(s, a.id), true);
  assert.equal(s.getItem('tmux_machine_id'), 'm-a', 'and back — never crossed');
});

test('activateConnected owns the live machine id — same machine refreshes it (lead blocker #2)', () => {
  const s = mem({
    tmux_address: 'ws://a-lan:1', tmux_token: 'ta',
    tmux_machines: JSON.stringify({ 'm-a': ['ws://a-lan:1', 'ws://a-alt:2'] }),
    // live key stale/absent — Settings no longer pre-writes it
  });
  migrateServers(s);
  const act = activateConnected(s, { address: 'ws://a-alt:2', token: 'ta', machineId: 'm-a' });
  assert.equal(act.reload, false);
  assert.equal(s.getItem('tmux_machine_id'), 'm-a', 'the same-machine branch refreshes the live key');
});

test('the REAL caller sequence cannot poison the old server’s parked machine id', () => {
  // Regression for the exact bug: Settings used to write the NEW machine's
  // id into the live key BEFORE activating, so parkAndPoint filed m-b under
  // A's slot. The contract is now: the caller never pre-writes the live key,
  // activateConnected parks whatever the OLD server left there.
  const s = mem({
    tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_machine_id: 'm-a',
    tmux_state: '{"page":"hub"}',
  });
  migrateServers(s);
  const aId = currentServerId(s);
  // The real caller: mirror keys pre-written, map updated, live key UNTOUCHED.
  s.setItem('tmux_address', 'ws://b:2');
  s.setItem('tmux_token', 'tb');
  s.setItem('tmux_machines', JSON.stringify({ 'm-a': ['ws://a:1'], 'm-b': ['ws://b:2'] }));
  const act = activateConnected(s, { address: 'ws://b:2', token: 'tb', machineId: 'm-b' });
  assert.equal(act.reload, true);
  assert.equal(s.getItem(MACHINE_PREFIX + aId), 'm-a', 'A’s slot holds A’s id — never B’s');
  assert.equal(s.getItem('tmux_machine_id'), 'm-b', 'the live key is B’s, written by parkAndPoint');
});
