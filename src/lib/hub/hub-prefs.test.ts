// Unit tests for hub-prefs.svelte.ts — the runes module runs under node with
// the `$state` shim (docs/conventions/testing.md) and an in-memory localStorage.
import test from 'node:test';
import assert from 'node:assert/strict';

const store = new Map<string, string>();
(globalThis as any).localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => { store.set(k, String(v)); },
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
