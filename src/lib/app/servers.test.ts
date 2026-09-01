import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  CURRENT_KEY, MACHINE_PREFIX, MAX_SERVERS, SERVERS_KEY, STATE_PREFIX,
  applySwitch, currentServerId, hostLabel, loadServers, migrateServers,
  removeServer, renameServer, upsertServer,
} from './servers.ts';

function mem(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => { m.set(k, String(v)); },
    removeItem: (k: string) => { m.delete(k); },
    dump: () => Object.fromEntries(m),
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
  assert.equal(servers[1]!.token, 'tokB');
  assert.equal(servers[2]!.address, 'ws://legacy-string:9899');
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
  assert.equal(servers[1]!.token, '');
  const many = Array.from({ length: 30 }, (_, i) => ({ id: `s${i}`, name: `${i}`, address: `ws://h${i}:1`, token: '' }));
  s.setItem(SERVERS_KEY, JSON.stringify(many));
  assert.equal(loadServers(s).length, MAX_SERVERS);
  s.setItem(SERVERS_KEY, '{not json');
  assert.deepEqual(loadServers(s), []);
});

test('upsert by address updates token in place and keeps the user’s rename', () => {
  const s = mem();
  migrateServers(s);
  upsertServer(s, { address: 'ws://a:1', token: 'old' });
  renameServer(s, loadServers(s)[0]!.id, 'my mac');
  const servers = upsertServer(s, { address: 'ws://a:1', token: 'new', socket: '/s' });
  assert.equal(servers.length, 1, 'same address never duplicates');
  assert.equal(servers[0]!.token, 'new');
  assert.equal(servers[0]!.socket, '/s');
  assert.equal(servers[0]!.name, 'my mac', 'rename survives reconnects');
  const two = upsertServer(s, { address: 'ws://b:2', token: 'tb' });
  assert.equal(two.length, 2);
  assert.equal(currentServerId(s), two[1]!.id, 'a fresh connect makes its server current');
});

test('removeServer refuses the current entry and clears parked state for others', () => {
  const s = mem();
  upsertServer(s, { address: 'ws://a:1', token: '' });   // current after upsert
  const a = loadServers(s)[0]!;
  upsertServer(s, { address: 'ws://b:2', token: '' });   // b is now current
  const b = loadServers(s)[1]!;
  s.setItem(STATE_PREFIX + a.id, '{"page":"hub"}');
  s.setItem(MACHINE_PREFIX + a.id, 'mid-a');
  assert.equal(removeServer(s, b.id).length, 2, 'current is not removable');
  const left = removeServer(s, a.id);
  assert.equal(left.length, 1);
  assert.equal(s.getItem(STATE_PREFIX + a.id), null, 'parked state goes with the entry');
  assert.equal(s.getItem(MACHINE_PREFIX + a.id), null);
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
  upsertServer(s, { address: 'ws://b:2', token: 'tb' }); // b appended (and made current — reset)
  const b = loadServers(s)[1]!;
  s.setItem(CURRENT_KEY, a.id);
  s.setItem(STATE_PREFIX + b.id, '{"page":"board"}');
  s.setItem(MACHINE_PREFIX + b.id, 'mid-b');

  assert.equal(applySwitch(s, 'nope'), false, 'unknown target is refused');
  assert.equal(applySwitch(s, a.id), false, 'already current is refused');
  assert.equal(applySwitch(s, b.id), true);

  // The leaving server's live pair is parked under ITS id…
  assert.equal(s.getItem(STATE_PREFIX + a.id), '{"page":"terminal","terminalTarget":"projA:1.1"}');
  assert.equal(s.getItem(MACHINE_PREFIX + a.id), 'mid-a');
  // …the target's parked pair becomes live (restore targets cannot cross servers)…
  assert.equal(s.getItem('tmux_state'), '{"page":"board"}');
  assert.equal(s.getItem('tmux_machine_id'), 'mid-b');
  // …the mirror keys now point at b, b is current, and the boot path will connect.
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
  upsertServer(s, { address: 'ws://b:2', token: 'tb' });
  const b = loadServers(s)[1]!;
  s.setItem(CURRENT_KEY, a.id);
  assert.equal(applySwitch(s, b.id), true);
  assert.equal(s.getItem('tmux_state'), null, 'no parked state — B starts fresh, never on A’s targets');
  assert.equal(s.getItem('tmux_machine_id'), null, 'B’s machine id arrives on connect, never A’s');
  // And switching BACK restores A's parked pair intact.
  assert.equal(applySwitch(s, a.id), true);
  assert.equal(s.getItem('tmux_state'), '{"page":"hub"}');
  assert.equal(s.getItem('tmux_machine_id'), 'mid-a');
  assert.equal(s.getItem('tmux_address'), 'ws://a:1');
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

test('upsert merges by machine identity across addresses; a pre-connect entry gets stamped', () => {
  const s = mem();
  // Hand-typed LAN connect, machineId learned after auth:
  upsertServer(s, { address: 'ws://192.168.1.5:9899', token: 't', machineId: 'm1' });
  // Later the SAME machine is reached over Tailscale — must update in place:
  const servers = upsertServer(s, { address: 'wss://mac.tail.ts.net/ws', token: 't2', machineId: 'm1' });
  assert.equal(servers.length, 1, 'machineId match merges, address difference notwithstanding');
  assert.equal(servers[0]!.address, 'wss://mac.tail.ts.net/ws', 'active address follows the connect');
  assert.equal(servers[0]!.token, 't2');
  // A deep link (no machineId yet) matching by address stamps nothing away:
  upsertServer(s, { address: 'ws://new:1', token: 'x' });               // pre-connect entry
  const stamped = upsertServer(s, { address: 'ws://new:1', token: 'x', machineId: 'm2' });
  assert.equal(stamped.length, 2);
  assert.equal(stamped[1]!.machineId, 'm2', 'address match stamps the learned machineId');
});

test('applySwitch prefers the entry’s own machineId for the failover set', () => {
  const s = mem({ tmux_address: 'ws://a:1', tmux_token: 'ta', tmux_machine_id: 'm-a' });
  migrateServers(s);
  const a = loadServers(s)[0]!;
  upsertServer(s, { address: 'ws://b:2', token: 'tb', machineId: 'm-b' });
  const b = loadServers(s)[1]!;
  s.setItem(CURRENT_KEY, a.id);
  assert.equal(applySwitch(s, b.id), true);
  assert.equal(s.getItem('tmux_machine_id'), 'm-b', 'B’s identity, even with no parked key');
  assert.equal(applySwitch(s, a.id), true);
  assert.equal(s.getItem('tmux_machine_id'), 'm-a', 'and back — never crossed');
});
