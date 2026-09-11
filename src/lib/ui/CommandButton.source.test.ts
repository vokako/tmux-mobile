import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./CommandButton.svelte', import.meta.url), 'utf8');
const style = source.match(/<style>([\s\S]*?)<\/style>/u)?.[1] ?? '';

// #155 review: sharing the rotation must not change the pending command's tempo.
test('pending commands use the shared spin while retaining their loading tempo and reduced-motion guard (#156)', () => {
  assert.match(style, /\.spinning \{ animation: spin 0\.6s linear infinite; \}/u);
  assert.doesNotMatch(style, /@keyframes\s+(?:command-spin|spin)\b/u);
  assert.match(style, /@media \(prefers-reduced-motion: reduce\) \{[^}]*\}[^}]*\.spinning \{ animation: none; \}/u);
});

// The token/contrast test owns the endpoints; these assertions pin their use
// at the actual hover/press boundary instead of merely testing unused tokens.
test('solid command overlays mix the shared endpoints at the accepted opacity without literals (#156)', () => {
  const hover = style.match(/\.command-button\.solid:hover:not\(:disabled\) \{([^}]+)\}/u)?.[1] ?? '';
  const pressed = style.match(/\.command-button\.solid:active:not\(:disabled\) \{([^}]+)\}/u)?.[1] ?? '';
  // #161 moves the unchanged composites to the smaller paint layer.
  assert.match(hover, /--command-overlay:\s*color-mix\(in srgb, var\(--control-overlay-light\) 4%, transparent\);/u);
  assert.match(pressed, /--command-overlay:\s*color-mix\(in srgb, var\(--control-overlay-dark\) 6%, transparent\);/u);
  assert.match(style, /box-shadow: inset 0 0 0 100px var\(--command-overlay, transparent\)/u);
  assert.doesNotMatch(style, /(?:rgba?|hsla?)\(|#[\da-f]{3,8}\b/iu);
});

// Engaged tools retain their state wash on hover/press, without another size or fill dialect.
test('engaged icon tools keep token wash and readable ink through hover and press (#157)', () => {
  const engaged = style.match(/\.command-button\.engaged,\s*\.command-button\.engaged:hover:not\(:disabled\),\s*\.command-button\.engaged:active:not\(:disabled\) \{([^}]+)\}/u)?.[1] ?? '';
  // Neutral selected surface is the owner-selected compact skin; state ink remains.
  assert.match(engaged, /--command-paint:\s*var\(--control-surface\);/u);
  assert.match(engaged, /color:\s*var\(--accent-ink\);/u);
  assert.doesNotMatch(engaged, /(?:height|width|padding|transform|box-shadow):/u);
});

test('command paint is inset inside the native target, with a visible keyboard ring (#161)', () => {
  const button = style.match(/\.command-button \{([^}]+)\}/u)?.[1] ?? '';
  const paint = style.match(/\.command-button::before \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(button, /height: var\(--control-height\)/u);
  assert.match(button, /border: 0/u, 'a transparent border would subtract another two pixels from the paint');
  assert.match(paint, /inset: var\(--control-paint-inset\) 0/u);
  assert.match(paint, /pointer-events: none/u);
  assert.match(style, /\.icon-only::before \{ inset: var\(--control-paint-inset\); \}/u);
  assert.match(style, /\.command-button:focus-visible::before \{ outline: 2px solid var\(--accent-ink\); outline-offset: 2px; \}/u);
});

test('warn interruption has token ink and no ground without removing the focus layer (#173)', () => {
  assert.match(source, /variant\?: 'primary' \| 'secondary' \| 'icon' \| 'danger' \| 'warn'/u);
  assert.match(source, /class:warn=\{variant === 'warn'\}/u);
  const warn = style.match(/\.warn \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(warn, /color: var\(--status-warn\)/u);
  assert.doesNotMatch(warn, /height|width|padding|border-radius/u,
    'warning semantics do not create a second target geometry');
  // Hover/press change --command-paint. This more-specific pseudo rule must
  // ignore that variable, not merely make the resting paint transparent.
  const paint = style.match(/\.command-button\.warn::before \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(paint, /background: none/u);
  assert.match(paint, /box-shadow: none/u);
  assert.doesNotMatch(paint, /content|display|visibility|opacity|outline/u,
    'the pseudo remains available to the existing keyboard focus ring');
  assert.match(style, /\.command-button:focus-visible::before \{ outline: 2px solid var\(--accent-ink\); outline-offset: 2px; \}/u);
});
