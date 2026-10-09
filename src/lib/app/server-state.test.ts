// One slot per server, and the one-time fold that gets there (board #335
// ②a-3). Neither is wired into production: ②b switches the live reads and
// writes over and calls the migration once, before any page mounts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServerStore, isResidentKey } from './server-store.ts';
import { RESIDENT_KEYS, migrateServerState, residentKey } from './server-state-migration.ts';
import { PARKED_KEYS, parkFrom, pointTo } from './servers.ts';

function mem(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => { m.set(k, String(v)); },
    removeItem: (k: string) => { m.delete(k); },
    dump: () => Object.fromEntries([...m.entries()].sort()),
  };
}

/** A client as it looked before #335: one live set of keys for the server it
 * is connected to, and a park for the server it last left. */
function preMigration() {
  return mem({
    tmux_servers: JSON.stringify([
      { id: 'a', name: 'alpha', address: 'ws://a:9899', token: 'tokA', machineId: 'm-a' },
      { id: 'b', name: 'beta', address: 'ws://b:9899', token: 'tokB', machineId: 'm-b' },
    ]),
    tmux_server_current: 'a',
    // A's live state: the user is on A right now.
    tmux_state: JSON.stringify({ page: 'hub', terminalTarget: 'work:1.0', terminalSession: 'work' }),
    tmux_machine_id: 'm-a',
    tmux_hub_project: 'work',
    tmux_hub_drafts: JSON.stringify({ work: 'half a line on A' }),
    tmux_hub_seen: JSON.stringify({ work: { seq: 12, ts: 1000 } }),
    tmux_hub_lead: JSON.stringify({ work: 'builder' }),
    tmux_hub_drawer: JSON.stringify({ work: 'files' }),
    tmux_hub_roster_expanded: JSON.stringify({ work: true }),
    tmux_hub_alerts: JSON.stringify([{ id: 'n1', room: 'proj:work' }]),
    // B's park, from the last time the user left it. Same project NAME, which
    // is the collision the whole phase exists for.
    'tmux_state::b': JSON.stringify({ page: 'terminal', terminalTarget: 'work:2.1', terminalSession: 'work' }),
    'tmux_machine_id::b': 'm-b',
    'tmux_hub_drafts::b': JSON.stringify({ work: 'half a line on B' }),
    'tmux_hub_seen::b': JSON.stringify({ work: { seq: 3, ts: 500 } }),
    // A stale park for A itself, from before this visit: older than the live
    // value by construction, because the live value is what replaced it.
    'tmux_hub_drafts::a': JSON.stringify({ work: 'what A looked like last time' }),
    // A global preference, which is the PERSON's and not any server's.
    tmux_hub_feed_level: 'status',
    tmux_theme: 'dark',
  });
}

test('the resident set is exactly the parked set, in both directions', () => {
  // Two lists would let a key be added to one and forgotten in the other, and
  // the symptom is a draft or a read mark that vanishes on a switch.
  // `servers.test.ts` already pins PARKED_KEYS against hub-prefs'
  // HUB_SERVER_KEYS by reading the source, because hub-prefs touches
  // localStorage at module load; this chains onto that pin rather than
  // repeating it.
  assert.deepEqual([...RESIDENT_KEYS], [...PARKED_KEYS]);
  for (const key of [
    'tmux_state', 'tmux_machine_id', 'tmux_hub_project', 'tmux_hub_drafts', 'tmux_hub_seen',
    'tmux_hub_lead', 'tmux_hub_drawer', 'tmux_hub_roster_expanded', 'tmux_hub_alerts',
  ]) {
    assert.ok(isResidentKey(key), `${key} is one server's state`);
  }
  // The person's and the window's preferences are not.
  for (const key of [
    'tmux_hub_feed_level', 'tmux_hub_sidebar', 'tmux_hub_steps_rows', 'tmux_theme', 'tmux_font_size',
    'tmux_servers', 'tmux_server_current', 'tmux_machines', 'tmux_address', 'tmux_token', 'tmux_socket',
  ]) {
    assert.equal(isResidentKey(key), false, `${key} must not become per-server`);
  }
});

