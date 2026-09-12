import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Select.svelte', import.meta.url), 'utf8');
const style = source.match(/<style>([\s\S]*?)<\/style>/u)?.[1] ?? '';

// #161 deliberately changes paint, not the actual trigger/anchor. Wrapping an
// input in a second click owner would put IME, blur and outside-dismiss at risk.
test('both Select modes use the shared inset field on the native trigger (#161)', async () => {
  assert.match(source, /<input class="sel-trigger control-field combo"/u);
  assert.match(source, /<button class="sel-trigger control-field"/u);
  assert.match(source, /anchor = anchorOf\(\(triggerEl \?\? inputEl\)!\)/u);
  assert.match(source, /bind:offsetHeight=\{menuH\}/u);
  const trigger = style.match(/\.sel-trigger \{([^}]+)\}/u)?.[1] ?? '';
  assert.doesNotMatch(trigger, /(?:^|[;}\s])(?:background|border|border-radius|height):/u,
    'there is one field paint owner, not a page/Select override');
  assert.match(style, /button\.sel-trigger \{ --field-paint: transparent;/u,
    'a select-only value uses its chevron rather than another heavy box');
  // #165 shares the row metrics without changing native trigger ownership.
  const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  assert.match(source, /class="sel-opt menu-item"/u);
  assert.match(css, /@media \(any-pointer: coarse\) \{\s*:root \{ --menu-row-height: 44px/u);
});
