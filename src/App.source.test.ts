// Source-contract tests for App.svelte (see docs/conventions/testing.md):
// component wiring that node can't execute is pinned by matching the source.
// If one of these fails after an intentional change, update the assertion —
// the point is that the change must be INTENTIONAL.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./App.svelte', import.meta.url), 'utf8');

test('every authenticated connection path refreshes agent notifications', () => {
  const reconnectSuccess = source.match(/function onReconnectSuccess[\s\S]*?\n  \}/u)?.[0] || '';
  const manualSuccess = source.match(/function onConnected[\s\S]*?\n  \}/u)?.[0] || '';
  const optimizedConnection = source.match(/async function optimizeConnection[\s\S]*?\n  \}/u)?.[0] || '';
  const automaticConnection = source.match(/\$effect\(\(\) => \{\n    if \(autoConnectAttempted[\s\S]*?\n  \}\);/u)?.[0] || '';

  assert.match(reconnectSuccess, /syncAgentNotifications\(\)/u);
  assert.match(manualSuccess, /syncAgentNotifications\(\)/u);
  assert.match(optimizedConnection, /syncAgentNotifications\(\)/u);
  assert.match(automaticConnection, /syncAgentNotifications\(\)/u);
});

test('Terminal navigation and page layer exist without an active target', () => {
  // The Sessions tab was retired into Terminal (2026-08-18): the list is
  // Terminal's sidebar, so no tab starts the list and terminal is always there.
  assert.doesNotMatch(source, /const t = \['sessions'\]/u);
  assert.match(source, /t\.push\('terminal'\)/u);
  assert.doesNotMatch(source, /switchTab\('sessions'\)/u);
  assert.match(
    source,
    /<button tabindex="-1" class:active=\{page === 'terminal'[\s\S]*?\{t\('terminal'\)\}[\s\S]*?<\/button>/u,
  );
  assert.match(
    source,
    /<div class="page-layer term-page" class:hidden=\{page !== 'terminal'\}>/u,
  );
  // The empty state keeps the page HEADER (ui-unification: every page's head
  // survives an empty detail pane — Chat, Agents, Settings all do), so the
  // else-branch opens with `.page-head` and the empty block follows it.
  assert.match(source, /\{:else\}[\s\S]{0,400}?<div class="page-head">\s*<h1>\{t\('terminal'\)\}<\/h1>/u);
  assert.match(source, /<div class="terminal-empty">/u);
});

test('the Terminal page uses the shared sidebar geometry', () => {
  // ui-unification.md §1: wherever a sidebar exists it is THE sidebar — one
  // width variable, one resize affordance. Terminal was the last holdout
  // (a hardcoded 280px column with no handle).
  assert.match(source, /\.page-layer\.term-page \{[^}]*grid-template-columns: var\(--sidebar-w\)/u);
  const aside = source.match(/<aside class="term-side side-sheet"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /<SideHandle \/>/u, 'the sidebar carries the shared handle');
  assert.doesNotMatch(source, /grid-template-columns: 280px/u);
});

test('the session list lives inside the Terminal page, sheeted on a phone', () => {
  // One Sessions instance, mounted as the terminal page's sidebar; on a phone
  // it slides over and a pick closes it.
  const mounts = source.match(/<Sessions\b/gu) ?? [];
  assert.equal(mounts.length, 1, 'exactly one Sessions mount');
  // side-sheet is the SHARED drawer dialect (app.css) — one geometry for
  // Chat/Terminal/Board (owner, 2026-08-30).
  const aside = source.match(/<aside class="term-side side-sheet"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /class:sheet=\{layout\.isTouchDevice\}/u);
  assert.match(aside, /class:open=\{layout\.isTouchDevice && sessListOpen\}/u);
  assert.match(aside, /onPick=\{\(\) => sessListOpen = false\}/u);
  // The terminal's session chip opens that same sheet on a phone.
  assert.match(source, /onOpenSessions=\{layout\.isTouchDevice \? \(\) => sessListOpen = true : null\}/u);
  // A phone back gesture closes the sheet instead of leaving the app.
  assert.match(source, /page === 'terminal' && sessListOpen.*sessListOpen = false/u);
});

// ── The desktop rail's icon order is the user's (board #6) ──────────────────
// The pure rules (normalize / reorder / drop geometry) are tested in
// lib/app/nav-order.test.ts. What node cannot execute — which element carries
// the gesture, which one must NOT, and that the phone is untouched — is pinned
// here by matching the source.
const rail = source.match(/<nav\n?\s*class="rail"[\s\S]*?<\/nav>/u)?.[0] ?? '';
const tabbar = source.match(/<nav class="tabbar">[\s\S]*?<\/nav>/u)?.[0] ?? '';

test('the rail renders the SAVED order, not a hardcoded sequence', () => {
  assert.ok(rail, 'the desktop rail must still be there');
  assert.match(rail, /\{#each railSlots as slot \(slot\)\}/u, 'the icons come from railSlots');
  assert.match(source, /let railOrder = \$state\(parseRailOrder\(localStorage\.getItem\(RAIL_ORDER_KEY\)\)\)/u,
    'the order is restored from localStorage through the untrusted-read helper');
  assert.match(source, /let railSlots = \$derived\(visibleRailSlots\(/u);

  // Availability filters the RENDERING only: hub/board/agents need the bus, and
  // the stored order keeps them so a toggled-off page returns where it was put.
  assert.match(source, /p === 'hub' \|\| p === 'board' \|\| p === 'agents' \? hubEligible : true/u);
  assert.doesNotMatch(rail, /\{#if hubEligible\}/u, 'the each + availability predicate replaced the per-icon gates');

  // No icon may be hardcoded back into the rail: that is how one of them stops
  // following the user's order without anything failing.
  assert.doesNotMatch(rail, /switchTab\('/u, 'a rail icon activates through railActivate(slot)');
  assert.match(rail, /onclick=\{\(\) => railActivate\(slot\)\}/u);
  assert.match(source, /function railActivate\(slot\)[\s\S]*?slot === 'prefs'\) togglePrefs\(\)[\s\S]*?switchTab\(slot\)/u,
    'the gear is a rail page icon like the others; it just toggles instead of switching');
});

test('every page icon is draggable and the brand is not', () => {
  // Consistency is the rule the rail is judged by: every .rail-btn is a page
  // icon, so every one of them carries the gesture — a single icon that refuses
  // to move is unexplainable. The brand is the app's mark, not a page.
  const buttons = rail.match(/<button[\s\S]*?<\/button>/gu) ?? [];
  assert.equal(buttons.length, 1, 'one templated button, not six copies');
  assert.match(String(buttons[0]), /data-rail-slot=\{slot\}/u, 'the drop geometry is read off these');
  assert.match(String(buttons[0]), /onpointerdown=\{\(e\) => railPointerDown\(e, slot\)\}/u);
  assert.match(String(buttons[0]), /class:dragging=\{railDrag\?\.slot === slot\}/u);

  const brand = rail.match(/<img class="rail-brand"[^>]*>/u)?.[0] ?? '';
  assert.ok(brand, 'the brand stays at the top of the rail');
  assert.doesNotMatch(brand, /data-rail-slot|onpointerdown/u, 'the brand is not a drag handle');
  assert.match(brand, /draggable="false"/u, 'and its native image drag must not fight the gesture');

  // The gap is a member of the order (so a drag can cross it) but never a handle.
  const spacer = rail.match(/<div class="rail-spacer"[^>]*>/u)?.[0] ?? '';
  assert.match(spacer, /data-rail-slot=\{slot\}/u, 'it is a drop anchor');
  assert.doesNotMatch(spacer, /onpointerdown/u, 'it is not a drag handle');
});

test('a plain click still switches pages; a drag never does', () => {
  // The whole risk of putting a gesture on a navigation button: the press that
  // was meant to switch pages must still switch pages.
  assert.match(source, /if \(Math\.abs\(dy\) < RAIL_DRAG_THRESHOLD\) return;/u,
    'the drag only begins after the threshold, so jitter on a click is not a reorder');
  assert.match(source, /function railActivate\(slot\) \{\s*if \(railClickGuard\) \{ railClickGuard = false; return; \}/u,
    'the click that follows a drag is swallowed');
  assert.match(source, /function railPointerUp\(\)[\s\S]*?railClickGuard = true;/u, 'a committed drag arms the guard');
  assert.match(source, /function railCancelDrag\(\)[\s\S]*?if \(railDrag\) railClickGuard = true;/u,
    'an ABANDONED drag arms it too — the pointer travelled, so it was not a pick');
  assert.match(source, /railClickGuard = false;\s*railPress = \{ slot/u,
    'every fresh press clears the guard, so a stale one cannot swallow the next click');
  // Capture at pointerdown: a fast drag leaving the 34px button must keep
  // reporting to the rail rather than to whatever it passes over.
  assert.match(source, /setPointerCapture\(e\.pointerId\)/u);
});

test('a drag shows what moves and where it lands, and can be abandoned', () => {
  assert.match(rail, /class:reordering=\{!!railDrag\}/u);
  assert.match(rail, /style:transform=\{railDrag\?\.slot === slot \? `translateY\(\$\{railDrag\.dy\}px\)` : null\}/u,
    'the carried icon follows the pointer by transform — it must not reflow the rail it is measuring');
  assert.match(rail, /<div\s*class="rail-drop"/u, 'the insertion line');
  assert.match(rail, /railDropOffset\(railDrag\.rects, railDrag\.idx\)/u);
  const style = source.match(/<style>[\s\S]*<\/style>/u)?.[0] ?? '';
  assert.match(style, /\.rail-drop \{[^}]*position: absolute/u, 'absolute, so opening it cannot reflow the snapshotted rects');
  assert.match(style, /\.rail\.reordering \{[^}]*user-select: none/u);
  assert.match(style, /\.rail\.reordering \.rail-btn:not\(\.dragging\):hover/u,
    'hover is suppressed mid-drag — under the cursor it reads as a second selection');
  // Escape / resize / a lost capture all end the drag: a stranded carried icon
  // with no release is the worst failure this gesture has.
  assert.match(source, /if \(e\.key === 'Escape'\) \{ e\.preventDefault\(\); railCancelDrag\(\); \}/u);
  assert.match(source, /window\.addEventListener\('resize', railCancelDrag\)/u);
  assert.match(source, /window\.addEventListener\('pointerup', railPointerUp, true\)/u);
});

test('the order persists, and the DEFAULT order stores nothing', () => {
  assert.match(
    source,
    /function setRailOrder\(next\) \{[\s\S]*?railOrderToStore\(next\)[\s\S]*?localStorage\.setItem\(RAIL_ORDER_KEY, raw\)[\s\S]*?localStorage\.removeItem\(RAIL_ORDER_KEY\)/u,
    'a null from railOrderToStore REMOVES the key rather than writing the shipped order back',
  );
  assert.match(source, /setRailOrder\(railDropAt\(railOrder, railDrag\.slot, railDrag\.rects, railDrag\.idx\)\)/u,
    'the drop commits through the pure path, with the SAME index the insertion line was drawn from');
});

test('the phone keeps the shipped order — the gesture is desktop-only', () => {
  // "桌面版左侧" is the whole scope. The bottom bar is thumb geography, and
  // press-and-drag there is the terminal's scroll.
  assert.ok(tabbar, 'the mobile tab bar must still be there');
  assert.doesNotMatch(tabbar, /railSlots|railOrder|data-rail-slot|onpointerdown/u,
    'the tab bar must not learn the rail order or the gesture');
  assert.match(tabbar, /onclick=\{\(\) => switchTab\('terminal'\)\}/u, 'it stays hardcoded in the shipped order');
  // Keyboard previous/next page follows the RAIL on desktop (one order, not a
  // hidden second one) while a touch layout keeps the shipped sequence.
  assert.match(
    source,
    /const tabs = \$derived\(\(\) => \{[\s\S]*?if \(!layout\.isTouchDevice\) return railSlots\.filter\(\(s\) => s !== RAIL_GAP && s !== 'prefs'\);/u,
  );
  assert.match(source, /const tabs = \$derived[\s\S]*?if \(hubEligible\) t\.push\('hub'\);[\s\S]*?t\.push\('terminal'\);/u,
    'the touch branch is unchanged');
});

// ── Agents is a Settings category on touch, a rail page on the desktop (#10) ──
test('the phone’s tab bar has no Agents icon, and the swipe does not stop there', () => {
  // Owner, 2026-08-29: "不用单独在底下一行展示了，现在看着有点多底下的标签".
  assert.ok(tabbar, 'the tab bar must still be there');
  assert.doesNotMatch(tabbar, /switchTab\('agents'\)/u, 'no Agents icon');
  assert.doesNotMatch(tabbar, /name="bot"/u);
  assert.match(tabbar, /onclick=\{togglePrefs\}/u, 'the gear is how you reach it now');
  // A swipe that reaches a page with no icon is a page you cannot get back to.
  const touchTabs = source.match(/const t = \[\];[\s\S]*?return t;/u)?.[0] ?? '';
  assert.ok(touchTabs, 'the touch tab sequence must still be there');
  assert.doesNotMatch(touchTabs, /'agents'/u, 'agents is not a swipe stop on touch');
  assert.match(touchTabs, /t\.push\('board'\)[\s\S]*?t\.push\('files'\)[\s\S]*?t\.push\('terminal'\)/u,
    'the owner-set order: chat, board, files, terminal (2026-08-29)');
});

test('the desktop rail keeps Agents as a draggable page icon', () => {
  // The whole point of scoping this to touch: nothing about the rail changes.
  assert.match(source, /agents:\s*\{ icon: 'bot',\s*label: 'agentsTitle' \}/u, 'still a rail item');
  assert.match(rail, /\{#each railSlots as slot \(slot\)\}/u, 'still the user’s draggable order');
  assert.match(source, /\{#if hubEligible && !agentsLivesInSettings\(layout\.isTouchDevice\)\}\s*<div class="page-layer" class:hidden=\{page !== 'agents'\}>/u,
    'and still a page layer — but not mounted on touch, where Settings owns the only instance');
});

test('every route into the agent config goes through ONE device-aware entry', () => {
  // Four places had to agree or it becomes unreachable in one of them; they all
  // call openAgentsConfig, which asks nav-state where Agents lives.
  assert.match(
    source,
    /function openAgentsConfig\(name = null\) \{[\s\S]*?if \(agentsLivesInSettings\(layout\.isTouchDevice\)\) \{[\s\S]*?prefsOpenReq = \{ tab: 'agents'[\s\S]*?if \(page !== 'prefs'\) togglePrefs\(\);[\s\S]*?\} else \{\s*switchTab\('agents'\);/u,
  );
  // The Hub's "configure agent" item: Settings on a phone, the page on a desktop.
  assert.match(source, /openAgentConfig=\{\(name\) => openAgentsConfig\(name\)\}/u);
  assert.doesNotMatch(source, /openAgentConfig=\{\(name\) => \{[^}]*switchTab\('agents'\)/u,
    'it must not switch straight to a page that does not exist on touch');
});

test('a saved `agents` page never strands a phone on an unreachable layer', () => {
  assert.match(source, /const nav = restoreNav\(s\.page, layout\.isTouchDevice\);\s*page = nav\.page;/u,
    'restore asks nav-state, which redirects agents → Settings on touch');
  assert.match(source, /if \(nav\.settingsTab\) prefsOpenReq = \{ tab: nav\.settingsTab, n: \+\+prefsOpenSeq \};/u);
  // The redirect effect is one of the writers, so the sequence must NOT be read
  // back off the state it writes — that effect would depend on itself.
  assert.match(source, /let prefsOpenSeq = 0;/u);
  assert.doesNotMatch(source, /prefsOpenReq\?\.n/u);
  // And ANY other route that sets the page — a deep link, an older build, the
  // bus probe — is corrected by the same rule in the redirect effect.
  assert.match(
    source,
    /if \(page === 'agents' && agentsLivesInSettings\(layout\.isTouchDevice\)\) \{\s*page = 'prefs';\s*prefsOpenReq = \{ tab: 'agents', n: \+\+prefsOpenSeq \};/u,
  );
});

test('Settings only offers the category when there is a bus and no Agents page', () => {
  // hubEligible is the desktop-server gate; agentsLivesInSettings is the device.
  assert.match(source, /showAgents=\{hubEligible && agentsLivesInSettings\(layout\.isTouchDevice\)\}/u);
  assert.match(source, /agentsEditRequest=\{agentsEditReq\}/u, 'the Hub’s jump reaches the embedded editor');
  assert.match(source, /openRequest=\{prefsOpenReq\}/u);
});

test('a jump from the chat returns THERE on back, and deliberate navigation clears it (2026-08-29)', () => {
  // The one-deep return slot: set by the chat's cross-page jumps, cleared by
  // any real tab switch, consumed at the target page's back FLOOR.
  assert.match(source, /jumpedFrom = null; \/\/ deliberate navigation stands the return slot down/u,
    'switchTab clears the slot');
  assert.match(source, /if \(from === 'hub'\) jumpedFrom = 'hub';/u, 'opening a pane from the chat remembers');
  assert.match(source, /switchTab\('files'\); jumpedFrom = 'hub';/u, 'the files jump remembers (after the switch cleared)');
  assert.match(source, /switchTab\('board'\); jumpedFrom = 'hub';/u, 'the board jump too');
  for (const p of ['files', 'board', 'terminal']) {
    assert.match(source, new RegExp(`if \\(page === '${p}' && jumpedFrom\\) \\{ switchTab\\(jumpedFrom\\); return; \\}`, 'u'),
      `${p}'s floor prefers the return slot`);
  }
  // Order matters: the slot fires only at the FLOOR — after the page's own
  // onGoBack chain (and the terminal's session sheet) had their turn.
  const idxGoBack = source.indexOf("if (page === 'board' && boardGoBack && boardGoBack())");
  const idxSlot = source.indexOf("if (page === 'board' && jumpedFrom)");
  assert.ok(idxGoBack >= 0 && idxGoBack < idxSlot, 'peel first, return second');
});
