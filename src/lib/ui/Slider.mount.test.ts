import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Slider.svelte', import.meta.url), []);

test('slider retains native range semantics and has a named reset command', async context => {
  const changes: number[] = [];
  const app = await (await compiled).mount(context, {
    props: { label: 'Line height', resetLabel: 'Reset line height', value: 1.2,
      min: 1, max: 2, step: 0.1, defaultValue: 1, onchange: (n: number) => changes.push(n) }, modules: [],
  });
  try {
    const range = app.document.querySelector('input')!;
    assert.equal(range.type, 'range');
    assert.equal(range.getAttribute('aria-label'), 'Line height');
    assert.equal(range.step, '0.1');
    assert.equal(range.valueAsNumber, 1.2);
    range.value = '1.5';
    range.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    assert.equal(range.valueAsNumber, 1.2, 'an uncommitted intent does not leave the thumb at another value');
    assert.equal(app.document.querySelector('output')?.textContent, '1.2');
    app.document.querySelector<HTMLButtonElement>('[aria-label="Reset line height"]')!.click();
    assert.deepEqual(changes, [1.5, 1]);
  } finally { await app.close(); }
});
