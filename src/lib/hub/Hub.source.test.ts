// Source-contract test for the tool lane's layout (see docs/conventions/testing.md).
//
// The lane is three columns: tool name (left, never moves), argument (middle, the
// ONLY thing that scrolls), time (right, never moves). The middle cell owns the
// horizontal overflow — `.st-scroll` wraps the text in the MARKUP — which is what
// makes bleed-through structurally impossible: the panning text is clipped by its
// own box, and the name and time are flex children BESIDE that box, not layers
// painted over it.
//
// This replaced a sticky-column build that failed three times in a row: pinned
// columns jumped when offsets were measured from 0, then were 97% transparent
// (`--surface` is a 3% wash), then still leaked into the lane's own padding beside
// the name — a sticky column covers its own box, never the area next to it (owner,
// 2026-08-20: "参数穿模到工具名左侧了"). Structure beats paint; these assertions
// keep the structure.
//
// The other invariant: a lane row is ONE line per call, because the 10-row cap is
// a max-height calculated in single lines (`--steps-rows * 1.5em`). A tool detail
// routinely contains real newlines (a heredoc, a multi-line shell command), so
// `white-space: pre` would silently turn one call into three rows; `nowrap`
// collapses the newlines and is the one allowed value.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Hub.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('the composer stacks above every feed layer, so its popovers are never buried', () => {
  // Board #1: the recipient menu opened UNDER a pinned bubble — .to-wrap's own
  // z-index:2 capped it below .ask-top's 6. The rule is decided ONCE at the
  // composer: it is a stacking context whose level beats the feed's layers
  // (pinned 6, actions overlay 8), so anything opening out of it — the
  // recipient menu, the / palette — wins without per-popover arithmetic.
  // `rule('.composer')` would find `.hub-root.compact .composer` first, so
  // anchor on the base declaration at the start of its own line.
  const composer = /\n  \.composer \{([^}]*)\}/u.exec(source)?.[1] ?? '';
  assert.match(composer, /position:\s*relative/u, 'the composer must be positioned to form a stacking context');
  const z = (css: string) => Number(/z-index:\s*(\d+)/u.exec(css)?.[1] ?? NaN);
  const composerZ = z(composer);
  assert.ok(Number.isFinite(composerZ), 'the composer carries an explicit z-index');
  for (const sel of ['.msg.ask-top', '.msg.ask-bottom']) {
    const feedZ = z(rule(sel));
    assert.ok(Number.isFinite(feedZ), `${sel} still declares a z-index`);
    assert.ok(composerZ > feedZ, `composer (${composerZ}) must stack above ${sel} (${feedZ})`);
  }
  // And inside the composer the palette must stay above the recipient menu —
  // both exist in the same context; the palette is the newer layer.
  assert.ok(z(rule('.cmd-menu')) > z(rule('.to-menu')), 'palette above recipient menu inside the composer');

});

