import test from 'node:test';
import assert from 'node:assert/strict';
import { clearLeaveGuardsForTests, confirmLeave, registerLeaveGuard } from './leave-guards.ts';

test.beforeEach(() => clearLeaveGuardsForTests());

test('only dirty guards ask, each on its own page, one at a time (board 315)', async () => {
  const seen: string[] = [];
  registerLeaveGuard({ page: 'files', dirty: () => false, ask: async () => { seen.push('files?'); return true; } });
  registerLeaveGuard({ page: 'hub', dirty: () => true, ask: async () => { seen.push('hub?'); return true; } });
  registerLeaveGuard({ page: 'prefs', dirty: () => true, ask: async () => { seen.push('prefs?'); return true; } });
  const ok = await confirmLeave(async (page) => { seen.push(`show ${page}`); });
  assert.equal(ok, true);
  assert.deepEqual(seen, ['show hub', 'hub?', 'show prefs', 'prefs?'], 'revealed before asking; clean guards are not asked');
});

test('a "stay" stops the walk; an unregistered guard is skipped', async () => {
  const seen: string[] = [];
  let offLater = () => {};
  registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('first'); offLater(); return true; } });
  offLater = registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('gone'); return true; } });
  registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('third'); return false; } });
  registerLeaveGuard({ page: '', dirty: () => true, ask: async () => { seen.push('never'); return true; } });
  assert.equal(await confirmLeave(async () => {}), false);
  assert.deepEqual(seen, ['first', 'third'], 'the unmounted one is skipped, nothing after a stay is asked');
});
