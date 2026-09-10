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