test('the agent card speaks the three-stage machine: select, then options, dblclick filters', () => {
  // Board #3: a click on an UNSELECTED card must only select; the menu is the
  // SECOND click's job; a double click enters the one-agent filter with the
  // menu suppressed (the 260ms defer is what keeps it from flashing).
  const live = source.slice(source.indexOf('class="acard" class:sel'), source.indexOf('class="acard off"'));
  assert.match(live, /onclick=\{\(e\) => cardClick\(a\.name, e\.currentTarget\)\}/u, 'one click handler, the machine');
  assert.match(live, /ondblclick=\{\(\) => cardDbl\(a\.name\)\}/u, 'double-click enters the filter');
  assert.match(live, /onkeydown=[^\n]*cardClick\(a\.name, e\.currentTarget\)/u, 'keyboard mirrors the machine');
  const machine = source.slice(source.indexOf('function cardClick'), source.indexOf('function setRecipient'));
  assert.match(machine, /if \(recipient !== name\) \{ setRecipient\(name\); return; \}/u,
    'an unselected card only selects — no menu on the first click');
  // Review fix: the pending-menu swallow is per CARD. A click on a DIFFERENT
  // card inside the 260ms window cancels the stale menu and still acts —
  // a global swallow ate the selection of the next card.
  assert.match(machine, /if \(cardTimerFor === name\) return;/u,
    'only the SAME card\u2019s second click is swallowed');
  assert.match(machine, /cardTimerFor = name;\s*\n\s*cardTimer = setTimeout/u,
    'the timer records which card it belongs to');
  const swallow = /if \(cardTimer\) \{\s*\n\s*clearTimeout\(cardTimer\); cardTimer = null;\s*\n\s*if \(cardTimerFor === name\) return;/u;
  assert.match(machine, swallow, 'a different card cancels the stale timer and falls through');
  assert.match(machine, /setTimeout\([\s\S]*?toggleAgentMenu\(name, el\)/u,
    'the selected card defers the menu one double-click window');
  assert.match(machine, /filterAgent = filterAgent === name \? '' : name/u,
    'double-click toggles the filter, so it is also an exit');
  // The mode is visible and leavable: a compact pill INSIDE the feed names
  // the agent (reopened #3: as a feed-wrap sibling it rendered as a
  // full-height left column — feed-wrap is row flex), ✕ clears it, and the
  // back gesture peels it before the drawer.
  assert.match(source, /class="filter-pill appear"/u, 'the filter pill lives inside the feed');
  const wrapIdx = source.indexOf('<div class="feed-wrap">');
  const feedIdx = source.indexOf('<div class="feed subtle-scroll"');
  const pillIdx = source.indexOf('class="filter-pill appear"');
  assert.ok(wrapIdx < feedIdx && feedIdx < pillIdx, 'the pill is a FEED child, never a feed-wrap sibling');
  const pill = /\.filter-pill \{([^}]*)\}/u.exec(source)?.[1] ?? '';
  assert.match(pill, /align-self: center/u, 'content width — it owns no column');
  assert.match(pill, /position: sticky/u, 'and stays visible while reading');
  assert.ok(!pill.includes('width: 100%'), 'never full width');
  assert.match(source, /onclick=\{\(\) => \(filterAgent = ''\)\}/u, 'the banner carries the exit');
  assert.match(source, /if \(filterAgent\) \{ filterAgent = ''; return true; \}/u, 'back gesture exits the filter');
});

test('a waiting agent\u2019s card carries the cue, in the dot\u2019s own amber, without motion', () => {
  // Review, 2026-09-03: running got a breathing halo, waiting a 6px static dot
  // — the state that needs the human most was the weakest signal. The CARD
  // wears it now (frame + wash + word), in the one status language: the same
  // --status-warn token stateDotColor paints, chosen by stateNeedsYou (pinned
  // against stateDotColor in hub.test.ts), and static — .live-dot's breathe
  // means "a turn is open", which a suspended turn is not.
  const live = source.slice(source.indexOf('class="acard" class:sel'), source.indexOf('class="acard off"'));
  assert.match(live, /class:needs=\{stateNeedsYou\(a\.state\)\}/u, 'the card class comes from the ONE definition');
  assert.match(live, /\{#if stateNeedsYou\(a\.state\)\}<span class="ac-needs appear">/u, 'and the word is gated by the same one');
  const needs = rule('.acard.needs');
  assert.match(needs, /var\(--status-warn\)/u, 'the frame is the dot\u2019s amber token');
  assert.ok(!/animation/u.test(needs), 'no motion: waiting is not in motion');
  assert.ok(!/#[0-9a-f]{3,8}\b/iu.test(needs), 'no literal colour — tokens only');
  assert.match(rule('.ac-needs'), /var\(--status-warn\)/u, 'the word speaks the same token');
});

test('the filter and the detail level are reachable from menus, not only from a gesture and Settings', () => {
  // Review, 2026-09-03: filtering by agent existed only as an undocumented
  // double-click; the detail level only in Settings (cycleFeedLevel was dead).
  // Both card menus — the tap menu and the right-click/long-press ContextMenu
  // — carry the filter verb from ONE toggle (a menu with its own action set is
  // a second source of truth), and the project title's menu carries the three
  // levels with the current one ticked.
  const items = source.slice(source.indexOf('function agentItems'), source.indexOf('function projectItems'));
  assert.equal([...items.matchAll(/filterItem\(name\)/g)].length, 2, 'live AND stopped agents get the filter verb');
  const menu = source.slice(source.indexOf('{#if menuFor}'), source.indexOf("t('hubRemoveHint')"));
  assert.match(menu, /onclick=\{\(\) => toggleFilter\(menuFor\)\}/u, 'the tap menu uses the same toggle');
  assert.match(source, /function toggleFilter\(name\) \{\s*\n\s*menuFor = '';\s*\n\s*filterAgent = filterAgent === name \? '' : name;/u,
    'the menu toggle is the double-click\u2019s rule, minus the recipient change');
  assert.match(source, /projectItems\(selectedRow, true\)/u, 'the title caret asks for the view rows');
  const lv = source.slice(source.indexOf('function feedLevelItems'), source.indexOf('function msgItems'));
  assert.match(lv, /hubPrefs\.feedLevel === level \? 'check' : 'circle'/u, 'radio semantics through the menu\u2019s own icons');
  assert.match(lv, /hubPrefs\.setFeedLevel\(level\)/u, 'and it writes the one pref Settings reads');
  assert.ok(!source.includes('cycleFeedLevel'), 'the dead cycle control stays gone');
});

test('sidebar dots and roster cards drink from ONE state map (board #8)', () => {
  // Both pollers route through mergeStates: the roster poll overlays its
  // project's keys onto the shared map, and the rooms poll overlays the
  // CURRENT roster onto its fresh snapshot before adopting it — whichever
  // response lands last, the freshest source drives the selected project.
  assert.match(source, /agentStates = mergeStates\(agentStates, s, got\);/u,
    'the 5s roster poll refreshes the shared dot map');
  assert.match(source, /agentStates = mergeStates\(roomsRes\.states \?\? \{\}, selected, agents\);/u,
    'the 20s snapshot is overlaid with the roster before it is adopted');
  assert.ok(!/agentStates = roomsRes\.states/u.test(source),
    'the raw snapshot is never adopted bare — that is the rollback bug');
});

test('can_hire wears ONE atom — the boxed M, words in the title (board #7)', async () => {
  const agentsPage = await readFile(new URL('./AgentsPage.svelte', import.meta.url), 'utf8');
  const i18n = await readFile(new URL('../core/i18n.svelte.ts', import.meta.url), 'utf8');
  // The atom renders as a literal M with the explanation in title/aria.
  assert.match(source, /class="m-badge" title=\{t\('agentsManagerHint'\)\}[^>]*>M</u, 'Hub preset rows wear the badge');
  assert.match(agentsPage, /class="m-badge" title=\{t\('agentsManagerHint'\)\}[^>]*>M</u, 'the config list wears the badge');
  assert.match(agentsPage, /<span class="m-badge">M<\/span>\{t\('agentsManager'\)\}/u, 'the editor toggle: badge + the short word');
  // One declaration, twice: no app.css edits were allowed, so the two scoped
  // copies must stay TEXT-IDENTICAL or the atom forks.
  const decl = (src: string) => /\.m-badge \{([^}]*)\}/u.exec(src)?.[1]?.trim() ?? '';
  assert.ok(decl(source).length > 0, 'Hub declares the atom');
  assert.equal(decl(source), decl(agentsPage), 'the two m-badge declarations are the same text');
  // The old vocabulary is gone from the UI strings (comments may cite history).
  assert.ok(!i18n.includes("agentsCanHire"), 'the old i18n keys are retired');
  assert.ok(!i18n.includes('可拉人'), 'the old zh wording is gone');
  assert.match(i18n, /agentsManager: 'Manager'/u, 'the word is Manager in both languages');
});

test('history paging: anchored prepend, guarded rooms, parked cursors (board #9)', () => {
  // The prepend re-enters through the reading anchor — scrollTop compensation
  // is off (overflow-anchor: none), so without this every older page teleports
  // the reader.
  const walk = source.slice(source.indexOf('async function loadOlder'), source.indexOf('function onFeedScroll'));
  assert.match(walk, /if \(selected !== s\) return;/u, 'a room switch drops the in-flight page');
  assert.match(walk, /await withReadingAnchor\(\(\) => \{/u,
    'the anchored prepend is AWAITED — releasing loadingOlder before the scroll compensation re-walked the same cursor');
  assert.match(walk, /if \(loadingOlder \|\| !selected \|\| \(!histMore && !actMore\)\) return;/u,
    'one walk at a time, parked at has_more=false');
  // Cursors ride the room cache, so returning to a room never re-walks it.
  assert.match(source, /roomCache\.set\(selected, \{ feed, lastTs, activity, lastActivityTs, agents, histSeq, histMore, actCursor, actMore \}\);/u,
    'both cursors and both has_more flags are parked per room');
  // The activity poll merges — the old concat-and-slice(-300) EVICTED walked
  // history, and a bare concat doubles what a page overlap re-sends.
  assert.match(source, /activity = mergeEvents\(activity, events\);/u, 'poll path merges');
  assert.ok(!source.includes('.slice(-300)'), 'the eviction cap is gone');
  // The reader is told, quietly: fetching, or the confirmed beginning.
  assert.match(source, /class="older-hint"/u, 'top-of-scrollback feedback exists');
  // A first page shorter than the viewport fires no scroll event, so the
  // walk has a clickable entry too (visible only while a page remains).
  assert.match(source, /class="older-hint older-more" onclick=\{loadOlder\}/u,
    'the load-earlier entry is the same whisper, wired to the same walk');
});

test('the fold budget goes through foldLines, and the basis is the column', () => {
  // Board #4: the budget math lives in hub.ts (pure, tested) — Hub only
  // measures. Re-inlining `* 0.2` here would fork the mapping again, and
  // measuring the FEED instead of its parent is the composer-shrink bug
  // (owner, 2026-08-27) coming back.
  assert.match(source, /const heldLines = \$derived\(foldLines\(compact, heldBasis, heldLine\)\);/u,
    'one derivation, the pure mapping');
  assert.match(source, /heldBasis = feedEl\.parentElement\?\.clientHeight/u,
    'the basis is the chat COLUMN (the feed parent), which the composer cannot shrink');
  const script = source.slice(0, source.indexOf('</script>'));
  assert.ok(!/\*\s*0\.2/u.test(script), 'no inline fifth — the fraction lives in foldLines only');
  assert.match(source, /const onResize = \(\) => withReadingAnchor\(measureHeld\);/u,
    'a real window resize still re-enters through the reading anchor');
});

test('the argument is wrapped in its own scroller, beside the name and the time', () => {
  // The markup IS the guarantee: text inside .st-scroll, name and time outside it.
  assert.match(
    source,
    /<span class="st-scroll"[^>]*><span class="st-text">\{ep\.text\}<\/span><\/span>\s*<span class="st-ts">/u,
    'st-text must live inside st-scroll, with st-ts a sibling after it',
  );
  assert.match(source, /class="tname"[^>]*>\{ep\.tool\}<\/span>\{\/if\}\s*(?:<!--[\s\S]*?-->\s*)?<span class="st-scroll"/u,
    'the name is a sibling before the scroller, never inside it');
});

test('only the middle cell scrolls, and a row stays one line', () => {
  const scroll = rule('.step .st-scroll');
  assert.match(scroll, /flex:\s*1/u, 'the middle takes the leftover width');
  assert.match(scroll, /min-width:\s*0/u, 'a flex child does not shrink below content without this');
  assert.match(scroll, /overflow-x:\s*auto/u);

  const text = rule('.step .st-text');
  assert.match(text, /white-space:\s*nowrap/u, 'nowrap keeps a multi-line detail on one row');
  assert.doesNotMatch(text, /white-space:\s*pre\b/u, 'pre would make a heredoc a three-line row');
  assert.doesNotMatch(text, /text-overflow:\s*ellipsis/u, 'what does not fit is scrolled to, not cut');

  // The columns beside the scroller are plain flex children: no sticky, no
  // painted-over backgrounds — the layers that bled through, twice.
  // (`flex: none` for the name lives on the base `.tname` rule.)
  assert.match(rule('.tname'), /flex:\s*none/u);
  assert.match(rule('.step .st-ts'), /flex:\s*none/u);
  for (const sel of ['.step .tname', '.step .st-ts']) {
    assert.doesNotMatch(rule(sel), /position:\s*sticky/u, `${sel}: structure beats paint`);
  }

  // The lane itself never scrolls horizontally — that is what makes the clip hold.
  assert.match(rule('.s-body'), /overflow-x:\s*hidden/u);
});

test('the row cap is still expressed in rows, not pixels', () => {
  // If this becomes a pixel height it stops following the type scale, and the
  // "ten rows" promise silently becomes "some height".
  assert.match(rule('.s-body.capped'), /max-height:\s*calc\(var\(--steps-rows\)/u);
  // And the inner scroller CHAINS at its edges: `overscroll-behavior: contain`
  // trapped the gesture, so scrolling across a tool group stuck the whole feed
  // (owner, 2026-08-21: "手势点在工具调用框框，就卡住了滚不上去了"). The one
  // legitimate contain is the held-ask's own scroller, pinned elsewhere.
  assert.doesNotMatch(rule('.s-body.capped'), /overscroll-behavior/u);
});

test('the lane offsets stay named, and named once', () => {
  // The body and the "show all" button both measure from --lane-indent/--lane-pad-r
  // on `.steps`; a literal copy anywhere is how the two drift apart (it happened:
  // `.s-all` carried its own 30px).
  const steps = rule('.steps');
  assert.match(steps, /--lane-indent:\s*30px/u);
  assert.match(steps, /--lane-pad-r:\s*10px/u);
  assert.match(rule('.s-body'), /padding:\s*5px var\(--lane-pad-r\) 6px var\(--lane-indent\)/u);
  assert.match(rule('.s-all'), /padding:\s*2px var\(--lane-pad-r\) 5px var\(--lane-indent\)/u);
});

test('an expanded message is never pinned, and nothing caps the bubble', () => {
  // The bubble must stay uncapped: a max-height on `.held`'s flow box is what
  // fed Chromium's scroll anchoring and produced the infinite blink (measured
  // 2026-08-19). And an EXPANDED message must leave the anchor pool entirely —
  // sticky ignores the feed's scrolling, so a pinned screen-tall message had an
  // unreachable bottom half (owner, 2026-08-27: "如果展开了消息 就要把钉住用户
  // 消息关掉 不然展开就没法上下滑动了"; the in-body held-scroll scroller was the
  // earlier answer and is retired — the feed itself scrolls the whole message).
  const heldMsg = rule('.msg.held');
  const heldBubble = rule('.msg.held .bubble');
  assert.doesNotMatch(heldMsg, /max-height/u);
  assert.doesNotMatch(heldBubble, /max-height/u);
  assert.ok(!source.includes('class:held-scroll'), 'the in-body scroller is retired');
  assert.ok(!source.includes('.held-scroll'), 'no orphaned held-scroll CSS');

  // Every pin class hangs off ONE gate that excludes the expanded message.
  assert.match(source, /const pinned = isAsk && askKey === key && !expanded\[key\]/u);
  assert.match(source, /class:ask-top=\{pinned && askEdge === 'top'\}/u);
  assert.match(source, /class:ask-bottom=\{pinned && askEdge === 'bottom'\}/u);
  assert.match(source, /class:held=\{pinned && askHeld\}/u);
});

test('the drawer opens and closes through the reading anchor, everywhere', () => {
  // The drawer regrids the columns and every message rewraps; without the anchor
  // the reader's message drifts (owner, 2026-08-20). One open path and one close
  // path, both wrapped — a bare `termOpen = true/false` outside selectProject is
  // a trigger someone forgot to route.
  const bare = [...source.matchAll(/termOpen = (?:true|false)/g)].length;
  assert.equal(bare, 2, 'exactly the two wrapped mutations; selectProject restores via termOpen = !!dv');
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = true; \}\)/u);
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = false; \}\)/u);
  // The reference skips every sticky variant: a pinned rect does not move with
  // the flow, so anchoring to it restores nothing.
  assert.match(source, /!el\.classList\.contains\('held'\)/u);
  assert.match(source, /!el\.classList\.contains\('ask-top'\)/u);
  assert.match(source, /!el\.classList\.contains\('ask-bottom'\)/u);
});

test('an Esc typed into the drawer terminal reaches the pane, not closeDrawer', () => {
  // Escape is how every agent TUI cancels the turn it is running. The drawer's
  // window-capture listener used to eat it unconditionally — pressing Esc in
  // the focused terminal closed the drawer instead of reaching the agent
  // (owner, 2026-08-26). Two guards, both load-bearing:
  // an event from inside the terminal is the pane's,
  assert.match(source, /e\.target\?\.closest\?\.\('\.xterm'\)/u, 'focused-terminal Esc must pass through');
  // and a HIDDEN Hub (pages stay mounted) must not steal the Terminal page's Esc.
  assert.match(source, /if \(!termOpen \|\| !visible\) return;/u, 'the listener is gated on visible');
});

test('a lifecycle group is one row per line, in one who/action/detail grammar', () => {
  // Joined by a `·` on one nowrap line, "removed k" and "spawned k" read as a
  // single grey run-on string (owner, 2026-08-24). The capsule stays (a stop plus
  // its restart is one fact) and becomes a column; the separator is gone.
  assert.doesNotMatch(source, /class="sys-sep"/u, 'rows are stacked now, not joined');
  const line = rule('.sysline');
  assert.match(line, /flex-direction:\s*column/u);
  assert.doesNotMatch(line, /white-space:\s*nowrap/u, 'the capsule no longer clips one long line');
  // Every row speaks the SAME grammar (owner, 2026-08-24: "都用统一的 ui 来展示"),
  // and each atom reuses a dialect the feed already has: the name wears the
  // bubble header's ink, the action the status-note badge (dot + word,
  // sysVerbColor), a /command the composer's monospace.
  assert.match(source, /class="sys-who"/u, 'the agent name is its own atom');
  assert.match(rule('.sysline .sys-who'), /font-weight:\s*650/u, "the name wears the bubble header's weight");
  assert.match(source, /class="sys-verb" style:color=\{c\}><span class="sv-dot"/u, 'the action badge carries the state dot');
  assert.match(source, /const c = sysVerbColor\(p\.verb\)/u);
  // No drawn frames on the inner atoms — they read as chrome, not content
  // ("不用这种边框的", owner 2026-08-24): the verb is dot + coloured word, the
  // command a soft --code-bg wash in the inline-code dialect.
  assert.doesNotMatch(rule('.sysline .sys-verb'), /border/u, 'the verb badge is dot + word, not a pill');
  // A /command's typed line stays ONE object — name and args together in the
  // composer's own command costume; a micro-pill name beside loose args at
  // another size read as fragments ("带参数的渲染好像不是很好", 2026-08-24).
  assert.match(source, /class="sys-cmd">\{p\.text \? `\$\{p\.verb\} \$\{p\.text\}` : p\.verb\}<\/span>/u);
  const cmd = rule('.sysline .sys-cmd');
  assert.match(cmd, /ui-monospace/u);
  assert.match(cmd, /var\(--code-bg\)/u, 'the wash is the inline-code dialect, not a drawn frame');
  assert.doesNotMatch(cmd, /border:/u, 'no border on the command capsule');
  assert.match(cmd, /text-overflow:\s*ellipsis/u, 'a long command clips itself, not its neighbours');
  assert.doesNotMatch(source, /class="sys-verb cmd"/u, 'no second command dialect');
  // Per-row ellipsis lives on the text, so a long detail cannot eat the badge.
  assert.match(rule('.sysline .sys-text'), /text-overflow:\s*ellipsis/u);
});

test('the add-agent button is reachable in every project', () => {
  // Two gates hid the ONE entry point to an empty room, and each hid it in a
  // different situation: a non-empty-roster gate lost it with the last agent, and
  // a live-session gate still hid it on a CLOSED project (owner, 2026-08-24,
  // twice — "test 这个 project"). So the row hangs off the SELECTION and the
  // button off nothing: `projects::spawn` calls `tmux::ensure_session`, so a
  // spawn into a project that is down opens it.
  const roster = source.match(/\{#if ([^}]*)\}\s*(?:<!--[\s\S]*?-->\s*)*<div class="roster">/u)?.[1];
  assert.equal(roster, 'selected', 'the roster row is gated on the selection alone');
  assert.match(source, /(?:<!--[\s\S]*?-->\s*)<button class="acard add"/u,
    'the add button has no {#if} of its own');
  assert.doesNotMatch(source, /\{#if liveSelected\}/u, 'no live gate stands between a project and its first agent');
  // The empty-room preset panel is the same entry point in another shape: it
  // must not disagree with the button about when adding an agent is possible.
  assert.match(source, /\{#if selected && !managedAgents\.length && registry\.length\}/u);
});

test('a command-shaped draft styles the composer, with the mirror in step', () => {

  // The look mirrors send()'s own branch (slashCommand + a target), so the
  // capsule never promises a command that send() would deliver as prose.
  assert.match(source, /class:cmd=\{composerIsCmd\}/u);
  assert.match(source, /const composerIsCmd = \$derived/u);
  // The metrics trap: growComposer's mirror re-lays-out the text to find the
  // last line. If the input flips to monospace and the mirror does not, the
  // send button's collision zone is measured in the wrong font.
  assert.match(
    source,
    /\.compose-shell\.cmd \.c-input, \.compose-shell\.cmd :global\(\.c-mirror\) \{ font-family: ui-monospace/u,
  );
  // And the height re-measures when the font flips, not just when text changes.
  assert.match(source, /void composerIsCmd;/u);
});

test('a confirmed project verb runs on the row it was asked on, never on `selected`', () => {
  // The context menu opens on ANY sidebar row; the confirm dialog then fired
  // `rows.find(… === selected)`, closing whichever project was OPEN instead of
  // the one long-pressed (owner, 2026-08-24: "关的不是我选中的 是其他的").
  // The target is frozen at ask time: askAction carries the row's session and
  // runAction resolves with it.
  assert.match(source, /askAction = \(kind, name, session = selected\)/u);
  const run = source.slice(source.indexOf('async function runAction'), source.indexOf('function restoreProject'));
  assert.match(run, /rows\.find\(\(r\) => r\.project\.session === session\)/u);
  assert.doesNotMatch(run, /rows\.find\(\(r\) => r\.project\.session === selected\)/u);
  // The context menu's two destructive verbs both pass their row's identity.
  assert.match(source, /askAction\('down', name, session\)/u);
  assert.match(source, /askAction\('delete', name, session\)/u);
  // The async flavor of the same bug: a poller's late reply must still be
  // about the project it asked about, or a project switch mid-flight merges
  // the OLD room's data into the NEW one. Each poller freezes `selected` and
  // drops a stale answer.
  const pollers = ['async function loadFeed', 'async function loadActivity', 'async function loadAgents'];
  for (const head of pollers) {
    const at = source.indexOf(head);
    assert.ok(at >= 0, `${head} exists`);
    const body = source.slice(at, at + 900);
    assert.match(body, /const s = selected;/u, `${head} freezes its project`);
    assert.match(body, /if \(selected !== s\) return;/u, `${head} drops a stale answer`);
  }
});

test('a stopped agent restarts only from its refresh button, never the card', () => {
  // The whole stopped card used to be onclick=startAgent — brushing it
  // restarted the agent (owner, 2026-08-24: "已经停止的agent我只要点击就自动
  // 重启了 并没有点到重启的那个圆圈箭头上"). The surface now opens the agent
  // MENU (owner, 2026-08-25: the dots were retired for a card-wide tap) —
  // showing options is safe; the one start trigger stays the .a-start button.
  const off = source.slice(source.indexOf('class="acard off"'), source.indexOf('{/each}', source.indexOf('class="acard off"')));
  assert.match(off, /class="a-start"/u, 'the refresh icon is a real button');
  assert.match(off, /stopPropagation\(\); startAgent\(name\)/u, 'and it is what starts the agent');
  const surface = off.slice(0, off.indexOf('<div class="ac-top">'));
  assert.match(surface, /onclick=\{\(e\) => toggleAgentMenu\(name, e\.currentTarget\)\}/u, 'the card surface opens the menu');
  assert.doesNotMatch(surface, /startAgent/u, 'and never starts the agent itself');
});

test('a board move renders in the sys grammar, transition visible (board #13)', () => {
  // The issue number is the WHO, the destination the coloured badge, the FROM
  // stays visible (done → todo is a REOPEN and must read as one).
  assert.match(source, /\{@const bl = boardLine\(item\)\}/u, 'each sys item is offered to the board parser first');
  assert.match(source, /<span class="sys-who">#\{bl\.id\}<\/span>/u, 'the issue number wears the name ink');
  assert.match(source, /<span class="sys-from">\{t\(`boardStatus_\$\{bl\.from\}`\)\} →<\/span>/u,
    'the origin status is quiet but present');
  assert.match(source, /style:color=\{boardStatusColor\(bl\.to\)\}/u,
    'the destination badge speaks the one progressive status language');
  assert.match(source, /\{t\(`boardStatus_\$\{bl\.to\}`\)\}/u, 'statuses wear the board page\u2019s own labels');
  // And the row is a BUTTON that jumps to the issue on the board page — the
  // same openBoardTab route the header's layout icon takes, now carrying the
  // issue id.
  assert.match(source, /<button class="sys-item sys-jump"[\s\S]{0,300}?openBoardTab\?\.\(selected, Number\(bl\.id\)\)/u,
    'tapping a board line opens that issue');
});

test('the drawer has a board partition, and the tap prefers it on desktop (board #13)', () => {
  // The task sidebar: the REAL Board component, embedded, following the room's
  // project — the same split the files partition makes (page on the phone,
  // partition on desktop).
  assert.match(source, /\{#if drawerView === 'board'\}[\s\S]{0,400}?<Board session=\{selected\}[^>]*embedded issueRequest=\{drawerIssueReq\}/u,
    'the drawer hosts the embedded Board');
  assert.match(source, /drawerView = 'board'; openDrawer\(\);/u, 'the feed tap opens the partition on desktop');
  assert.match(source, /if \(mobile \|\| compact\) \{ openBoardTab\?\.\(selected, Number\(bl\.id\)\); return; \}/u,
    'the phone still jumps to the board page');
  assert.match(source, /e\.target\?\.closest\?\.\('\.board-body'\)/u,
    'an Esc inside the partition belongs to the Board (the files-body territory rule)');
});

test('the header toggles read board, files, terminal — the owner-set order (2026-08-29)', () => {
  const start = source.indexOf('<!-- The task board:');
  const bar = source.slice(start, source.indexOf('{#if selected}', start));
  const iBoard = bar.indexOf("name=\"layout\"");
  const iFiles = bar.indexOf("name=\"files\"");
  const iTerm = bar.indexOf("name=\"terminal\"");
  assert.ok(iBoard >= 0 && iFiles > iBoard && iTerm > iFiles, 'board, then files, then terminal');
});

test('the header folds the project verbs into ONE dots menu (owner, 2026-08-29)', () => {
  // Rename/Open/Close/Delete live behind the same projectItems menu the
  // sidebar row speaks — one source of truth; the partition toggles stay out
  // (navigation, not consequence).
  // `true` = plus the feed's detail-level rows (review, 2026-09-03) — the same
  // shared list, one flag, not a second menu.
  assert.match(source, /projectItems\(selectedRow, true\)\);/u, 'the dots button opens the shared menu');
  // The ⋯/name grouping is pinned by its own test below ("grouped WITH the
  // name") — the title-group holds them at a 3px gap, sibling of the h1.
  const head = source.slice(source.indexOf('class="h1-text"'), source.indexOf('{#if selected}', source.indexOf('class="h1-text"')));
  assert.ok(!head.includes('h1-pen'), 'the title pencil retired — rename lives in the menu');
  const bar = source.slice(source.indexOf('<span class="spacer"></span>'), source.indexOf('<!-- The task board:'));
  assert.ok(!bar.includes("name=\"trash\"") && !bar.includes("name=\"stop\"") && !bar.includes("name=\"zap\""),
    'no standalone delete/close/open buttons in the header');
  assert.ok(!bar.includes('name="dots"'), 'the ⋯ moved BESIDE the name (owner, 2026-08-30) — not in the right-aligned group');
});

test('the ⋯ is grouped WITH the name, so the row gap cannot separate them (owner, 2026-08-30)', () => {
  // "离 project name 还是有点远，可以直接紧挨着 name，让人觉得是可以点击操作的".
  // Being the next child of .page-head was not enough: the header's own gap
  // (10px, 7px compact) sat between them, and on ≤760px the shared
  // `.page-head h1 { flex: 1 1 auto }` stretched the title across the row and
  // parked the ⋯ at the far right. One content-sized group fixes both without
  // touching the shared rule.
  const group = source.slice(source.indexOf('<div class="title-group">'), source.indexOf('<!-- The FULL path'));
  assert.ok(group.length > 0, 'the title group must exist');
  assert.ok(group.includes('class="h1-text"'), 'the name lives in the group');
  assert.ok(group.includes('class="h1-edit"'), 'so does the rename input — the ⋯ must not jump when renaming starts');
  assert.ok(group.includes('name="chevron-down"'), 'and so does its caret — the dropdown grammar (owner, 2026-08-30: "向下的直角箭头…像把这个名字展开")');
  assert.match(source, /\.title-group \{[^}]*gap: 1px[^}]*\}/u, 'a tight, deliberate gap — not the row rhythm');
  // The NAME wins the width fight against the PATH (owner, 2026-08-30: "名字
  // 还是优先要显示全的") but never against the BUTTONS (owner, 2026-09-02,
  // board #72: a long name on a phone pushed the icon buttons off-screen while
  // the group was `flex: none`). So: the group shrinks at weight 1 with
  // min-width 0 (the ellipsis can engage), the path at weight 1000 (it yields
  // first, and scrolls), the buttons are flex: none.
  assert.match(source, /\.title-group \{[^}]*flex: 0 1 auto; min-width: 0;[^}]*\}/u, 'the name group CAN shrink — buttons come first');
  assert.ok(!/\.title-group \{[^}]*flex: none/u.test(source), 'flex: none is what hid the buttons');
  assert.match(source, /\.path \{[\s\S]{0,800}?min-width: 0; flex: 0 1000 auto;/u,
    'the path gives way first — a 1000× shrink weight, below content, scrolls');
  // The button must stay a SIBLING of the h1, not a child: the heading's
  // `overflow: hidden` would clip the invisible ~42px tap overlay the compact
  // rule adds, making the affordance read closer but tap worse.
  const h1 = source.slice(source.indexOf('<h1>', source.indexOf('<div class="title-group">')), source.indexOf('</h1>'));
  assert.ok(!h1.includes('name="chevron-down"'), 'the caret sits beside the h1, never inside it');
});

test('a delivered prompt sheds its stamp and board deliveries wear the dialect (board #18)', () => {
  assert.match(source, /\{@const pp = promptParts\(b\.text\)\}/u, 'every prompt row goes through the reader');
  assert.match(source, /\{#if pp\.from\}<span class="p-from">\{pp\.from\}<\/span>\{\/if\}/u,
    'the sender joins the head — the stamp never renders');
  assert.match(source, /<span class="p-chip">#\{pp\.board\.id\}<\/span>/u, 'the issue chip');
  assert.match(source, /style:color=\{boardStatusColor\('review'\)\}/u,
    'the review badge speaks the one status language');
});

test('the drawer wears the app ground and its head is the page-head\u2019s twin (board #23)', () => {
  // A hardcoded #000 drawer leaked out as a black seam beside the chat column
  // ("侧边栏竖线现在是一个黑色的线条"): the terminal paints its OWN theme-
  // adapted background, so every uncovered sliver of the drawer read as black
  // in a light app. The drawer's ground is the app's.
  const drawer = /\.drawer \{ display: flex;[^}]*\}/u.exec(source)?.[0] ?? '';
  assert.match(drawer, /background: var\(--bg\);/u, 'the drawer sits on the theme ground');
  assert.ok(!source.includes('background: #000'), 'no hardcoded black ground anywhere in the Hub');
  // The two top bars must read as ONE line through the divider ("横条…没对齐，
  // 颜色不一致"): same 42px min-height + box-sizing as app.css .page-head,
  // same border token, and NO private background (bg2 was the mismatch).
  const head = /\.drawer-head \{[^}]*\}/u.exec(source)?.[0] ?? '';
  assert.match(head, /min-height: 42px; box-sizing: border-box;/u, 'the head shares the page-head height');
  assert.match(head, /border-bottom: 1px solid var\(--border\);/u, 'and the page-head border');
  assert.ok(!head.includes('background'), 'transparent over the shared ground — no second color');
  // The board partition's + lives in the drawer head and reaches the embedded
  // Board as a request (its own page-head is gone — see Board.source.test).
  assert.match(source, /drawerBoardNew = \{ n: \(drawerBoardNew\?\.n \?\? 0\) \+ 1 \}/u, 'the + issues a request');
  assert.match(source, /<Board [^>]*createRequest=\{drawerBoardNew\}/u, 'and the Board receives it');
  // Parent-owned suppression (the lead's belt over the child's gate): the
  // drawer KNOWS the embedding, so it enforces the one-header contract itself —
  // any page-head a prop/HMR/child-path drift might leak into the partition
  // neither shows nor keeps its height. display:none, not visibility: a
  // hidden-but-laid-out header would still push the board down ("保留高度").
  assert.match(source, /\.board-body :global\(\.page-head\) \{ display: none; \}/u,
    'the drawer suppresses any child page-head — the drawer head is the only header');
});

test('paste and the + button stage attachments through ONE pipeline (board #25)', () => {
  // The composer textarea accepts pasted images/files: onpaste routes through
  // pastedFiles() (files win over co-riding text) into the SAME stageFiles()
  // the file picker uses — a second upload path would drift (token insertion,
  // re-encode, .tmm/uploads layout) the moment either one changed.
  assert.match(source, /class="c-input"[^>]*onpaste=\{onComposerPaste\}/su,
    'the composer textarea must wire onpaste');
  const handler = /function onComposerPaste\(e\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(handler, /pastedFiles\(e\.clipboardData\)/u, 'files come from the pure extractor');
  assert.match(handler, /preventDefault/u, 'a file paste suppresses the default text insertion');
  assert.match(handler, /stageFiles\(files\)/u, 'staging is the shared pipeline');
  const picker = /async function onPickFiles\(e\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(picker, /stageFiles\(files\)/u, 'the + button goes through the same pipeline');
  assert.doesNotMatch(picker, /fsUpload|encodeImage/u, 'the picker holds no upload logic of its own');
});

test('the drawer follows the project — partition parked and restored per room (board #23)', () => {
  // Owner: "chat的右侧边栏打开哪个的状态前端帮我记住，这样我切换不同的
  // project 回来原来的视图还在". ONE record point per direction — openDrawer
  // (every view switch routes through it) and closeDrawer (the one close
  // path) — and ONE restore point in selectProject. A second write site for
  // the same pref is how two sources of truth drift.
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = true; \}\);\n(?:\s*\/\/[^\n]*\n)*\s*hubPrefs\.setDrawer\(selected, drawerView\);/u,
    'opening records which partition this room shows');
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = false; \}\);\n\s*hubPrefs\.setDrawer\(selected, ''\);/u,
    'closing records closed');
  assert.equal([...source.matchAll(/hubPrefs\.setDrawer\(/g)].length, 2,
    'exactly the two record points');
  // Restore: compact has no drawer, an unknown room comes back closed, and
  // the old room's pane target never leaks into the new room's terminal.
  assert.match(source, /const dv = compact \? '' : hubPrefs\.drawer\(session\);/u);
  assert.match(source, /termOpen = !!dv;/u);
  assert.match(source, /termTarget = ''; termCommand = '';/u, 'stale pane target cleared on switch');
});

test('a stage job dies with its room, and nothing sends while one is in flight (board #25 review)', () => {
  // Lead review of 898d888, race (1): stageFiles is async — a project switch
  // mid-upload used to let the finished job refill the NEW room's composer
  // with the OLD room's attachment. clearAttachments (which selectProject
  // calls) bumps the generation; the job snapshots it at entry and re-checks
  // after EVERY await; the commit (pending, token, sequence) sits after the
  // last check with no await in between.
  assert.match(source, /function clearAttachments\(\) \{\n\s*attachGen\+\+;/u,
    'clear/switch invalidates every in-flight stage job');
  const stage = /async function stageFiles\(files\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.ok(stage, 'stageFiles found');
  assert.match(stage, /const gen = attachGen;/u, 'the job snapshots its generation at entry');
  const code = stage.replace(/\/\/[^\n]*/g, ''); // comments SAY "await" too
  const awaits = [...code.matchAll(/await /g)].length;
  const checks = [...code.matchAll(/if \(stale\(\)\) return;/g)].length;
  assert.ok(awaits >= 6, `stageFiles has its awaits (saw ${awaits})`);
  assert.equal(checks, awaits, 'one staleness check per await — a new await without one is the race back');
  assert.ok(stage.lastIndexOf('if (stale()) return;') < stage.indexOf('pending = [...pending, item]'),
    'the commit mutates only after the final check');
  // Race (2): with text already typed, sendable stayed true while a job was
  // in flight — the message could leave without its attachment, which then
  // landed in an emptied composer as an orphaned token. Both gates, so the
  // keyboard's Enter (send() entry) and the pointer (disabled) agree; while
  // attaching the button is disabled OUTRIGHT, so an empty composer cannot
  // turn into a clickable interrupt mid-upload.
  const sendFn = /async function send\(\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.ok(sendFn, 'send found');
  assert.match(sendFn, /if \(attaching\) return;/u, 'send() refuses while a stage job is in flight');
  assert.match(source, /disabled=\{!selected \|\| attaching \|\|/u, 'the send button is disabled too');
  // The gate answers PER GENERATION, and every job books/releases only its
  // OWN entry: a stale job neither holds the new room's send closed nor —
  // via its finally — unlocks a job the new room started. A count or a flag
  // fails one side or the other (a blanket reset in clearAttachments would
  // let the old finally push it negative and free a running new job).
  assert.match(source, /const attaching = \$derived\(jobGens\.includes\(attachGen\)\);/u,
    'attaching asks whether the CURRENT generation has jobs');
  assert.match(stage, /jobGens = \[\.\.\.jobGens, gen\];/u, 'a job books itself under its own generation');
  const fin = /finally \{([\s\S]*?)\n    \}/u.exec(stage)?.[1] ?? '';
  assert.match(fin, /jobGens\.indexOf\(gen\)/u, 'finally releases exactly its own entry');
  assert.ok(!/jobGens = \[\]/u.test(source), 'nothing blanket-resets the job list');
  // Same genre on the way out (round 2): the SUCCESS path after `await
  // hubPost` used to reset attachSeq and refresh the feed unconditionally —
  // a room switched to mid-post that had staged its own attachments got its
  // token numbering reset (colliding numbers) and the wrong feed refreshed.
  // The post goes to the CAPTURED room; per-room state mutates only if the
  // user is still there; and a failed post must not restore the old room's
  // draft/attachments into the new one.
  assert.match(sendFn, /await hubPost\(room, text\);/u, 'the post names its room, not whatever is on screen');
  assert.equal([...sendFn.matchAll(/if \(selected !== room\) return;/g)].length, 2,
    'BOTH branches (command, message) stop their success path at the room boundary');
  const postIdx = sendFn.indexOf('await hubPost(room, text);');
  const guardIdx = sendFn.indexOf('if (selected !== room) return;', postIdx);
  const seqIdx = sendFn.indexOf('attachSeq = 1;');
  assert.ok(guardIdx > postIdx && seqIdx > guardIdx,
    'the message-path guard sits BETWEEN the post and the per-room mutations (attachSeq/loadFeed/scroll)');
  assert.match(sendFn, /if \(selected === room\) \{ pending = atts; composerText = raw; \}/u,
    'a failed post restores only into its own room');
  // The slash-command branch is the same function, same race, same rule.
  assert.match(sendFn, /await hubCommand\(room, cmdTarget, cmd\.command\);/u);
  assert.match(sendFn, /if \(selected === room\) composerText = raw;/u,
    'a failed command restores only into its own room');
});

test('a failed attachment is a chip that blocks send, never a console line', () => {
  // Review, 2026-09-03: an oversized file or a failed upload only
  // console.warned, so the user could not tell what the message would carry.
  const stage = /async function stageFiles\(files\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.ok(!/console\.warn/u.test(stage), 'staging reports to the user, not to the console');
  assert.ok([...stage.matchAll(/failedAttachment\(/g)].length >= 3, 'too large, a per-file throw and a dir failure each become a chip');
  assert.match(stage, /try \{[\s\S]*?\} catch \(err\) \{[\s\S]*?if \(!stale\(\)\) pending = \[\.\.\.pending, failedAttachment/u,
    'the per-file catch guards staleness before it touches the room');
  const sendFn = /async function send\(\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(sendFn, /if \(failed\) return;/u, 'send() refuses while a failed chip stands');
  assert.match(source, /disabled=\{!selected \|\| attaching \|\| failed \|\|/u, 'and the button is disabled too');
  assert.match(source, /const sendable = \$derived\(!!composerText\.trim\(\) \|\| pending\.some\(\(a\) => !a\.error\)\);/u,
    'a failed chip alone is not content');
  assert.match(source, /\{#each pending as a, i \(a\.key\)\}\s*\n\s*\{#if a\.error\}/u, 'the error state renders first, keyed by key (a failed chip has no path)');
  const err = rule('.pend-chip.err');
  assert.match(err, /var\(--status-danger\)/u, 'the danger token, not a literal');
  assert.ok(!/#[0-9a-f]{3,8}\b/iu.test(err), 'no literal colour');
});

test('the title caret expands the NAME — left-aligned on its real rect (board #32)', () => {
  // The project-actions menu used to right-align on the caret and could clip
  // at the right edge. It now anchors on the name element's REAL rect
  // (anchorOf → zoom-corrected) with the left alignment, so the menu's left
  // edge sits where the visible name starts; while renaming (the span is an
  // input) the caret itself anchors.
  assert.match(source, /<span class="h1-text" bind:this=\{titleNameEl\}>/u, 'the name span is the anchor');
  assert.match(source, /openCtx\(\{ anchor: anchorOf\(titleNameEl \?\? e\.currentTarget\), align: 'left' \}/u,
    'the title entry alone chooses the left alignment');
  // Every OTHER context menu keeps the pointer default (right-aligned):
  // exactly one opener passes an anchor/align.
  const opens = [...source.matchAll(/openCtx\((?!at, who)/g)].length; // call sites, not the definition
  const aligned = [...source.matchAll(/openCtx\(\{ anchor:/g)].length;
  assert.equal(aligned, 1, 'ONE left-aligned entry');
  assert.equal(opens - aligned, 7, 'the seven pointer entries stay pointer-anchored');
  assert.ok(!/getBoundingClientRect\(\)[^]{0,80}openCtx/u.test(source),
    'no raw client rect reaches openCtx — anchorOf owns the zoom correction');
  // The narrowed caret resets the BROWSER's button padding (Chromium: 1px 6px,
  // which .icon-btn never clears): without it the 14px glyph overflowed flush
  // against the button's right edge — measured live, svg.right == button.right
  // — and read as clipped the moment the wash showed (owner, 2026-09-01).
  assert.match(source, /\.title-caret \{ width: 20px; padding: 0;/u, 'the 20px caret zeroes the UA padding');
});

test('the composer scrollbar exists exactly while overflowing, and placeholders are short (board #34)', async () => {
  // hidden → auto → hidden: the base CSS state is hidden (an empty composer
  // never shows a track), growComposer flips it in its ONE measurement — the
  // same `scrollHeight > maxH + 1` verdict that drives the padding — so a
  // shrink or the post-send reset (growComposer re-runs on composerText)
  // lands back on hidden immediately.
  assert.match(source, /const overflowing = el\.scrollHeight > maxH \+ 1;/u, 'one verdict for scrollbar AND padding');
  assert.match(source, /el\.style\.overflowY = overflowing \? 'auto' : 'hidden';/u, 'the toggle rides that verdict');
  assert.match(source, /resize: none; overflow-y: hidden;/u, 'the base state is hidden');
  // Not hidden PERMANENTLY: a long message must really scroll — the .c-input
  // block declares overflow-y exactly once (the hidden base; auto comes only
  // from the JS toggle), and no masking the scrollbar.
  const cInput = source.match(/\n  \.c-input \{[^}]*\}/su)?.[0] ?? '';
  assert.equal([...cInput.matchAll(/overflow-y/g)].length, 1, 'one overflow-y in .c-input, the hidden base');
  assert.ok(!/\.c-input[^}]*scrollbar-width:\s*none/su.test(source), 'the real scrollbar is never masked away');

  // The placeholders name the reach and stop (the recipient MENU keeps the
  // explanatory small hints — hubToAllHint/hubToRoomHint still render there).
  const i18n = await readFile(new URL('../core/i18n.svelte.ts', import.meta.url), 'utf8');
  assert.equal([...i18n.matchAll(/hubComposerAll: 'Message every agent…',/g)].length, 1, 'EN all is short');
  assert.equal([...i18n.matchAll(/hubComposerRoom: 'Leave a note…',/g)].length, 1, 'EN room is short');
  assert.equal([...i18n.matchAll(/hubComposerAll: '发给所有 agent…',/g)].length, 1, 'zh all is short');
  assert.equal([...i18n.matchAll(/hubComposerRoom: '留一句话…',/g)].length, 1, 'zh room is short');
  assert.match(source, /<small>\{t\('hubToAllHint'\)\}<\/small>/u, 'the menu hint stays');
  assert.match(source, /<small>\{t\('hubToRoomHint'\)\}<\/small>/u, 'the menu hint stays');
});

test('leaving at the tail means returning to the tail — and ONLY then (board #38)', () => {
  // Tail intent survives a page switch through three joints, each pinned:
  // 1) every scroll event routes `following` through the ONE pure transition
  //    (visible + bottom gap), so a hidden page's layout noise cannot pollute
  //    it, and the rest of the handler stops off-screen;
  assert.match(source, /following = tailAfterScroll\(visible, following, feedEl \? bottomGap\(feedEl\) : 0\);\n\s*if \(!visible\) return;/u,
    'the scroll handler speaks the transition rule, then stops when hidden');
  assert.match(source, /const atBottom = \(\) => !feedEl \|\| bottomGap\(feedEl\) < TAIL_GAP;/u,
    'atBottom is the same gap measure — no second definition of the tail');
  assert.ok(!/scrollHeight - feedEl\.scrollTop - feedEl\.clientHeight/u.test(source),
    'no inline gap math survives outside the pure helper');
  // 2) the physical jump is DEFERRED while hidden (data updates freely);
  assert.match(source, /if \(!feedEl \|\| !visible\) return;\n\s*feedEl\.scrollTop = feedEl\.scrollHeight;/u,
    'scrollFeed defers the physical scroll off-screen');
  // 3) the visible-restore effect settles Svelte, then forces the tail — but
  //    NEVER for a reader parked in history.
  const restore = source.match(/let hubWasVisible = false;\n\s*\$effect\(\(\) => \{[\s\S]*?\}\);/u)?.[0] || '';
  assert.match(restore, /if \(!visible\) \{ hubWasVisible = false; return; \}/u, 'hidden re-arms the restore');
  assert.match(restore, /if \(hubWasVisible\) return;/u, 'restore fires on the false→true edge only');
  assert.match(restore, /if \(!following\) return;/u, 'a history reader is never yanked to the bottom');
  assert.match(restore, /settled\(\)\.then\(\(\) => scrollFeed\(true\)\);/u,
    'settle first; scrollFeed(true) then rAFs, re-seeds the ask anchor and marks seen');
  // Project switch keeps its own forced tail (untouched by this feature).
  assert.match(source, /following = true;\n\s*if \(feed\.length\) scrollFeed\(true\);/u,
    'entering a room still lands at its tail');
});

test('long-press on a message is the system selection gesture, never our menu (board #48)', () => {
  // Owner: "在chat页面长按消息选中文字的时候 不应该出现选项卡 应该走手机系统本身
  // 默认选中文字的逻辑" (2026-09-01). On Android a long-press over text fires
  // contextmenu; the bubble's handler must route through the pure gate and
  // RETURN — without preventDefault — for a touch-sourced event, so the
  // native selection proceeds. A mouse right-click keeps the menu.
  assert.match(source,
    /oncontextmenu=\{\(e\) => \{ if \(touchContextMenu\(e\.pointerType\)\) return; e\.preventDefault\(\); openCtx\(pointOf\(e\), m\.from, msgItems\(m\)\); \}\}/u,
    'the bubble contextmenu yields to the system on touch, before preventDefault');
  // The selection's tail click must not toggle the action row — the same
  // isCollapsed guard the Board notes wear (#46). Without it, finishing a
  // selection ALSO pops the overlay the owner just asked to keep away.
  assert.match(source,
    /onclick=\{\(\) => \{ if \(typeof getSelection === 'function' && !\(getSelection\(\)\?\.isCollapsed \?\? true\)\) return; msgOpen = msgOpen === key \? '' : key; \}\}/u,
    'a non-collapsed selection swallows the bubble tap');
  // For the system gesture to have anything to select, the message body must
  // be selectable at all — the app shell's global user-select:none reaches it
  // otherwise (the Board's .n-text re-enable is the precedent).
  assert.match(source, /\.m-body \{[^}]*user-select: text;[^}]*-webkit-user-select: text;/u,
    'bubble text is selectable — both vendor forms, like the Board notes');
});

test('the fold measures its own line — perLine is never assumed (board #53 review)', () => {
  // Lead blocker: foldBody passed elideTail's default 80 at every width; at
  // 420px the real line is ~38 latin glyphs, so wrap was under-priced ~2.1×
  // and a folded bubble showed ~8 lines on a 4-line budget. The fold now
  // measures BOTH halves and maps them through the one pure function.
  assert.match(source, /const heldPerLine = \$derived\(perLineOf\(heldWidth, heldGlyph\)\);/u,
    'perLine derives through the pure mapping, no inline math');
  assert.match(source, /const foldBody = \(text\) => elideTail\(text, heldLines, heldPerLine\);/u,
    'foldBody spends the MEASURED perLine, not the default');
  // The width half: the feed content box × the --msg-max cap minus bubble
  // padding — computed with the SAME constants the CSS declares, and this
  // pin holds calc and CSS together so neither drifts alone.
  assert.match(source, /Math\.min\(feedW \* 0\.84, 1360\) - padX/u,
    'the calc speaks 84% / 1360px');
  assert.match(source, /--msg-max: min\(84%, 1360px\);/u,
    'and the CSS still declares the same pair');
  // The glyph half: the bubble font's own average, measured once per font on
  // a cached canvas — never a hardcoded px.
  assert.match(source, /gctx\.measureText\(sample\)\.width \/ sample\.length/u,
    'average glyph is measured from a representative sample');
  assert.match(source, /glyphCache\.get\(font\)/u, 'and cached per font string');
  // Both measurements ride the existing measureHeld path, so the resize
  // re-cut still routes through the reading anchor (2026-08-27 rule).
  assert.match(source, /const onResize = \(\) => withReadingAnchor\(measureHeld\);/u,
    'resize re-measures through the reading anchor, unchanged');
});

test('the anchor transaction re-measures the fold line it is about to restore (board #46, second blocker)', () => {
  // The drawer regrids the columns WITHOUT a window resize, so the resize
  // listener never fires and heldPerLine kept the OLD width until some later
  // blocks tick — a drawer toggle could leave every folded message over
  // budget (lead, 2026-09-01). The re-measure is part of the transaction
  // itself, in ORDER: mutate → settled (new grid) → measureHeld (new width/
  // glyph → heldPerLine → folds re-cut) → settled (re-cut rendered) → only
  // then take the tail or restore the reference offset. Both branches.
  const tx = source.match(/async function withReadingAnchor\(mutate\) \{[\s\S]*?\n  \}/u)?.[0] || '';
  const step = String.raw`(?:\s*//[^\n]*\n)*\s*`; // why-comments allowed between steps, order is not
  assert.match(tx, new RegExp(String.raw`mutate\(\);\n\s*await settled\(\);\n${step}measureHeld\(\);\n${step}await settled\(\);\n${step}scrollFeed\(true\);`, 'u'),
    'the FOLLOWING branch re-measures and lets folds re-cut before taking the tail');
  assert.match(tx, new RegExp(String.raw`mutate\(\);\n\s*await settled\(\);\n${step}measureHeld\(\);\n${step}await settled\(\);\n${step}if \(ref\?\.isConnected\)`, 'u'),
    'the HISTORY branch re-measures and lets folds re-cut before restoring the offset');
  // The width halves live in measureHeld, so ONE call is the whole re-read —
  // no second measurement dialect inside the transaction.
  assert.ok(!/withReadingAnchor[\s\S]*?heldWidth =/u.test(tx), 'the transaction never writes measurements directly');
});

test('the agent-card menu opens on the CARD’s left edge (board #47)', () => {
  // Owner: "点击 agent 卡片，出来的选项卡应该和 agent 卡片左边缘对齐，而不是
  // 右边缘对齐" (2026-09-01). The tap menu anchors on the card rect
  // (toggleAgentMenu → anchorOf) and menuPos passes the LEFT alignment to the
  // shared menuPlacement — the same reading the title menu chose in #32, same
  // flip and clamp, so the two alignments cannot drift apart. The right-click/
  // long-press context menu stays pointer-anchored (pinned in the #32 test).
  assert.match(source, /menuPlacement\(menuAnchor, \{ w: menuW, h: menuH \}, viewBox\(\), 6, 8, 'left'\)/u,
    'the card tap menu is left-aligned via the shared placement math');
});
