import assert from 'node:assert/strict';
import test from 'node:test';
import {
  TEAM_ACTIVE_ROOM_KEY,
  pickActiveRoom,
  readStoredActiveRoom,
  writeStoredActiveRoom,
} from './team-selection.js';

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
  const values = new Map();
  const storage = {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: key => values.delete(key),
  };

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
  };

  assert.equal(readStoredActiveRoom(storage), '');
  assert.doesNotThrow(() => writeStoredActiveRoom('alpha', storage));
});
