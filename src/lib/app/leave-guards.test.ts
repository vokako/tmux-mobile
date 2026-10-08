import test from 'node:test';
import assert from 'node:assert/strict';
import { clearLeaveGuardsForTests, confirmLeave, registerLeaveGuard, type LeaveWalk } from './leave-guards.ts';

test.beforeEach(() => clearLeaveGuardsForTests());

function walk(current: string, seen: string[]): LeaveWalk {
  return {
    current,
    reveal: async (page) => { seen.push(`show ${page}`); },
    hold: (on) => { seen.push(on ? 'hold' : 'release'); },
  };
}

test('the current page asks first; others are revealed, held, and the user ends where they started (board 315)', async () => {
  const seen: string[] = [];
  registerLeaveGuard({ page: 'files', dirty: () => true, ask: async () => { seen.push('files?'); return true; } });
  registerLeaveGuard({ page: 'hub', dirty: () => false, ask: async () => { seen.push('hub?'); return true; } });
  registerLeaveGuard({ page: 'prefs', dirty: () => true, ask: async () => { seen.push('prefs?'); return true; } });
  assert.equal(await confirmLeave(walk('prefs', seen)), true);
  assert.deepEqual(seen, ['hold', 'prefs?', 'show files', 'files?', 'show prefs', 'release'],
    'the current page answers before any navigation; the held pages are released last');
});

test('a "stay" stops the walk and still returns to the start; a guard that left on its own is skipped', async () => {
  const seen: string[] = [];
  let offLater = () => {};
  registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('first'); offLater(); return true; } });
  offLater = registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('gone'); return true; } });
  registerLeaveGuard({ page: 'files', dirty: () => true, ask: async () => { seen.push('third'); return false; } });
  registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('never'); return true; } });
  assert.equal(await confirmLeave(walk('hub', seen)), false);
  assert.deepEqual(seen, ['hold', 'first', 'show files', 'third', 'show hub', 'release']);
});

test('nothing dirty: no hold, no navigation', async () => {
  const seen: string[] = [];
  registerLeaveGuard({ page: 'files', dirty: () => false, ask: async () => true });
  assert.equal(await confirmLeave(walk('hub', seen)), true);
  assert.deepEqual(seen, []);
});
