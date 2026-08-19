import test from 'node:test';
import assert from 'node:assert/strict';
import { PAGES, defaultPage, restorePage } from './nav-state.ts';

test('a saved tab is restored, and only a known one', () => {
  for (const p of PAGES) {
    assert.equal(restorePage(p, false), p, `${p} survives a reload`);
    assert.equal(restorePage(p, true), p);
  }
  // The tab used to be dropped unless a terminal target was saved with it, so
  // reading the chat and refreshing landed on the device default.
  assert.equal(restorePage('hub', true), 'hub', 'a phone may sit on the chat');
  assert.equal(restorePage('terminal', false), 'terminal', 'a desktop may sit on the terminal');
});

test('anything else falls back to the device default', () => {
  for (const bad of [undefined, null, '', 'crew', 'Hub', 42, {}, []]) {
    assert.equal(restorePage(bad, true), 'terminal', `${JSON.stringify(bad)} on a phone`);
    assert.equal(restorePage(bad, false), 'hub', `${JSON.stringify(bad)} on a desktop`);
  }
  assert.equal(defaultPage(true), 'terminal');
  assert.equal(defaultPage(false), 'hub');
});
