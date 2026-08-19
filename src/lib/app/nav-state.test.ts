import test from 'node:test';
import assert from 'node:assert/strict';
import { PAGES, defaultPage, restorePage, retarget } from './nav-state.ts';

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

test('retarget follows a renamed session, and only that session', () => {
  assert.equal(retarget('old:1.0', 'old', 'new'), 'new:1.0');
  assert.equal(retarget('old:12.3', 'old', 'renamed-probe'), 'renamed-probe:12.3');
  // A name that merely STARTS with the old one is a different session.
  assert.equal(retarget('older:1.0', 'old', 'new'), 'older:1.0');
  assert.equal(retarget('other:1.0', 'old', 'new'), 'other:1.0');
  // Nothing to do, nothing broken.
  const noops: [string, string, string][] = [
    ['', 'old', 'new'], ['old:1.0', '', 'new'], ['old:1.0', 'old', ''], ['old:1.0', 'x', 'x'],
  ];
  for (const [target, from, to] of noops) assert.equal(retarget(target, from, to), target);
});
