// Source-contract tests for Terminal.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');

test('Escape on desktop WebKit: the guard watches PAGE focus, not just the DOM (round four)', () => {
  // What Escape drops in the macOS app is the webview's first-responder
  // status, not the DOM focus: AppKit turns the key into `cancelOperation:`,
  // WebKit hands the key to the input method before the DOM sees it (so
  // preventDefault cannot stop the command), the command walks the native
  // responder chain, and WebKit fires a relatedTarget-less blur while
  // document.activeElement STAYS the textarea. Three rounds of `term.focus()`
  // were therefore no-ops (board #20 + reopen; owner 2026-09-03 "之前修了好几
  // 次都没修好"). The guard now reads `document.hasFocus()` — false means the
  // page lost native focus whatever the DOM says — and reclaims through
  // Tauri's Webview.setFocus() (makeFirstResponder) before the DOM focus.
  assert.match(source, /const pageFocused = document\.hasFocus\(\);/u, 'native focus is the signal');
  assert.match(source, /getCurrentWebview\(\)\.setFocus\(\)/u, 'reclaims native focus through Tauri');
  assert.match(source, /if \(!pageFocused && nativeFocus\) nativeFocus\(\)\.then\(domFocus, domFocus\);/u, 'native first, DOM after');
  // Both signatures reclaim: the DOM-level blur (nobody took the focus) and
  // the page-level window blur; the watch also polls twice after the claim.
  assert.match(source, /if \(e\.relatedTarget \|\| !escRecent\(\)\) return;/u, 'textarea blur signature');
  assert.match(source, /window\.addEventListener\('blur', onEscWindowBlur\)/u, 'page-level blur');
  assert.match(source, /setTimeout\(escCheck\('60ms'\), 60\), setTimeout\(escCheck\('300ms'\), 300\)/u, 'two checks after the claim');
  // Bounded per Escape: a genuine window switch right after an Escape must
  // not turn into a focus tug-of-war.
  assert.match(source, /if \(!escRecent\(\) \|\| escReclaims >= ESC_RECLAIM_MAX\) return;/u);
  // Evidence goes to the debug panel, keyCode included — 229 is WebKit's
  // "the input method answered this key" marker, the case preventDefault
  // cannot reach.
  assert.match(source, /kb: esc claimed keyCode=\$\{event\.keyCode\}/u);
  // Listeners and timers are torn down with the terminal.
  assert.match(source, /window\.removeEventListener\('blur', onEscWindowBlur\)/u);
  assert.match(source, /for \(const t of escWatchTimers\) clearTimeout\(t\);/u);
});

test('bare Escape is CLAIMED in capture and encoded by hand (board #20)', () => {
  // Whether the \x1b reached the pane used to depend on whose keydown ran
  // before WebKit's un-preventable Escape blur — "esc 没有发送到后端，而是
  // 让当前框失去焦点" (owner, 2026-08-30). The hardware-capture handler
  // claims the bare key exactly like the Ctrl/Alt combos, so the send never
  // depends on where focus lands; mid-IME Escape stays with the composition.
  assert.match(source, /!event\.isComposing && event\.key === 'Escape'/u, 'claimed in onHardwareKeydown');
  assert.match(source, /&& !event\.ctrlKey && !event\.altKey && !event\.metaKey/u, 'bare only — combos keep their encoder');
  // The claim's stopImmediatePropagation kills any later same-node keydown
  // listener, so the claim itself stamps the time the guard reads and arms
  // the focus watch.
  assert.match(source, /if \(bareEsc\) \{\n\s+lastEscAt = Date\.now\(\);/u, 'the claim feeds the guard');
  assert.match(source, /armEscFocusWatch\?\.\(\);/u, 'the claim arms the watch');
});

test('the native half of the Escape fix lives in the Tauri shell (macOS)', async () => {
  // The command is answered ON the webview so the responder-chain walk never
  // starts: a no-op `cancelOperation:` added to the WKWebView's class.
  const rs = await readFile(new URL('../../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  assert.match(rs, /mod escape_stays_in_webview/u);
  assert.match(rs, /sel!\(cancelOperation:\)/u);
  assert.match(rs, /win\.with_webview\(\|pw\| \{\n\s+let added = unsafe \{ escape_stays_in_webview::install\(pw\.inner\(\)\) \};/u);
  const cargo = await readFile(new URL('../../../src-tauri/Cargo.toml', import.meta.url), 'utf8');
  assert.match(cargo, /\[target\.'cfg\(target_os = "macos"\)'\.dependencies\]\nobjc2 = \{ version = "0\.6", optional = true \}/u);
  assert.match(cargo, /"dep:objc2"/u, 'gated behind the gui feature like the rest of the shell');
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
