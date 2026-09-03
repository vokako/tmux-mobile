// Source-contract tests for PanePicker.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./PanePicker.svelte', import.meta.url), 'utf8');

test('the retired unread-notification dots stay retired (2026-09-01)', () => {
  assert.doesNotMatch(source, /agent-notifications\.svelte/u);
  assert.doesNotMatch(source, /picker-attention|sessionHasNotification|NotificationForWindow/u);
});

test('the picker is the one popover mechanism, not a backdrop panel (2026-09-03)', () => {
  assert.doesNotMatch(source, /picker-backdrop/u, 'no backdrop button — dismissal is the shared set');
  assert.match(source, /import \{ anchorOf, menuPlacement, viewBox, type AnchorRect \} from '\.\.\/ui\/placement\.ts'/u);
  assert.match(source, /menuPlacement\(anchorRect, \{ w, h \}, viewBox\(\), 6, 8, align\)/u, 'placed from the opener, flipped and clamped by the shared math');
  const style = source.match(/<style>[\s\S]*<\/style>/u)?.[0] ?? '';
  assert.match(style, /\.picker \{[^}]*position: fixed/u, 'a fixed layer — a scrolling or overflow:hidden caller cannot clip it');
  assert.doesNotMatch(style, /top: 36px|left: 6px/u, 'no hard-coded offsets');
  // All four dismissals; only a scroller that contains the opener dismisses
  // (the list scrolling itself, or a terminal viewport under the panel, does not).
  for (const ev of ["'pointerdown', onDown, true", "'keydown', onKey, true", "'resize', onClose", "'scroll', onScroll, true"]) {
    assert.ok(source.includes(`window.addEventListener(${ev})`), `listens: ${ev}`);
  }
  assert.match(source, /if \(!\(scroller instanceof Node\) \|\| !opener \|\| !scroller\.contains\(opener\)\) return;/u);
});
