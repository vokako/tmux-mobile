// Source-contract tests for App.svelte (see docs/conventions/testing.md):
// component wiring that node can't execute is pinned by matching the source.
// If one of these fails after an intentional change, update the assertion —
// the point is that the change must be INTENTIONAL.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./App.svelte', import.meta.url), 'utf8');

test('the retired unread-notification store stays retired (2026-09-01)', () => {
  // The old agent-notification dots (unread.json inbox + per-window attention
  // marks) were replaced by the project room's auto-post + read cursor and the
  // derived status dots — two unread ledgers never agreed (owner: "原来我用的
  // 感觉不是很好用"). App must not resurrect the store.
  assert.doesNotMatch(source, /agent-notifications\.svelte/u);
  assert.doesNotMatch(source, /syncAgentNotifications|markWindowRead/u);
});

test('Terminal navigation and page layer exist without an active target', () => {
  // The Sessions tab was retired into Terminal (2026-08-18): the list is
  // Terminal's sidebar, so no tab starts the list and terminal is always there.
  assert.doesNotMatch(source, /const t = \['sessions'\]/u);
  assert.match(source, /t\.push\('terminal'\)/u);
  assert.doesNotMatch(source, /switchTab\('sessions'\)/u);
  assert.match(
    source,
    /<button class:active=\{page === 'terminal'[\s\S]*?\{t\('terminal'\)\}[\s\S]*?<\/button>/u,
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
  // The empty state's "Sessions" button goes through the one opener: a touch
  // layout lifts the drawer, the desktop hands focus to the list that is
  // already on screen. Setting `sessListOpen` alone did nothing on the
  // desktop — no desktop rule reads it (review, 2026-09-03).
  assert.match(source, /<div class="terminal-empty">[\s\S]*?<button class="chip-btn" onclick=\{openSessionsList\}>/u);
  const opener = /function openSessionsList\(\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(opener, /if \(layout\.isTouchDevice\) \{ sessListOpen = true; return; \}/u);
  assert.match(opener, /termSideEl\?\.querySelector\('\.sessions \.content \[tabindex="0"\], \.sessions \.content button'\)\?\.focus\(\)/u,
    'desktop: focus the list itself, never the SideHandle separator');
  assert.match(source, /<aside class="term-side"[^>]*bind:this=\{termSideEl\}>/u);
});

test('the Terminal page uses the shared sidebar geometry', () => {
  // ui-unification.md §1: wherever a sidebar exists it is THE sidebar — one
  // width variable, one resize affordance. Terminal was the last holdout
  // (a hardcoded 280px column with no handle).
  assert.match(source, /\.page-layer\.term-page \{[^}]*grid-template-columns: var\(--sidebar-w\)/u);
  const aside = source.match(/<aside class="term-side"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /<SideHandle \/>/u, 'the sidebar carries the shared handle');
  assert.doesNotMatch(source, /grid-template-columns: 280px/u);
});

test('the session list lives inside the Terminal page, sheeted on a phone', () => {
  // One Sessions instance, mounted as the terminal page's sidebar; on a phone
  // it slides over and a pick closes it.
  const mounts = source.match(/<Sessions\b/gu) ?? [];
  assert.equal(mounts.length, 1, 'exactly one Sessions mount');
  // side-sheet is the SHARED drawer dialect (app.css), CLASS-driven: applied
  // exactly when this page's own compact condition holds — touch AND narrow,
  // the old media gate (a media-gated sheet disagreed with the Hub's wider
  // compact and stacked the sidebar into the page; owner, 2026-08-30).
  const aside = source.match(/<aside class="term-side"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /class:side-sheet=\{layout\.isTouchDevice && narrowVp\}/u);
  assert.match(aside, /class:sheet=\{layout\.isTouchDevice\}/u);
  assert.match(aside, /class:open=\{layout\.isTouchDevice && narrowVp && sessListOpen\}/u);
  assert.match(aside, /onPick=\{\(\) => sessListOpen = false\}/u);
  // The terminal's session chip opens that same sheet on a phone.
  assert.match(source, /onOpenSessions=\{layout\.isTouchDevice \? \(\) => sessListOpen = true : null\}/u);
  // The phone Back gesture uses this sheet as Terminal's FLOOR (board #58),
  // matching Chat/Board: a bare Terminal lifts it; once open it falls through
  // and stays open, so Back can never close→open cycle. A Chat jump returns
  // to Chat before the ordinary floor lift.
  const jumpAt = source.indexOf("if (page === 'terminal' && jumpedFrom)");
  const liftAt = source.indexOf("if (page === 'terminal' && layout.isTouchDevice && narrowVp && !sessListOpen)");
  assert.ok(jumpAt >= 0 && liftAt > jumpAt, 'a Chat return slot outranks the Terminal floor');
  assert.match(source, /if \(page === 'terminal' && layout\.isTouchDevice && narrowVp && !sessListOpen\) \{\s*sessListOpen = true;\s*navPush\(\);\s*return;\s*\}/u,
    'bare compact Terminal lifts the session drawer and replenishes history');
  assert.ok(!/page === 'terminal' && sessListOpen[^\n]*sessListOpen = false/u.test(source),
    'an open floor never peels closed — no open/close loop');
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
  // Consistency is the rule the rail is judged by: every page icon carries
  // the gesture — a single icon that refuses to move is unexplainable. The
  // brand is the app's mark, not a page; the server switcher (board #55) is
  // the one CONTROL among the buttons — no slot, no drag, only a popover.
  const buttons = rail.match(/<button[\s\S]*?<\/button>/gu) ?? [];
  assert.equal(buttons.length, 2, 'the templated page button and the server-switcher control');
  const pageBtn = buttons.find((b) => b.includes('data-rail-slot={slot}')) ?? '';
  assert.ok(pageBtn, 'the drop geometry is read off the page button');
  assert.match(String(pageBtn), /onpointerdown=\{\(e\) => railPointerDown\(e, slot\)\}/u);
  assert.match(String(pageBtn), /class:dragging=\{railDrag\?\.slot === slot\}/u);
  const serverBtn = buttons.find((b) => b.includes('rail-server')) ?? '';
  assert.ok(serverBtn, 'the switcher is the other button');
  assert.doesNotMatch(String(serverBtn), /data-rail-slot|onpointerdown/u,
    'a control: never a drag handle, never a drop anchor');

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
  assert.match(rail, /<div\s*class="rail-drop appear"/u, 'the insertion line — it fades in (motion.md), never grows');
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

test('board back on a phone lifts the project drawer, never the terminal (board #47)', () => {
  // The bottom-bar entry's floor is the DRAWER (Board's own chain lifts it);
  // App must not add a terminal fallback below it — a fall-through re-pushes,
  // exactly like Hub. Only the chat's jump (the return slot) leaves the page.
  assert.ok(!/if \(page === 'board'\) \{ page = 'terminal'; return; \}/u.test(source),
    'the board→terminal floor stays retired');
  // Files' floor is the SAME rule (owner, on the issue): its own chain climbs
  // parent directories; App adds no terminal fallback below it either.
  assert.ok(!/if \(page === 'files'\) \{ page = 'terminal'; return; \}/u.test(source),
    'the files→terminal floor stays retired');
  assert.match(source, /<Files [^\n]*jumped=\{!!jumpedFrom\}/u, 'the files page instance gets the return-slot gate');
  // Board is told whether a return slot exists, so its drawer lift can stand
  // aside and let back fall through to the conversation.
  assert.match(source, /<Board [^\n]*jumped=\{!!jumpedFrom\}/u, 'the return slot reaches the drawer-lift gate');
});

test('the multi-server registry wires migrate → deep-link → boot, in that order (board #55)', () => {
  // Migration must run BEFORE the deep-link consumer: on a pre-registry
  // client a link would otherwise create the registry via upsert, turn
  // migrateServers into a no-op, and silently drop the current user +
  // history. The consumer then upserts the linked server by address.
  const idxMigrate = source.indexOf('migrateServers(localStorage)');
  const idxConsume = source.indexOf('consumeConnectUrlParams();');
  assert.ok(idxMigrate >= 0, 'boot migrates the single-server keys');
  assert.ok(idxMigrate < idxConsume, 'migrate BEFORE the deep-link consumer');
  assert.match(source, /activateConnected\(localStorage, \{ address: a2, token: token \|\| ''/u,
    'a deep-linked server joins the registry AND activates pre-boot (parks the old current)');
});

test('the rail server switcher sits above the configure group and only places (board #55)', () => {
  // "右下角agent上边": the entry rides the RAIL_GAP branch — glued to the top
  // of the bottom group, above agents in the shipped order — and is a
  // CONTROL: no data-rail-slot, so it can never become a drag target.
  const gapBranch = source.match(/\{#if slot === RAIL_GAP\}[\s\S]*?\{:else\}/u)?.[0] ?? '';
  assert.match(gapBranch, /class="rail-btn rail-server"/u, 'the switcher lives in the gap branch');
  assert.ok(!/rail-server[^>]*data-rail-slot/u.test(source), 'a control, not a draggable slot');
  assert.match(source, /onclick=\{\(e\) => toggleServerMenu\(e\)\}/u, 'it opens the registry popover');
});

test('a server switch fully drops the old socket, applies the plan, and reboots (board #55)', () => {
  // Order is the contract: cancel any reconnect loop (it re-reads
  // tmux_address and would race the storage writes), close the socket, THEN
  // applySwitch (park/restore per-server state) and reload — the boot path
  // is the one way up against a server, so nothing in-memory can leak across.
  const fn = source.match(/function doServerSwitch\(id\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  const iCancel = fn.indexOf('reconnectMachine.cancel()');
  const iDisc = fn.indexOf('disconnect()');
  const iApply = fn.indexOf('applySwitch(localStorage, id)');
  const iReload = fn.indexOf('location.reload()');
  assert.ok(iCancel >= 0 && iCancel < iDisc && iDisc < iApply && iApply < iReload,
    'cancel → disconnect → applySwitch → reload');
});

test('failover success RECORDS by machine identity — and never moves CURRENT (board #55)', () => {
  // onReconnectSuccess proved a DIFFERENT address of the SAME machine —
  // recordServer merges it into the one entry instead of growing a second
  // "server" (the lead-review invariant: multi-server must not break the
  // multi-address semantics), and recording is NOT activating: CURRENT
  // stays put, no reload, the failover semantics own the socket swap.
  const fn = source.match(/function onReconnectSuccess\(useAddr, primaryAddr\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  assert.match(fn, /recordServer\(localStorage, \{[\s\S]*?address: useAddr/u, 'the active address follows the connect');
  assert.match(fn, /machineId: mid/u, 'merged by machine identity, never by address alone');
  assert.ok(!fn.includes('activateConnected') && !fn.includes('applySwitch'),
    'failover records; it never activates');
});

test('a Settings connect to a DIFFERENT server reboots before onConnected (board #55)', async () => {
  // The lead blocker: connect() swaps the socket but Hub room caches,
  // mounted terminals and Files cwds are old-server memory, and the old
  // tmux_state was never parked. The form therefore asks activateConnected
  // AFTER auth and BEFORE onConnected: same server → proceed in place,
  // different server → location.reload() through the one boot path (the
  // reload return also skips onConnected — no flash of the old world).
  const settings = await readFile(new URL('./lib/app/Settings.svelte', import.meta.url), 'utf8');
  const fn = settings.match(/async function doConnect\(\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  const iAct = fn.indexOf('activateConnected(localStorage');
  const iReload = fn.indexOf('if (act.reload) { location.reload(); return; }');
  const iDone = fn.indexOf('onConnected()');
  assert.ok(iAct >= 0 && iReload > iAct && iDone > iReload,
    'activate → maybe-reload-and-return → only then onConnected');
  assert.match(fn, /machineId: mid/u, 'the learned machine identity rides the activation');
});

test('the boot auto-connect RECORDS the machine identity, never activates (board #55)', () => {
  // Most sessions connect through the boot path; without this stamp an
  // entry never learns its machineId and a later connect to an alternate
  // address of the SAME machine reads as a new server (live Chromium
  // finding). Record only — the entry booted as current.
  const fn = source.match(/connect\(addr, token\)\.then\(\(\) => \{[\s\S]*?\n    \}\)/u)?.[0] ?? '';
  assert.match(fn, /recordServer\(localStorage, \{[\s\S]*?machineId: mid/u, 'boot stamps the identity');
  assert.ok(!fn.includes('activateConnected'), 'boot never activates — it IS the current server');
});

test('doConnect never writes the live tmux_machine_id — activateConnected owns it (board #55)', async () => {
  // Lead blocker #2: the form pre-wrote the NEW machine's id into the live
  // key before activating, so parkAndPoint filed it under the OLD server's
  // parking slot. The map (tmux_machines, keyed by machineId) stays the
  // form's to update; the live key belongs to the activation.
  const settings = await readFile(new URL('./lib/app/Settings.svelte', import.meta.url), 'utf8');
  const fn = settings.match(/async function doConnect\(\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  assert.ok(!fn.includes("setItem('tmux_machine_id'"), 'no live machine-id pre-write in the form');
  assert.match(fn, /setItem\('tmux_machines'/u, 'the identity MAP update stays');
  // The boot path may write it: its server IS the current entry, no park to poison.
  const boot = source.match(/connect\(addr, token\)\.then\(\(\) => \{[\s\S]*?\n    \}\)/u)?.[0] ?? '';
  assert.match(boot, /recordServer\(localStorage[\s\S]*?setItem\('tmux_machine_id', mid\)/u,
    'boot stamps the live key AFTER recording — it is the current server');
});

test('removing a saved server goes through the shared ConfirmDialog (board #55)', () => {
  // Lead blocker #3: the × called removeServer directly — a destructive path
  // with no confirmation, in a dense popover. Now the × only CAPTURES the
  // row's identity, the shared dialog asks, and the confirm consumes the
  // captured id — never a re-read of the menu row or the current id, so a
  // menu that switched or closed cannot retarget a delayed confirm.
  assert.match(source, /onclick=\{\(\) => serverRemoveAsk\(s\)\}/u, 'the × only asks');
  assert.ok(!/onclick=\{\(\) => serverRemoveRow/u.test(source), 'the direct-remove handler is retired');
  assert.match(source, /pendingServerRemove = \{ id: s\.id, name: s\.name, address: s\.address \}/u,
    'identity captured at click time');
  const fn = source.match(/function serverRemoveConfirm\(\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  assert.match(fn, /const victim = pendingServerRemove/u, 'confirm consumes the CAPTURED identity');
  assert.match(fn, /removeServer\(localStorage, victim\.id\)/u, 'and removes by that id alone');
  assert.match(source, /<ConfirmDialog open=\{!!pendingServerRemove\}[\s\S]*?onconfirm=\{serverRemoveConfirm\} oncancel=\{\(\) => \(pendingServerRemove = null\)\}/u,
    'the shared dialog, cancel drops the capture');
});

test('the system-vitals strip mounts connected-desktop-only inside reserved sidebar space (board #85)', () => {
  // ONE component (src/lib/system/), transport injected from ws.ts — App
  // wires, it does not re-implement.
  assert.match(source, /import SystemStatus from '\.\/lib\/system\/SystemStatus\.svelte'/u,
    'the shared component, not a second rendering');
  assert.match(source, /import \{[^}]*\bsystemStatus\b[^}]*\} from '\.\/lib\/core\/ws\.ts'/u,
    'the typed wrapper is the injected transport');
  assert.match(source, /load=\{systemStatus\}/u, 'load is INJECTED — the component never imports ws');
  // Desktop + connected only, ONE flag: a phone never mounts it and a
  // disconnected client must not poll a socket that is not there.
  assert.match(source, /sysMounted = \$derived\(connected && !layout\.isTouchDevice\)/u,
    'one flag: connected desktop');
  assert.match(source, /\{#if sysMounted\}[\s\S]{0,500}<aside class="sys-sidebar"[\s\S]{0,100}<SystemStatus/u,
    'mount gate wraps the sidebar strip');
  assert.match(source, /<SystemStatus[^/]*visible=\{connected\}/u,
    'visible tracks the connection so a drop stops the timer');
  // Board #85: no footer row under main content. The strip is exactly the
  // shared sidebar width, starts after the 46px rail, and every PRIMARY
  // sidebar shell reserves the same named height beneath its own content.
  assert.match(source, /\.sys-sidebar \{[^}]*position: fixed[^}]*left: 46px[^}]*width: var\(--sidebar-w\)/u,
    'the strip is confined to the primary sidebar column');
  assert.match(source, /--sys-sidebar-h: 34px/u, 'one named height owns geometry');
  assert.match(source, /\.with-rail :global\(\.sidebar\),\s*\.with-rail \.term-side,\s*\.with-rail :global\(\.files-left\) \{[^}]*padding-bottom: var\(--sys-sidebar-h\)/u,
    'every primary sidebar reserves the exact strip height');
  assert.ok(!/sys-footer/u.test(source), 'the retired full-width footer cannot regrow');
});

test('a typed address ends the running reconnect loop before it connects (review 2026-09-03)', () => {
  // onAddress is a NEW intent. Without cancel() first, the machine's next
  // attempt raced this socket; ws.ts superseded the loser, whose 'connection
  // timeout' then marked a reachable address unreachable for two minutes.
  assert.match(source,
    /onAddress=\{\(address\) => \{[\s\S]{0,400}?reconnectMachine\.cancel\(\);\s*disconnect\(\);\s*connect\(address/u,
    'cancel → disconnect → connect, in that order');
});

test('the back-gesture history dance is the phone’s; a desktop browser keeps its Back (2026-09-03)', () => {
  // The seed + re-push + popstate router protect the PHONE's back gesture
  // (app-shell.md). On a desktop layout there is no gesture, and the same
  // dance swallowed the browser's Back for good.
  assert.match(source, /function navPush\(\) \{\s*if \(!layout\.isTouchDevice\) return false;\s*history\.pushState\(\{ app: true \}, ''\);\s*return true;\s*\}/u);
  assert.match(source, /\$effect\(\(\) => \{\s*if \(!layout\.isTouchDevice\) return;[^]*?window\.addEventListener\('popstate', handler\);/u,
    'the popstate router installs only on the touch layout, and re-evaluates when the layout mode changes');
  // A caller only spends an entry it really pushed.
  assert.match(source, /prefsPushed = navPush\(\);/u);
  assert.doesNotMatch(source, /navPush\(\); prefsPushed = true;/u);
});

test('navigation is reachable by keyboard: no nav item opts out of the Tab order (2026-09-03)', () => {
  // Rail icons, the server control, the tab bar and the gear were all
  // tabindex="-1" — the whole navigation was unreachable by keyboard, with no
  // documented reason. The focus ring is the global button:focus-visible.
  const nav = source.match(/<nav class="topbar">[^]*?<\/nav>|<nav\s+class="rail"[^]*?<\/nav>|<nav class="tabbar">[^]*?<\/nav>/gu) ?? [];
  assert.equal(nav.length, 3, 'top bar, rail and tab bar are all present');
  for (const n of nav) assert.doesNotMatch(n, /tabindex="-1"/u, 'a nav item must stay in the Tab order');
  // The current page is announced, not only coloured.
  assert.match(source, /class="rail-btn"[^]*?aria-current=\{page === slot \? 'page' : undefined\}/u);
  assert.match(source, /class:active=\{page === 'terminal'\} aria-current=\{page === 'terminal' \? 'page' : undefined\}/u);
  const style = source.match(/<style>[^]*<\/style>/u)?.[0] ?? '';
  assert.doesNotMatch(style, /\.(?:rail-btn|tabbar button|gear-btn)[^{]*\{[^}]*outline:\s*none/u, 'the ring must not be switched off');
});

test('the server registry has an entry on the touch layout (2026-09-03)', () => {
  assert.match(source, /onServers=\{connected && layout\.isTouchDevice \? toggleServerMenu : null\}/u,
    'Settings gets the opener only where the rail (and its switcher) does not exist');
  assert.match(source, /const serverName = \$derived\(\s*serverList\.find\(\(x\) => x\.id === serverCurId\)\?\.name \|\| hostLabel\(activeAddress\),/u);
  // The popover's outside-dismissal spares whichever control opened it.
  assert.match(source, /serverMenuTrigger\?\.contains\?\.\(e\.target\)/u);
  assert.doesNotMatch(source, /closest\?\.\('\.server-menu, \.rail-server'\)/u);
});
