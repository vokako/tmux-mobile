// Source-contract test for the type scale (see docs/conventions/testing.md).
//
// The app has ONE font-size vocabulary — the `--fs-*` steps on `:root` in
// app.css — and the point of the 2026-08-19 audit was that 185 raw px values
// had grown around it, drifting half a pixel apart per page ("对我们全部的 ui
// 里的字号系统做一个梳理，不要出现太多 hardcode"). Rounding them onto the scale
// is only half the fix; without a guard the next component re-grows its own.
//
// So: no component may name a raw px font-size. The exceptions are listed here
// BY REASON, which is the other thing this test buys — an exception has to be
// argued for in one place instead of hiding in a stylesheet.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';

const SRC = new URL('../../', import.meta.url);   // src/

/** Raw px font sizes that are NOT typography and must stay raw. */
const ALLOWED = [
  // SVG user units inside a viewBox: the graph scales with its viewport, so a
  // CSS px token would be measured against the wrong box.
  { file: 'lib/team/CollabGraph.svelte', match: '.lbl { fill:' },
  // Deliberately below --fs-input-touch: mono glyphs are wider and this
  // textarea was tuned by eye. Changing it is a behaviour change, not cleanup.
  { file: 'lib/team/TeamTemplates.svelte', match: '.ag-mono { font-size: 15px; }' },
];

async function* walk(dir: URL): AsyncGenerator<URL> {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const child = new URL(entry.name + (entry.isDirectory() ? '/' : ''), dir);
    if (entry.isDirectory()) yield* walk(child);
    else if (/\.(svelte|css)$/u.test(entry.name)) yield child;
  }
}

test('no component names a raw px font-size — the scale is the vocabulary', async () => {
  const offenders: string[] = [];
  for await (const file of walk(SRC)) {
    const rel = file.href.slice(SRC.href.length);
    const text = await readFile(file, 'utf8');
    text.split('\n').forEach((line, i) => {
      if (!/font-size:\s*\d/u.test(line)) return;
      // A relative size (em) is a document scaling correctly with its base.
      if (/font-size:\s*[\d.]+em/u.test(line)) return;
      if (ALLOWED.some((a) => rel === a.file && line.includes(a.match))) return;
      offenders.push(`${rel}:${i + 1}  ${line.trim()}`);
    });
  }
  assert.deepEqual(offenders, [], `use a --fs-* step instead:\n${offenders.join('\n')}`);
});

test('the scale itself is six chrome steps plus the named exceptions', async () => {
  const css = await readFile(new URL('app.css', SRC), 'utf8');
  for (const token of ['--fs-micro', '--fs-meta', '--fs-sub', '--fs-ui', '--fs-body', '--fs-title']) {
    assert.match(css, new RegExp(`${token}:\\s*[\\d.]+px`, 'u'), `${token} must be defined`);
  }
  // Display steps are for the connect card only; the input constant is a
  // BEHAVIOUR (iOS auto-zooms a focused input below 16px), not a type step.
  assert.match(css, /--fs-hero:\s*[\d.]+px/u);
  assert.match(css, /--fs-display:\s*[\d.]+px/u);
  assert.match(css, /--fs-input-touch:\s*16px/u);
  // The control alias must resolve to a step, never to its own number.
  assert.match(css, /--ui-font-control:\s*var\(--fs-[a-z]+\)/u);
});
