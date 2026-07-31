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

test('the keyboard is an overlay for agent TUIs, not a resize', () => {
  // The costly chain is: keyboard shrinks the viewport → the terminal box
  // shrinks → cols×rows change → tmux resizes the window → a full-screen agent
  // repaints its whole conversation. Pinning the box breaks it at step two.
  assert.match(
    source,
    /const keepRowsOnKeyboard = \$derived\(isMobile && !!detectAgent\(command\)\)/u,
    'the whitelist must be the shared agent table, not a hardcoded name',
  );
  assert.match(
    source,
    /<div class="xterm-wrap" class:keep-rows=\{keepRowsOnKeyboard\}/u,
  );
  // Pinned height + bottom anchor: the element keeps its size and the keyboard
  // covers its top rows.
  assert.match(
    source,
    /:global\(html\.keyboard-open\) \.xterm-wrap\.keep-rows \{[^}]*height: var\(--kb-locked-h, 100%\)/u,
  );
  assert.match(
    source,
    /:global\(html\.keyboard-open\) \.xterm-wrap\.keep-rows \{[^}]*bottom: 0/u,
  );
});

test('the pinned height is captured only while the keyboard is down', () => {
  // Capturing it with the keyboard up would pin the SHRUNK height, which is the
  // bug rather than the fix.
  const body = /function doResize\(\) \{([\s\S]*?)\n    \}/u.exec(source)?.[1];
  assert.ok(body, 'doResize must exist');
  assert.match(
    body,
    /if \(!document\.documentElement\.classList\.contains\('keyboard-open'\)\) \{\s*termEl\.style\.setProperty\('--kb-locked-h'/u,
  );
});

test('the keyboard-shift handler still never resizes', () => {
  // Sizing has ONE trigger, the ResizeObserver
  // (docs/design-docs/pages/terminal-sizing.md). The overlay works by keeping
  // the observed box constant, so this handler must stay out of sizing.
  const body = /const onKbShift = \(e\) => \{([\s\S]*?)\n    \};/u.exec(source)?.[1];
  assert.ok(body, 'onKbShift must exist');
  assert.doesNotMatch(body, /term\.resize\(|queuePaneResize\(|doResize\(/u);
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
