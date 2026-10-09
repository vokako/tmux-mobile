import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Segmented.svelte', import.meta.url), []);
const options = [{ value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }];

test('segments retain a single chosen marker and emit one choice', async context => {
  const changes: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { options, value: 'light', ariaLabel: 'Theme', onchange: (v: string) => changes.push(v) }, modules: [],
  });
  try {
    assert.equal(app.document.querySelectorAll('.slide-pill').length, 1);
    assert.equal(app.document.querySelector('[role=group]')?.getAttribute('aria-label'), 'Theme');
    const buttons = app.document.querySelectorAll('button');
    assert.equal(buttons[0]!.getAttribute('aria-pressed'), 'true');
    buttons[1]!.click();
    assert.deepEqual(changes, ['dark']);
  } finally { await app.close(); }
});

test('disabled segments block activation', async context => {
  const app = await (await compiled).mount(context, {
    props: { options, value: 'light', disabled: true, onchange: () => assert.fail('disabled segment') }, modules: [],
  });
  try {
    for (const button of app.document.querySelectorAll('button')) {
      button.click();
      button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    }
  } finally { await app.close(); }
});

test('an icon row names its options for assistive tech; a MIXED row falls back to text (#326)', async context => {
  const icons = [{ value: 'bottom', label: 'Bottom', icon: 'panel-bottom' }, { value: 'right', label: 'Right', icon: 'panel-right' }];
  const app = await (await compiled).mount(context, {
    props: { options: icons, value: 'bottom', ariaLabel: 'Panel edge', onchange: () => {} }, modules: [],
  });
  try {
    const buttons = [...app.document.querySelectorAll('button')];
    assert.deepEqual(buttons.map((b) => b.getAttribute('aria-label')), ['Bottom', 'Right'],
      'the label is the accessible name when the glyph replaces it');
    assert.ok(buttons.every((b) => !b.textContent?.trim()), 'and no words are drawn');
    assert.ok(buttons.every((b) => b.querySelector('svg')), 'each option draws its glyph');
    assert.equal(buttons[0]!.getAttribute('aria-pressed'), 'true', 'the state contract is the row’s own');
    assert.ok(app.document.querySelector('.segmented')!.classList.contains('iconic'));
  } finally { await app.close(); }
});

test('a mixed icon/text row draws TEXT for every option', async context => {
  // The pill travels between cells, so it cannot cross two shapes: a row that
  // is not wholly iconic is wholly textual. Pinned because the alternative —
  // rendering each option in its own dialect — looks reasonable in a diff.
  const mixed = [{ value: 'a', label: 'Alpha', icon: 'panel-bottom' }, { value: 'b', label: 'Beta' }];
  const app = await (await compiled).mount(context, {
    props: { options: mixed, value: 'a', onchange: () => {} }, modules: [],
  });
  try {
    const buttons = [...app.document.querySelectorAll('button')];
    assert.deepEqual(buttons.map((b) => b.textContent?.trim()), ['Alpha', 'Beta']);
    assert.ok(buttons.every((b) => !b.querySelector('svg')), 'no half-iconic row');
    assert.ok(!app.document.querySelector('.segmented')!.classList.contains('iconic'));
    assert.ok(buttons.every((b) => !b.getAttribute('aria-label')), 'the words are the name again');
  } finally { await app.close(); }
});
