// Source-contract tests for Terminal.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');

test('WebKit Escape-cancel blur gives focus straight back to the terminal', () => {
  // WebKit's default action for Escape blurs the focused element and IGNORES
  // preventDefault — the first Esc sent \x1b but dropped focus, and every key
  // after it landed nowhere (owner, 2026-08-26, drawer AND standalone page).
  // The guard's signature is load-bearing: blur + no relatedTarget (nobody
  // took the focus) + a fresh Escape keydown.
  assert.match(source, /if \(e\.relatedTarget \|\| Date\.now\(\) - lastEscAt > 250\) return;/u);
  assert.match(source, /if \(e\.key === 'Escape'\) lastEscAt = Date\.now\(\);/u);
});

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

test('the win-bar IS the page head, and carries a title on the desktop', () => {
  // ui-unification.md "Page skeleton": one header dialect app-wide. The bar
  // had the geometry copied into its own rule and no <h1> at all, so Terminal
  // was the one page with a headerless head (owner, 2026-08-19).
  assert.match(source, /<div class="win-bar page-head">/u);
  // Geometry/border/h1 come from the shared class — not re-declared here.
  const rule = /\n  \.win-bar \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.doesNotMatch(rule, /min-height/u);
  assert.doesNotMatch(rule, /border-bottom/u);
  // The strip must not wrap: the phone page-head rule wraps head actions.
  assert.match(rule, /flex-wrap: nowrap/u);

  // Three roles, one branch: a split cell keeps a chip (its own pane picker),
  // a phone keeps a chip (it opens the session sheet), the desktop shows the
  // page title because the sidebar is already on screen beside it.
  const branch = /\{#if embedded\}([\s\S]*?)\{:else\}([\s\S]*?)\{\/if\}/u.exec(source);
  assert.ok(branch, 'the identity branch must exist');
  assert.match(branch[0], /\{:else if onOpenSessions\}/u);
  assert.match(branch[0], /<h1 class="win-title"/u);
  // A cell's chip opens the picker; it must not fall through to a null call.
  assert.match(branch[1] ?? '', /showPanePicker = !showPanePicker/u);
});

test('collapsing the switcher hides the chips, not the desktop page head', () => {
  // The collapsed state is the DEFAULT (tmux_winswitcher unset), so dropping
  // the bar entirely made a fresh desktop install the only page in the app
  // with no header (measured 2026-08-19: no .win-bar, xterm at top 0).
  assert.match(source, /\{#if isMobile\}\s*<div class="win-collapsed-anchor">/u);
  const desktopCollapsed = /\{:else\}\s*<div class="win-bar page-head">[\s\S]*?<\/div>/u.exec(source)?.[0] ?? '';
  assert.match(desktopCollapsed, /<h1 class="win-title"/u);
  assert.match(desktopCollapsed, /chevron="left"/u, 'the chip stays the expand control');
  assert.match(desktopCollapsed, /tmux_winswitcher', '1'/u);
});

test('the phone bar leads with hamburger + name, chips unchanged (board #19)', () => {
  // Chat and Board's lead-in ("三个横线 + 项目名", owner 2026-08-30): the
  // hamburger opens the session drawer, the h1 names the session, and the
  // window chips after it keep their quick-switch behavior.
  const bar = source.slice(source.indexOf('{:else if onOpenSessions}'), source.indexOf('{:else}', source.indexOf('{:else if onOpenSessions}')));
  assert.match(bar, /class="icon-btn ham"[\s\S]{0,120}?onOpenSessions\(\)/u, 'the hamburger opens the drawer');
  assert.match(bar, /<Icon name="menu"/u, 'three lines, like Chat and Board');
  assert.match(bar, /<h1 class="win-title" title=\{session\}>\{session\}<\/h1>/u, 'the name is the title');
  assert.match(bar, /ham-dot/u, 'the chip\u2019s attention cue survives on the hamburger');
  assert.ok(!bar.includes('<AgentChip'), 'the session chip is retired from the lead-in');
  // The window chips keep their switch handler.
  assert.match(source, /onSwitchPane\(`\$\{w\.session\}:\$\{w\.window\}\.\$\{w\.pane\}`, w\.current_command\);/u,
    'chip quick-switch untouched');
});
