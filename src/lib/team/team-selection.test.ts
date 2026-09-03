import assert from 'node:assert/strict';
import test from 'node:test';
import {
  TEAM_ACTIVE_ROOM_KEY,
  pickActiveRoom,
  readStoredActiveRoom,
  teamDisplayName,
  writeStoredActiveRoom,
} from './team-selection.ts';

const teams = [{ room: 'alpha' }, { room: 'beta' }];

test('keeps the current room when it still exists', () => {
  assert.equal(pickActiveRoom(teams, 'beta', 'alpha'), 'beta');
});

test('restores the stored room before falling back to the first team', () => {
  assert.equal(pickActiveRoom(teams, '', 'beta'), 'beta');
  assert.equal(pickActiveRoom(teams, 'missing', 'missing'), 'alpha');
  assert.equal(pickActiveRoom([], 'alpha', 'alpha'), '');
});

test('reads, writes, and clears the stored room', () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  } as unknown as Storage;

  writeStoredActiveRoom('beta', storage);
  assert.equal(values.get(TEAM_ACTIVE_ROOM_KEY), 'beta');
  assert.equal(readStoredActiveRoom(storage), 'beta');

  writeStoredActiveRoom('', storage);
  assert.equal(readStoredActiveRoom(storage), '');
});

test('ignores unavailable storage', () => {
  const storage = {
    getItem() { throw new Error('unavailable'); },
    setItem() { throw new Error('unavailable'); },
    removeItem() { throw new Error('unavailable'); },
  } as unknown as Storage;

  assert.equal(readStoredActiveRoom(storage), '');
  assert.doesNotThrow(() => writeStoredActiveRoom('alpha', storage));
});

test('a team is named by its project, then its folder, never by its room id (2026-09-03)', () => {
  const projects = [{ name: 'Mobile App', path: '/home/u/work/tmux-mobile' }];
  const team = { room: 'tmux-mobile-default-9f3a1c', workspace: '/home/u/work/tmux-mobile' };
  assert.equal(teamDisplayName(team, projects), 'Mobile App');
  // Trailing slashes do not break the match.
  assert.equal(teamDisplayName({ ...team, workspace: '/home/u/work/tmux-mobile/' }, projects), 'Mobile App');
  // No project owns the folder: the folder's own name, as Sessions labels a team session.
  assert.equal(teamDisplayName({ room: 'other-default-aa11bb', workspace: '/srv/other' }, projects), 'other');
  assert.equal(teamDisplayName({ room: 'other-default-aa11bb', workspace: '/srv/other' }), 'other');
  // No workspace at all: the room minus its 6-hex tail — still never the raw id.
  assert.equal(teamDisplayName({ room: 'other-default-aa11bb' }), 'other-default');
  assert.equal(teamDisplayName(null), '');
});
