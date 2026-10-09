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
  assert.match(source, /<div class="segmented" class:iconic role="group"[^>]*use:slideIndicator=\{\{ key: value, active: '\.active' \}\}>/u,
    'the container carries the action keyed on the value');
  assert.match(source, /<span class="slide-pill control" aria-hidden="true"><\/span>/u, 'the shared compact paint variant is the first child');
  assert.match(source, /class="state-ctl" class:active=\{o\.value === value\} aria-pressed=\{o\.value === value\}/u,
    'each option is a .state-ctl (ink cross-fades) and announces its state');
  assert.match(style, /\.segmented \{\s*position: relative;/u, 'the container is the pill’s containing block');
  assert.match(style, /\.segmented button \{\s*position: relative; z-index: 1;/u, 'the buttons sit above the pill');
});

test('compact selection uses one neutral marker, not outlined boxes per choice (#161)', () => {
  const active = style.match(/\.segmented button\.active \{([^}]*)\}/u)?.[1] ?? '';
  // Owner-selected #160 replaces #155's group outline; action/geometry stay shared.
  assert.match(active, /font-weight: 600/u, 'a text-weight cue survives loss of color');
  assert.match(style, /\.segmented::before \{[^}]*background: var\(--control-surface\)/u, 'one quiet inset track');
  assert.doesNotMatch(style, /var\(--control-border\)/u);
  assert.match(style, /\.segmented button \{[^}]*border: 0/u, 'no separate option frames');
  assert.doesNotMatch(active, /background/u, 'no background of its own — that would be a second highlight');
  // The atom's look (compact surface and glide) lives in app.css, not here.
  assert.doesNotMatch(style, /\.slide-pill/u);
});

test('equal segments expose their actual max-content width to compact row wrapping (#162)', () => {
  // A flex group's sum of unequal label widths underestimates the equal
  // rendered tracks. Native grid sizing accounts for the longest option.
  const group = style.match(/\.segmented \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(group, /display: grid/u);
  assert.match(group, /grid-auto-flow: column/u);
  assert.match(group, /grid-auto-columns: 1fr/u);
});

test('Preferences spells every segmented row through the component', async () => {
  const prefs = await readFile(new URL('app/Preferences.svelte', SRC), 'utf8');
  const markup = prefs.replace(/<style>[\s\S]*<\/style>/u, '');
  assert.doesNotMatch(markup, /class="segmented/u, 'no hand-rolled .segmented markup');
  assert.match(markup, /import Segmented from '\.\.\/ui\/Segmented\.svelte'/u);
  const uses = markup.match(/<Segmented\b/gu) ?? [];
  // #156: boolean preferences deliberately move to Switch, not a second
  // segmented implementation. Only the five multi-choice preferences remain.
  assert.ok(uses.length >= 5, `theme, language, layout, feed level, notify level — got ${uses.length}`);
  assert.match(markup, /<Switch checked=\{notifyOn\}/u);
  assert.doesNotMatch(prefs, /\.segmented/u, 'the dialect’s CSS moved with it');
});

test('an icon option draws a glyph and keeps its words as the accessible name (#326)', () => {
  // The scratch panel's two edges are icons ("你用两个小图标去做状态切换"), and
  // that is a MODE of the one segmented control, not two hand-rolled buttons:
  // same travelling pill, same aria-pressed, same disabled contract.
  assert.match(source, /options: \{ value: T; label: string; icon\?: string \}\[\];/u, 'the icon is part of the option');
  assert.match(source, /const iconic = \$derived\(options\.every\(\(o\) => !!o\.icon\)\);/u,
    'a row is iconic only when EVERY option is — the pill cannot travel between cells of two shapes');
  assert.match(source, /aria-label=\{iconic \? o\.label : undefined\}/u, 'the label becomes the accessible name');
  assert.match(source, /\{#if iconic\}<Icon name=\{o\.icon \?\? ''\} size=\{14\} \/>\{:else\}\{o\.label\}\{\/if\}/u,
    'and the whole row follows one verdict, so a mixed row cannot render half-iconic');
  // An icon-only control needs a NAME on hover, and the app has exactly one
  // way to give it: the shared hover card (design-language §3 — a native
  // `title` next to it would be a second tooltip species).
  assert.match(source, /use:hoverInfo=\{iconic \? \(\) => \(\{ title: o\.label \}\) : null\}/u);
  assert.doesNotMatch(source, /title=/u, 'no native title beside the shared hover card');
  // Icon cells are square and wear the icon-only ink families; the text rows
  // keep their own padding and size untouched.
  assert.match(style, /\.segmented\.iconic button \{ display: grid; place-items: center; padding: 0; color: var\(--text2\); \}/u);
  assert.match(style, /\.segmented\.iconic button\.active \{ color: var\(--text\); \}/u);
  assert.match(style, /\.segmented button \{[^}]*padding: 0 8px/u, 'the text dialect is unchanged');
});
