// Source-contract tests for Terminal.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');

test('bare Escape is CLAIMED in capture and encoded by hand; no focus guards (board #20, closed)', () => {
  // The hardware-capture handler claims the bare key exactly like the
  // Ctrl/Alt combos, so the \x1b send never depends on whose keydown runs
  // first; mid-IME Escape stays with the composition.
  assert.match(source, /!event\.isComposing && event\.key === 'Escape'/u, 'claimed in onHardwareKeydown');
  assert.match(source, /&& !event\.ctrlKey && !event\.altKey && !event\.metaKey/u, 'bare only — combos keep their encoder');
  // "Esc 让当前框失去焦点" (2026-08-26 … 09-03) was chased through three rounds
  // of blur guards and one native (objc2) patch before the owner traced it to
  // a browser EXTENSION blurring inputs on Esc. All of it is gone (owner:
  // "避免我们过度修复了"); this pins the absence so it does not creep back.
  assert.doesNotMatch(source, /lastEscAt|onEscBlur|onEscKeydown|escGuardTa|hasFocus\(\)/u, 'no Escape focus guard');
  assert.match(source, /BROWSER EXTENSION/u, 'the cause is recorded where the next reader looks');
});

test('the retired unread-notification dots stay retired (2026-09-01)', () => {
  // The old per-window attention dots (unread inbox) were replaced by the
  // project room's auto-post + read cursor and the derived status dots.
  assert.doesNotMatch(source, /agent-notifications\.svelte/u);
  assert.doesNotMatch(source, /NotificationForWindow\(|ham-dot/u);
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
  assert.ok(!bar.includes('<AgentChip'), 'the session chip is retired from the lead-in');
  // The window chips keep their switch handler.
  assert.match(source, /onSwitchPane\(`\$\{w\.session\}:\$\{w\.window\}\.\$\{w\.pane\}`, w\.current_command\);/u,
    'chip quick-switch untouched');
});

test('the title never stretches the bar apart — chips follow the name (owner, 2026-08-30)', () => {
  // The phone's shared `.page-head h1 { flex: 1 1 auto }` (app.css) grows
  // titles on other pages; on the win-bar it shoved the chips to the far
  // right. The scoped win-title opts out.
  assert.match(source, /\.win-title \{ max-width: 22ch; flex: 0 1 auto; \}/u,
    'content-sized title — the chips strip takes the leftover space');
});

test('a font, theme or active change is a live option update, never an xterm rebuild (review 2026-09-03)', () => {
  // The lifecycle effect owns dispose → new Terminal → resubscribe → capture
  // → WebGL init → kbLocked. Its ONLY dependency is `target`: the body runs
  // under untrack so the font size/family, line height, theme and `active`
  // it reads synchronously cannot re-trigger it. Before this, a system
  // light/dark auto-switch mid-sentence on the phone rebuilt the terminal
  // and dropped the keyboard.
  assert.match(source, /import \{ untrack \} from 'svelte';/u);
  assert.match(
    source,
    /\$effect\(\(\) => \{\s*target;\s*return untrack\(\(\) => \{/u,
    'the lifecycle effect reads target, then untracks its whole body',
  );
  // The live effects must read their reactive inputs BEFORE any `!term`
  // early return: `term` is a plain let, so an effect that returned first
  // tracked nothing and never ran again (both were dead before this test).
  assert.match(
    source,
    /\$effect\(\(\) => \{\s*termGen;\s*const t = getTermTheme\(\);\s*if \(!term\) return;/u,
    'theme effect: termGen + theme are read, then the instance guard',
  );
  assert.match(
    source,
    /\$effect\(\(\) => \{\s*termGen;\s*const size = fontSize;\s*const family = fonts\.stack;[^\n]*\n\s*const lh = terminalPrefs\.lineHeight;\s*if \(!term\) return;/u,
    'font effect: termGen + fontSize + fonts.stack + lineHeight are read, then the instance guard',
  );
  // The handle those effects wait on is bumped once per build, inside the
  // lifecycle effect, after the instance is complete.
  assert.match(source, /let termGen = \$state\(0\);/u);
  assert.match(source, /termGen\+\+;\s*subscribe\(target\);/u, 'bumped right before the pane is subscribed');
});

test('kbLocked has exactly two writers: unlockKeyboard() and lockKeyboard()', () => {
  // terminal-keyboard.md: `endTouchScroll` and other delayed timers must never
  // lock — a timer racing a fresh unlock is how the keyboard vanished under
  // the user's finger. Every lock site (pane switch, blur timer, keyboard-shift
  // close transition, toggle close half) goes through the one function so the
  // list of callers is greppable.
  const writes = [...source.matchAll(/^\s*kbLocked = (true|false);/gmu)].map(m => m[1]);
  assert.deepEqual(writes.sort(), ['false', 'true'], 'one lock write and one unlock write');
  assert.match(source, /function lockKeyboard\(\) \{\s*kbLocked = true;\s*\}/u);
  assert.match(/function unlockKeyboard\(\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '', /kbLocked = false;/u);
  // The known callers, each labelled at the call site.
  for (const label of ['pane switch', 'blur timer', 'keyboard-shift', 'toggle: close half']) {
    assert.match(source, new RegExp(`lockKeyboard\\(\\); // ${label}`, 'u'), `caller "${label}" labelled`);
  }
});

test('double-tap is the ONE terminal-area gesture that opens the keyboard (review 2026-09-03)', () => {
  // terminal-touch.md / terminal-keyboard.md promised "double-tap → keyboard,
  // single tap does nothing" while the only caller of unlockKeyboard() was the
  // toggle button. The detector is pure (terminal-keyboard.ts) and is fed from
  // exactly one place: the clean-tap branch of onTouchEnd.
  assert.match(source, /import \{[^}]*\bcreateDoubleTapDetector\b[^}]*\} from '\.\/terminal-keyboard\.ts';/u);
  const end =/const onTouchEnd = \(e\) => \{([\s\S]*?)\n    \};/u.exec(source)?.[1] ?? '';
  assert.ok(end, 'onTouchEnd must exist');
  const down = /if \(endedMode === 'down'\) \{([\s\S]*?)\n        return;\n      \}/u.exec(end)?.[1] ?? '';
  assert.ok(down, 'the clean-tap branch must exist');
  assert.match(down, /doubleTap\.tap\(\{ x: t0\.clientX, y: t0\.clientY, t: Date\.now\(\) \}\)/u, 'fed from the clean tap');
  assert.match(down, /if \(selection\) \{\s*doubleTap\.reset\(\);/u, 'a tap that cancels a selection never starts a pair');
  // The second tap suppresses the browser's synthetic dblclick, or xterm would
  // word-select under the keyboard and onSelChange would adopt it.
  assert.match(down, /if \(e\.cancelable\) e\.preventDefault\(\);\s*(?:[^\n]*\n\s*)*?unlockKeyboard\(\); \/\/ double-tap/u);
  assert.match(source, /addEventListener\('touchend', onTouchEnd, \{ passive: false \}\)/u, 'preventDefault needs a non-passive touchend');
  // Every non-tap gesture end breaks the pair.
  assert.match(end, /if \(endedMode !== 'down'\) doubleTap\.reset\(\);/u);
  const cancel = /const onTouchCancel = \(\) => \{([\s\S]*?)\n    \};/u.exec(source)?.[1] ?? '';
  assert.match(cancel, /doubleTap\.reset\(\);/u);
  // unlockKeyboard() has exactly two callers, each labelled.
  const calls = [...source.matchAll(/unlockKeyboard\(\);(?: \/\/ ([^\n]*))?/gu)].map(m => m[1] ?? '');
  assert.deepEqual(calls.sort(), ['double-tap', 'toggle: open half']);
  // endTouchScroll stays out of the keyboard entirely.
  const ets = /function endTouchScroll\(\) \{([\s\S]*?)\n    \}/u.exec(source)?.[1] ?? '';
  assert.ok(ets, 'endTouchScroll must exist');
  assert.doesNotMatch(ets, /kbLocked|lockKeyboard|unlockKeyboard/u);
});

test('the Ctrl one-shot lives in terminal-keyboard.ts; ctrlArmed is only its mirror (review 2026-09-03)', () => {
  // The armed modifier used to be a bare boolean toggled in four places and
  // never expired: a letter typed minutes after tapping Ctrl still became a
  // control character. Arming, consumption and the 4 s expiry are one object
  // now, so the template flag has exactly one writer — the onChange mirror.
  assert.match(source, /import \{[^}]*\bcreateOneShotCtrl\b[^}]*\} from '\.\/terminal-keyboard\.ts';/u);
  assert.match(source, /const ctrlOneShot = createOneShotCtrl\(\{ onChange: \(armed\) => \{ ctrlArmed = armed; \} \}\);/u);
  const writes = [...source.matchAll(/ctrlArmed = ([^;]+);/gu)].map(m => m[1]);
  assert.deepEqual(writes, ['$state(false)', 'armed'], 'the declaration and the mirror — no direct ctrlArmed writes');
  // Both typed-input paths route through apply(): the capture-phase insertText
  // forwarder and onData (after the paste branch has returned).
  assert.match(source, /enqueueKeys\(ctrlOneShot\.apply\(e\.data\), true\);/u);
  assert.match(source, /enqueueKeys\(ctrlOneShot\.apply\(data\), true\);/u);
  // The release sites are labelled like the lock sites.
  for (const label of ['pane switch', 'blur']) {
    assert.match(source, new RegExp(`ctrlOneShot\\.disarm\\(\\); // ${label}`, 'u'), `release "${label}" labelled`);
  }
  // The armed state is visible on the bar and drops with the expiry.
  assert.match(source, /<button class="modifier" class:active=\{ctrlArmed\} aria-pressed=\{ctrlArmed\}/u);
});
