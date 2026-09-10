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
  assert.match(hover, /box-shadow: inset 0 0 0 100px color-mix\(in srgb, var\(--control-overlay-light\) 4%, transparent\);/u);
  assert.match(pressed, /box-shadow: inset 0 0 0 100px color-mix\(in srgb, var\(--control-overlay-dark\) 6%, transparent\);/u);
  assert.doesNotMatch(style, /(?:rgba?|hsla?)\(|#[\da-f]{3,8}\b/iu);
});

// Engaged tools retain their state wash on hover/press, without another size or fill dialect.
test('engaged icon tools keep token wash and readable ink through hover and press (#157)', () => {
  const engaged = style.match(/\.command-button\.engaged,\s*\.command-button\.engaged:hover:not\(:disabled\),\s*\.command-button\.engaged:active:not\(:disabled\) \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(engaged, /background:\s*var\(--accent-bg\);/u);
  assert.match(engaged, /color:\s*var\(--accent-ink\);/u);
  assert.doesNotMatch(engaged, /(?:height|width|padding|transform|box-shadow):/u);
});
