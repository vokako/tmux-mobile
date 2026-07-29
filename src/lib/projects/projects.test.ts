import test from 'node:test';
import assert from 'node:assert/strict';
import { ageLabel, shortPath, sortRows, windowChips, type ProjectRow, type Slot } from './projects.ts';

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

test('window chips show only the windows up would restore', () => {
  const chips = windowChips([
    slot('logs', 1),
    slot('editor', 0),
    slot('scratch', 2, { settled_at: undefined }),
    slot('kiro', 3, { kind: 'agent', command: 'kiro', auto_run: true }),
  ]);
  assert.deepEqual(
    chips,
    [
      { name: 'editor', agent: null },
      { name: 'logs', agent: null },
      { name: 'kiro', agent: 'kiro' },
    ],
    'window order, no unsettled window, agents carry their backend',
  );
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