test('a server store reads and writes its own slot, and nobody else s', () => {
  const storage = preMigration();
  const a = createServerStore(storage, 'a');
  const b = createServerStore(storage, 'b');
  assert.equal(a.keyFor('tmux_hub_drafts'), 'tmux_hub_drafts::a');
  assert.equal(b.keyFor('tmux_hub_drafts'), 'tmux_hub_drafts::b');

  assert.deepEqual(JSON.parse(b.getItem('tmux_hub_drafts')!), { work: 'half a line on B' });
  b.setItem('tmux_hub_drafts', JSON.stringify({ work: 'typed on B' }));
  assert.deepEqual(JSON.parse(a.getItem('tmux_hub_drafts')!), { work: 'what A looked like last time' },
    'A s slot is untouched by a write to B');
  assert.equal(storage.getItem('tmux_hub_drafts'), JSON.stringify({ work: 'half a line on A' }),
    'and so is the unprefixed key, which is no longer a location');

  a.removeItem('tmux_hub_drafts');
  assert.equal(a.getItem('tmux_hub_drafts'), null);
  assert.ok(b.getItem('tmux_hub_drafts'), 'removing A s row leaves B s');
});

test('a global preference is the person s, whichever server is asked', () => {
  const storage = preMigration();
  const a = createServerStore(storage, 'a');
  const b = createServerStore(storage, 'b');
  assert.equal(a.getItem('tmux_hub_feed_level'), 'status');
  assert.equal(b.getItem('tmux_hub_feed_level'), 'status', 'the same human gets the same app');
  b.setItem('tmux_theme', 'light');
  assert.equal(a.getItem('tmux_theme'), 'light');
  assert.equal(storage.getItem('tmux_theme'), 'light', 'written where every server reads it');
  assert.equal(storage.getItem('tmux_theme::b'), null, 'and not scoped behind one');
});

test('a store without a server is refused, not quietly unscoped', () => {
  // The dangerous case is the path where CURRENT has not resolved yet: a view
  // that fell back to the unprefixed keys would write one server's state into
  // the place every other server reads, and only there.
  assert.throws(() => createServerStore(mem(), ''), /serverId/u);
});

test('the fold files the live values under the CURRENT server', () => {
  const storage = preMigration();
  const report = migrateServerState(storage);
  assert.equal(report.ran, true);
  assert.equal(report.serverId, 'a');
  for (const key of RESIDENT_KEYS) {
    const live = storage.getItem(key);
    if (live == null) continue;
    assert.equal(storage.getItem(residentKey(key, 'a')), live, `${key} is now resident under A`);
  }
  // The active value wins over A's own stale park: it is what the user was
  // looking at a moment ago, and the park is from the last time they left.
  assert.deepEqual(
    JSON.parse(storage.getItem('tmux_hub_drafts::a')!),
    { work: 'half a line on A' },
  );
});

test('the fold keeps every other server s park', () => {
  const storage = preMigration();
  const before = storage.dump();
  migrateServerState(storage);
  const after = storage.dump();
  for (const [key, value] of Object.entries(before)) {
    if (key.endsWith('::b')) {
      assert.equal(after[key], value, `${key} is already in its final home`);
    }
  }
  // B's draft for the same project name is still B's.
  assert.deepEqual(JSON.parse(after['tmux_hub_drafts::b']!), { work: 'half a line on B' });
});

test('the fold leaves the unprefixed keys readable for a rollback', () => {
  const storage = preMigration();
  const before = storage.dump();
  migrateServerState(storage);
  for (const key of RESIDENT_KEYS) {
    assert.equal(storage.getItem(key), before[key] ?? null,
      `${key} still reads as it did, so a build from before #335 finds its world`);
  }
});

test('running the fold twice is running it once', () => {
  const storage = preMigration();
  const first = migrateServerState(storage);
  const once = storage.dump();
  const second = migrateServerState(storage);
  assert.deepEqual(storage.dump(), once, 'the second run changes nothing');
  assert.deepEqual(second.folded, [], 'and writes nothing');
  assert.deepEqual(second.unchanged, first.folded, 'it recognises its own work');
  // A third run, after a write through the server store, still converges.
  createServerStore(storage, 'a').setItem('tmux_hub_drafts', JSON.stringify({ work: 'newer' }));
  const third = migrateServerState(storage);
  assert.deepEqual(third.folded, ['tmux_hub_drafts'],
    'a live value that has moved on is folded again — the live key is the pre-②b writer');
});

