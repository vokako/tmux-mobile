import assert from 'node:assert/strict';
import test from 'node:test';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./OperationFeedback.svelte', import.meta.url), []);

test('feedback distinguishes polite completion from persistent error semantics (#167)', async context => {
  const app = await (await compiled).mount(context, {
    props: { value: { kind: 'error', message: 'Copy failed', detail: 'Permission denied' } }, modules: [],
  });
  try {
    const error = app.document.querySelector('[role=alert]')!;
    assert.match(error.textContent ?? '', /Copy failed.*Permission denied/);
    assert.ok(error.querySelector('.config-error'), 'same error text role as confirmation/form errors');
    await app.advance(10000);
    assert.ok(app.document.querySelector('[role=alert]'), 'presentation does not invent an expiry');
  } finally { await app.close(); }
});

test('only a measured transfer percentage has a numeric progress value (#167)', async context => {
  for (const value of [null, 25.4]) {
    const app = await (await compiled).mount(context, {
      props: { value: { kind: 'progress', message: 'Downloading report', progress: value } }, modules: [],
    });
    try {
      if (value === null) {
        const meter = app.document.querySelector('[role=progressbar]')!;
        assert.equal(meter.getAttribute('aria-valuenow'), null);
        assert.equal(app.document.querySelector('.feedback-percent'), null);
        assert.ok(app.document.querySelector('.spinning'));
      } else {
        assert.equal(app.document.querySelector('progress')?.value, 25);
        assert.equal(app.document.querySelector('.feedback-percent')?.textContent, '25%');
      }
    } finally { await app.close(); }
  }
});

test('actionable results stay present and dismiss through caller intent only (#167)', async context => {
  let dismissals = 0;
  const app = await (await compiled).mount(context, {
    props: { value: { kind: 'result', message: 'Saved', detail: '/downloads/report.txt' },
      ondismiss: () => dismissals++ }, modules: [],
  });
  try {
    await app.advance(10000);
    assert.ok(app.document.querySelector('[role=status]'));
    app.document.querySelector<HTMLButtonElement>('button')!.click();
    assert.equal(dismissals, 1);
    assert.ok(app.document.querySelector('[role=status]'), 'the caller owns clearing the result');
  } finally { await app.close(); }
});

test('a download keeps its ring turning with a known percent; other progress keeps refresh (board #307)', async context => {
  for (const [glyph, progress] of [['download', 40], ['download', null], [undefined, 40]] as const) {
    const app = await (await compiled).mount(context, {
      props: { value: { kind: 'progress', message: 'Downloading', progress, ...(glyph ? { glyph } : {}) } }, modules: [],
    });
    try {
      const icon = app.document.querySelector('.feedback-icon')!;
      if (glyph) {
        assert.ok(icon.classList.contains('downloading'), `the ring turns at ${progress}`);
        assert.ok(!icon.classList.contains('spinning'), 'the whole glyph does not');
        assert.ok(icon.querySelector('circle.dl-ring'));
      } else {
        assert.ok(!icon.classList.contains('downloading') && !icon.querySelector('.dl-ring'), 'others keep refresh');
        assert.ok(!icon.classList.contains('spinning'), 'which still stops at a known percent');
      }
    } finally { await app.close(); }
  }
});
