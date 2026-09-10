import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Stepper.svelte', import.meta.url), []);
const labels = { label: 'Amount', decreaseLabel: 'Decrease amount', increaseLabel: 'Increase amount' };

test('stepper uses named commands, formats its value and clamps at the boundary', async context => {
  const changes: number[] = [];
  const app = await (await compiled).mount(context, {
    props: { ...labels, value: 9, min: 0, max: 10, step: 3,
      format: (n: number) => `${n}%`, onchange: (n: number) => changes.push(n) }, modules: [],
  });
  try {
    assert.equal(app.document.querySelector('output')?.textContent, '9%');
    app.document.querySelector<HTMLButtonElement>('[aria-label="Increase amount"]')!.click();
    app.document.querySelector<HTMLButtonElement>('[aria-label="Decrease amount"]')!.click();
    assert.deepEqual(changes, [10, 6]);
  } finally { await app.close(); }
});

test('a stepper at its upper bound cannot increment', async context => {
  const app = await (await compiled).mount(context, {
    props: { ...labels, value: 10, min: 0, max: 10, onchange: () => assert.fail('past maximum') }, modules: [],
  });
  try {
    const plus = app.document.querySelector<HTMLButtonElement>('[aria-label="Increase amount"]')!;
    assert.equal(plus.disabled, true);
    plus.click();
  } finally { await app.close(); }
});
