// Source contract for the ONE segmented control (design-language.md §3,
// motion.md §1.14, board #86): the chosen option is marked by a pill that
// TRAVELS (ui/indicator.ts), so every segmented row in the app must be this
// component — a hand-rolled `.segmented` div would light its buttons up in
// place and drift from the dialect without anything failing.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const SRC = new URL('../', import.meta.url); // src/lib/
const source = await readFile(new URL('ui/Segmented.svelte', SRC), 'utf8');
const style = source.match(/<style>[\s\S]*<\/style>/u)?.[0] ?? '';

test('the pill is the atom and the action places it', () => {
  assert.match(source, /<div class="segmented" role="group"[^>]*use:slideIndicator=\{\{ key: value, active: '\.active' \}\}>/u,
    'the container carries the action keyed on the value');
  assert.match(source, /<span class="slide-pill" aria-hidden="true"><\/span>/u, 'the pill is the first child');
  assert.match(source, /class="state-ctl" class:active=\{o\.value === value\} aria-pressed=\{o\.value === value\}/u,
    'each option is a .state-ctl (ink cross-fades) and announces its state');
  assert.match(style, /\.segmented \{ position: relative;/u, 'the container is the pill’s containing block');
  assert.match(style, /\.segmented button \{\s*position: relative; z-index: 1;/u, 'the buttons sit above the pill');
});

test('the chosen option keeps only its ink — the wash and the ring are the pill’s', () => {
  const active = style.match(/\.segmented button\.active \{([^}]*)\}/u)?.[1] ?? '';
  assert.match(active, /color: var\(--accent\)/u);
  assert.match(active, /border-color: transparent/u, 'its own border yields to the pill’s ring');
  assert.doesNotMatch(active, /background/u, 'no background of its own — that would be a second highlight');
  // The atom's look (wash, ring, glide) lives in app.css, not here.
  assert.doesNotMatch(style, /\.slide-pill/u);
});

test('Preferences spells every segmented row through the component', async () => {
  const prefs = await readFile(new URL('app/Preferences.svelte', SRC), 'utf8');
  const markup = prefs.replace(/<style>[\s\S]*<\/style>/u, '');
  assert.doesNotMatch(markup, /class="segmented/u, 'no hand-rolled .segmented markup');
  assert.match(markup, /import Segmented from '\.\.\/ui\/Segmented\.svelte'/u);
  const uses = markup.match(/<Segmented\b/gu) ?? [];
  assert.ok(uses.length >= 7, `theme, language, layout, feed level, notify on/off, notify level, debug — got ${uses.length}`);
  assert.doesNotMatch(prefs, /\.segmented/u, 'the dialect’s CSS moved with it');
});
