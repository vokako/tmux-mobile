// Unit tests for hub-prefs.svelte.ts — the runes module runs under node with
// the `$state` shim (docs/conventions/testing.md) and an in-memory localStorage.
import test from 'node:test';
import assert from 'node:assert/strict';

const store = new Map<string, string>();
const writes: string[] = [];
(globalThis as any).localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => { writes.push(k); store.set(k, String(v)); },
  removeItem: (k: string) => { store.delete(k); },
};
(globalThis as any).$state = (v: unknown) => v;

const { hubPrefs } = await import('./hub-prefs.svelte.ts');

test('the lead has three states: a name, the ROOM (empty string), and nobody chose (null)', () => {
  // Review C (2026-09-03): '' used to be stored as an ABSENT key, so choosing
  // "send to the room" was indistinguishable from never having chosen, and the
  // next roster poll re-seated a lead the user had just dismissed.
  assert.equal(hubPrefs.lead('proj-a'), null, 'nobody chose yet');
  hubPrefs.setLead('proj-a', 'dev');
  assert.equal(hubPrefs.lead('proj-a'), 'dev');
  hubPrefs.setLead('proj-a', '');
  assert.equal(hubPrefs.lead('proj-a'), '', 'the room is a real, remembered choice');
  assert.equal(JSON.parse(store.get('tmux_hub_lead')!)['proj-a'], '', 'and it is persisted as such');
  hubPrefs.clearLead('proj-a');
  assert.equal(hubPrefs.lead('proj-a'), null, 'cleared = back to nobody chose');
  assert.ok(!('proj-a' in JSON.parse(store.get('tmux_hub_lead')!)), 'a cleared project leaves no row');
});

test('renameSession carries the lead — including the room choice — onto the new session name', () => {
  hubPrefs.setLead('old', '');
  hubPrefs.setLead('other', 'qa');
  hubPrefs.renameSession('old', 'new');
  assert.equal(hubPrefs.lead('old'), null);
  assert.equal(hubPrefs.lead('new'), '', 'the room choice survives the rename');
  assert.equal(hubPrefs.lead('other'), 'qa', 'unrelated projects untouched');
});

const ROSTER_KEY = 'tmux_hub_roster_expanded';
let moduleVersion = 0;
async function freshPrefs(): Promise<typeof hubPrefs> {
  // Bypass Node's module cache to exercise the real localStorage read on reload.
  return (await import(new URL(`./hub-prefs.svelte.ts?roster=${++moduleVersion}`, import.meta.url).href)).hubPrefs;
}

test('roster expansion is per session and survives a fresh-module reload (#168)', async () => {
  store.clear();
  const prefs = await freshPrefs();
  assert.equal(prefs.rosterExpanded('alpha'), false);
  assert.equal(prefs.rosterExpanded('beta'), false);
  prefs.setRosterExpanded('alpha', true);
  assert.equal(prefs.rosterExpanded('alpha'), true);
  assert.equal(prefs.rosterExpanded('beta'), false);
  assert.deepEqual(JSON.parse(store.get(ROSTER_KEY)!), { alpha: true });

  const reloaded = await freshPrefs();
  assert.equal(reloaded.rosterExpanded('alpha'), true);
  assert.equal(reloaded.rosterExpanded('beta'), false);
  reloaded.setRosterExpanded('beta', true);
  reloaded.setRosterExpanded('alpha', false);
  assert.equal(reloaded.rosterExpanded('alpha'), false);
  assert.equal(reloaded.rosterExpanded('beta'), true);
  assert.deepEqual(JSON.parse(store.get(ROSTER_KEY)!), { beta: true }, 'collapsed sessions leave no entry');
  const collapsed = await freshPrefs();
  assert.equal(collapsed.rosterExpanded('alpha'), false);
  assert.equal(collapsed.rosterExpanded('beta'), true);
});

test('empty sessions do not write roster preferences and only true expands (#168)', async () => {
  store.clear();
  store.set(ROSTER_KEY, JSON.stringify({ alpha: true, beta: false, invalid: 'true', '': true }));
  const prefs = await freshPrefs();
  assert.equal(prefs.rosterExpanded('alpha'), true);
  assert.equal(prefs.rosterExpanded('beta'), false);
  assert.equal(prefs.rosterExpanded('invalid'), false);
  assert.equal(prefs.rosterExpanded(''), false);
  const saved = store.get(ROSTER_KEY);
  writes.length = 0;
  prefs.setRosterExpanded('', true);
  prefs.setRosterExpanded('', false);
  assert.deepEqual(writes, []);
  assert.equal(store.get(ROSTER_KEY), saved);
});

test('renameSession moves and persists roster expansion exactly once (#168)', async () => {
  store.clear();
  const prefs = await freshPrefs();
  prefs.setRosterExpanded('old', true);
  prefs.setRosterExpanded('other', true);
  prefs.setDrawer('old', 'files');
  prefs.setLead('old', '');
  prefs.setSeen('old', 123);
  prefs.setDraft('old', 'draft');
  prefs.setProject('old');
  writes.length = 0;
  prefs.renameSession('old', 'new');
  assert.equal(writes.filter(key => key === ROSTER_KEY).length, 1);
  assert.deepEqual(JSON.parse(store.get(ROSTER_KEY)!), { other: true, new: true });
  assert.equal(prefs.rosterExpanded('old'), false);
  assert.equal(prefs.rosterExpanded('new'), true);
  assert.equal(prefs.rosterExpanded('other'), true);
  assert.equal(prefs.drawer('new'), 'files');
  assert.equal(prefs.lead('new'), '');
  assert.equal(prefs.seen('new'), 123);
  assert.equal(prefs.draft('new'), 'draft');
  assert.equal(prefs.project, 'new');
  const reloaded = await freshPrefs();
  assert.equal(reloaded.rosterExpanded('old'), false);
  assert.equal(reloaded.rosterExpanded('new'), true);
  assert.equal(reloaded.rosterExpanded('other'), true);
  writes.length = 0;
  reloaded.renameSession('', 'new');
  reloaded.renameSession('new', '');
  reloaded.renameSession('new', 'new');
  assert.deepEqual(writes, [], 'invalid and identity renames remain no-ops');
});
