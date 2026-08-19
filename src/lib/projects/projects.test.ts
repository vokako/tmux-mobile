import test from 'node:test';
import assert from 'node:assert/strict';
import { ageLabel, declaredWindowChips, liveWindowChips, shortPath, sortRows, type ChipPane, type ProjectRow, type Slot } from './projects.ts';

function slot(name: string, ord: number, extra: Partial<Slot> = {}): Slot {
  return {
    ord,
    window_name: name,
    cwd: '',
    kind: 'shell',
    auto_run: false,
    first_seen_at: 1000,
    settled_at: 1200,
    ...extra,
  };
}

function pane(session: string, window: number, paneIdx: number, extra: Partial<ChipPane> = {}): ChipPane {
  return {
    session,
    window,
    pane: paneIdx,
    window_name: `w${window}`,
    current_command: 'zsh',
    active: true,
    ...extra,
  };
}

function row(id: string, live: boolean, lastSeen?: number): ProjectRow {
  return {
    project: {
      id,
      name: id,
      path: `/w/${id}`,
      session: id,
      adopted: false,
      autostart: false,
      created_at: 100,
      last_seen_at: lastSeen,
      archived: false,
    },
    slots: [],
    live,
  };
}

test('a closed project shows only the windows up would restore', () => {
  const chips = declaredWindowChips([
    slot('logs', 1),
    slot('editor', 0),
    slot('scratch', 2, { settled_at: undefined }),
    slot('kiro', 3, { kind: 'agent', command: 'kiro', auto_run: true }),
  ]);
  assert.deepEqual(
    chips.map((c) => c.name),
    ['editor', 'logs', 'kiro'],
    'window order, and no unsettled window',
  );
  assert.equal(chips[2]?.agentTag, 'Kiro', 'the agent backend becomes its icon');
  assert.equal(chips[0]?.target, null, 'nothing to open while the project is down');
});

test('a live project offers one tappable chip per window, from its active pane', () => {
  const chips = liveWindowChips([
    pane('app', 2, 1, { window_name: 'api', active: false }),
    pane('app', 2, 2, { window_name: 'api', active: true, current_command: 'node' }),
    pane('app', 1, 1, { window_name: 'editor' }),
    // A window that has not settled into the declaration yet is still live and
    // still worth jumping into.
    pane('app', 3, 1, { window_name: 'scratch' }),
  ]);
  assert.deepEqual(
    chips.map((c) => [c.name, c.target]),
    [
      ['editor', 'app:1.1'],
      ['api', 'app:2.2'],
      ['scratch', 'app:3.1'],
    ],
    'window order, active pane wins as the tap target',
  );
});

test('a live agent window carries its icon from the running process', () => {
  const chips = liveWindowChips([
    pane('app', 1, 1, { window_name: 'agent', current_command: 'kiro-cli-chat' }),
  ]);
  assert.equal(chips.length, 1);
  assert.equal(chips[0]?.agentTag, 'Kiro');
  assert.ok(chips[0]?.agentIcon?.endsWith('kiro.svg'));
});

test('live projects sort first, then by when tmux last had them', () => {
  const rows = sortRows([
    row('cold', false, 500),
    row('warm', false, 900),
    row('open', true, 100),
  ]);
  assert.deepEqual(rows.map((r) => r.project.id), ['open', 'warm', 'cold']);
});

test('a project with no recorded activity falls back to its creation time', () => {
  const fresh = row('fresh', false);
  fresh.project.created_at = 9999;
  const rows = sortRows([row('old', false, 500), fresh]);
  assert.deepEqual(rows.map((r) => r.project.id), ['fresh', 'old']);
});

test('the conversation orders the sidebar, not whichever session tmux touched last', () => {
  // The symptom this fixes: `last_seen_at` is rewritten by the capturer on every
  // tick, so for a live project it always means "just now" — every live project
  // floats up and their order is arbitrary.
  const a = row('a', true, 1_000_000);   // live, captured a moment ago
  const b = row('b', true, 1_000_001);   // live, captured a moment later
  const c = row('c', false, 500);        // stopped ages ago
  a.project.room = 'proj:a';
  b.project.room = 'proj:b';
  c.project.room = 'proj:c';
  // We talked in C most recently, then A. Never in B.
  const talk = { 'proj:c': 9_000_000_000, 'proj:a': 8_000_000_000 };
  assert.deepEqual(sortRows([a, b, c], talk).map((r) => r.project.id), ['c', 'a', 'b'],
    'newest conversation first; the one nobody talked in goes last');

  // Without the map, nothing changes: the Projects page keeps its own ordering.
  assert.deepEqual(sortRows([a, b, c]).map((r) => r.project.id), ['b', 'a', 'c']);

  // A project with no `room` recorded falls back to the derived id, because the
  // column was backfilled as `proj:<session>` and an older row may predate it.
  const d = row('d', false, 100);
  delete d.project.room;
  assert.deepEqual(sortRows([c, d], { 'proj:d': 9_999_999_999, 'proj:c': 1 })
    .map((r) => r.project.id), ['d', 'c']);

  // Seconds vs milliseconds: the projects table is in seconds and the bus is in
  // ms, so a conversation must never lose to a raw `last_seen_at`.
  const talked = row('talked', false, 1);
  talked.project.room = 'proj:talked';
  const busy = row('busy', false, 1_800_000_000);   // seconds: ~2027
  assert.deepEqual(
    sortRows([busy, talked], { 'proj:talked': 1_700_000_000_000 }).map((r) => r.project.id),
    ['talked', 'busy'],
  );
});

test('long paths collapse to the last two segments', () => {
  assert.equal(shortPath('/Users/me/work/app'), '/Users/me/work/app');
  assert.equal(
    shortPath('/Users/me/work/very-long-project-name/packages/api'),
    '…/packages/api',
  );
  assert.equal(shortPath('/a/b', 4), '/a/b', 'nothing to collapse');
});

test('age labels stay short', () => {
  const now = 1_000_000;
  assert.equal(ageLabel(now - 10, now), 'now');
  assert.equal(ageLabel(now - 600, now), '10m');
  assert.equal(ageLabel(now - 7200, now), '2h');
  assert.equal(ageLabel(now - 86400 * 3, now), '3d');
  assert.equal(ageLabel(undefined, now), '');
});
