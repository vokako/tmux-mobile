import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('edge fade paints only measured overflow and uses alpha, never a pointer-blocking overlay (#176)', async () => {
  const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  const rule = css.match(/\.edge-fade:is\(\.edge-before, \.edge-after\) \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(rule, /mask-mode: alpha/u);
  assert.match(rule, /mask-image: linear-gradient/u);
  assert.match(rule, /--edge-fade-start, 0px/u);
  assert.match(rule, /--edge-fade-end, 0px/u);
  assert.doesNotMatch(rule, /position|pointer-events|background|transition/u);
  assert.match(css, /\.edge-fade:has\(:focus-visible\) \{ mask-image: none; \}/u,
    'native Tab and menu focus return must not leave a focused glyph/ring faded');
});