test('with no current server the fold does nothing and says so', () => {
  // Guessing a slot would file A's drafts under B. The caller runs it again
  // once a connect has resolved which server this is.
  const storage = preMigration();
  storage.removeItem('tmux_server_current');
  const before = storage.dump();
  const report = migrateServerState(storage);
  assert.deepEqual(report, { ran: false, serverId: '', folded: [], unchanged: [] });
  assert.deepEqual(storage.dump(), before, 'not one key moved');
});

test('a fresh client has nothing to fold', () => {
  const storage = mem({ tmux_servers: '[]', tmux_server_current: 'only' });
  const report = migrateServerState(storage);
  assert.equal(report.ran, true);
  assert.deepEqual(report.folded, [], 'an absent live value is not evidence of an empty server');
  assert.deepEqual(storage.dump(), { tmux_server_current: 'only', tmux_servers: '[]' });
});

test('the fold survives a pre-#335 switch having happened in between', () => {
  // The realistic upgrade path: the user switches A→B→A on the OLD build, so
  // park/point has already shuffled the live keys, and only then does the new
  // build run the fold. Whatever is live belongs to whatever CURRENT says.
  const storage = preMigration();
  const servers = JSON.parse(storage.getItem('tmux_servers')!);
  parkFrom(storage, 'a');
  pointTo(storage, servers[1]);
  assert.equal(storage.getItem('tmux_server_current'), 'b');
  assert.deepEqual(JSON.parse(storage.getItem('tmux_hub_drafts')!), { work: 'half a line on B' },
    'the old switch surfaced B s park as the live value');

  const report = migrateServerState(storage);
  assert.equal(report.serverId, 'b');
  assert.deepEqual(JSON.parse(storage.getItem('tmux_hub_drafts::b')!), { work: 'half a line on B' });
  assert.deepEqual(JSON.parse(storage.getItem('tmux_hub_drafts::a')!), { work: 'half a line on A' },
    'and A s park, written by that switch, is A s resident value');

  // Both servers now read their own state through their own store.
  assert.deepEqual(JSON.parse(createServerStore(storage, 'a').getItem('tmux_hub_drafts')!), { work: 'half a line on A' });
  assert.deepEqual(JSON.parse(createServerStore(storage, 'b').getItem('tmux_hub_drafts')!), { work: 'half a line on B' });
});

test('the legacy switch still owns the live keys, which is why the fold stays off', () => {
  // The reviewer's P1 on the ② plan, as an executable fact: `parkFrom` writes
  // `<key>::<id>` — the very string a resident slot uses — from the LIVE key.
  // So while any module still writes the live key, a switch overwrites the
  // resident slot with that module's value, and while any module already
  // reads the resident slot, a write through the live key is lost to it.
  // Enabling the fold before the readers and writers move is therefore not a
  // partial improvement, it is a way to lose a draft; ②b flips the fold, the
  // stores, park/point and the reset entry points in ONE commit.
  const storage = preMigration();
  migrateServerState(storage);
  const a = createServerStore(storage, 'a');

  // A module that has already moved writes through the store.
  a.setItem('tmux_hub_drafts', JSON.stringify({ work: 'written the ②b way' }));
  assert.deepEqual(JSON.parse(storage.getItem('tmux_hub_drafts')!), { work: 'half a line on A' },
    'the live key never sees it — the two are independent homes');

  // Then today's switch runs, and files the live (older) value over it.
  parkFrom(storage, 'a');
  assert.deepEqual(JSON.parse(storage.getItem('tmux_hub_drafts::a')!), { work: 'half a line on A' },
    'parkFrom overwrote the resident slot: one commit must move both sides');
});

test('the fold does not invent a slot for a key the server never had', () => {
  const storage = preMigration();
  storage.removeItem('tmux_hub_lead');            // A never chose a lead
  migrateServerState(storage);
  assert.equal(storage.getItem('tmux_hub_lead::a'), null,
    'an absent live value leaves the slot absent, so a park under its own id stays the only copy');
});
