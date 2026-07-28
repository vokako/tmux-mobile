// Source-contract tests for Terminal.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');

test('Terminal chrome uses only Team-filtered notification queries', () => {
  assert.match(source, /attention=\{otherTerminalSessionHasNotification\(session\)\}/u);
  assert.match(
    source,
    /\{@const notice = terminalNotificationForWindow\(w\.session, w\.window\)\}/u,
  );
  // The unfiltered query must never appear in Terminal chrome — Team dots
  // would leak into the window switcher.
  assert.doesNotMatch(source, /\bnotificationForWindow\(/u);
});

test('every key send returns the display to the live tail', () => {
  // Rendering is suppressed by a pinned selection, an unsettled touch scroll
  // and a scrolled-up viewport. If input does not release them, the typed
  // characters never appear: the server does not re-send a frame it already
  // delivered, so the screen stays frozen on the pre-input snapshot.
  assert.match(
    source,
    /function enqueueKeys\([^)]*\)\s*\{\s*(?:\/\/[^\n]*\n\s*)*resumeLiveTailRef\?\.\(\)/u,
  );
  // Paste is input too and bypasses enqueueKeys (it goes to paste_text).
  assert.match(source, /isPasting = false;\s*(?:\/\/[^\n]*\n\s*)*resumeLiveTail\(\)/u);
});

test('resumeLiveTail releases every render-suppressing state', () => {
  const body = /function resumeLiveTail\(\) \{([\s\S]*?)\n    \}/u.exec(source)?.[1];
  assert.ok(body, 'resumeLiveTail must exist');
  assert.match(body, /if \(selection\) clearSelection\(\);/u);
  assert.match(body, /stopMomentum\(\);/u); // a coast re-parks the viewport otherwise
  assert.match(body, /touchScrolling = false;/u);
  assert.match(body, /termAtBottom = true;/u);
  assert.match(body, /writeToXterm\(lastContent, lastCursor\)/u);
});

test('unlockKeyboard settles the touch-scroll pin instead of cancelling it', () => {
  const body = /function unlockKeyboard\(\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1];
  assert.ok(body, 'unlockKeyboard must exist');
  // clearTimeout(endTouchScrollTimer) here dropped the ONLY pending reset of
  // `touchScrolling`, freezing every later frame.
  assert.doesNotMatch(body, /clearTimeout\(endTouchScrollTimer\)/u);
  assert.match(body, /resumeLiveTailRef\?\.\(\)/u);
});
