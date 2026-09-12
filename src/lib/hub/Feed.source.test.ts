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

const source = await readFile(new URL('./Feed.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('Feed exports three reading operations and owns its existing DOM lifecycle (#134)', () => {
  assert.deepEqual([...source.matchAll(/export (?:async )?function (\w+)/gu)].map((m) => m[1]).sort(),
    ['resetForRoom', 'scrollToTail', 'withReadingAnchor']);
  assert.match(source, /following = \$bindable\(true\), newBelow = \$bindable\(false\)/u);
  assert.match(source, /export function resetForRoom\(\) \{\s*reading = null;\s*expanded = \{\};\s*msgOpen = '';\s*rawOpen = '';\s*\}/u,
    'the resize snapshot now clears with the original room-local choices (#135)');
  assert.doesNotMatch(source, /hubPrefs|core\/ws|popstate|pushState|backLayers/u);
  assert.match(source, /registerActions\?\.\(\{\s*isOpen: \(\) => !!msgOpen,/u);
  assert.match(source, /if \(msgOpen && !t\?\.closest\?\.\('\.m-acts, \.bubble'\)\) msgOpen = '';/u);
  assert.match(source, /escape: \(e\) => \{ if \(msgOpen\) \{ msgOpen = ''; e\.stopPropagation\(\); \} \}/u);
  assert.doesNotMatch(source, /window\.addEventListener\('(?:keydown|pointerdown)'/u,
    'capture installation remains in Hub');
  assert.match(source, /<div class="feed-wrap">\s*<div class="feed subtle-scroll"/u);
  assert.match(source, /\.feed > :global\(\*\) \{ flex: none; \}/u,
    'all direct rows, including the parent empty snippet, retain non-shrinking layout');
  assert.match(source, /\{@render emptyFeed\?\.\(\)\}/u);
  assert.match(/\n  \.feed \{([^}]*)\}/u.exec(source)?.[1] ?? '', /overflow-anchor: none/u);
  assert.match(source, /getComputedStyle\(row\)\.position !== 'sticky'/u,
    'no pinned bubble or fixed filter label can be a reading reference (#135)');
});

test('resize restores a retained layout snapshot through one observer and one transaction (#135)', () => {
  assert.match(source, /import \{ boxFromOffsets \} from '\.\.\/ui\/indicator\.ts';/u);
  assert.match(source, /boxFromOffsets\(row, feedEl\)\.offsetTop/u,
    'cached positions exclude intro/press transforms using the existing helper');
  assert.match(source, /ro\.observe\(feedEl\);/u);
  assert.match(source, /ro\.disconnect\(\);/u);
  assert.doesNotMatch(source, /window\.(?:add|remove)EventListener\('resize'/u);
  assert.match(source, /if \(before && sameReadingSize\(before\.size, size\)\) return;/u,
    'an explicit transaction already committed this size; RO must not repeat it');
  assert.match(source, /void withReadingAnchor\(measureHeld, before\);/u);
  assert.match(source, /if \(visible && resizePending\(\)\) return;/u,
    'a resize-generated scroll cannot overwrite the old reading or tail intent');
  assert.match(source, /if \(!acceptResize && reading\?\.session === selected && !sameReadingSize\(reading\.size, size\)\) return;/u);
  assert.match(source, /Object\.values\(stepsChoice\); Object\.values\(stepsAll\);/u);
  // #137 deliberately extends resource completion: settle the image view,
  // reapply live tail through its existing writer, otherwise remember history.
  assert.match(source, /onloadcapture=\{onContentLoad\} onerrorcapture=\{onContentLoad\}/u);
  const resource = source.slice(source.indexOf('async function onContentLoad()'), source.indexOf('function resizePending()'));
  assert.match(resource, /const element = feedEl, session = selected, tail = following;/u);
  assert.match(resource, /await settled\(\);\s*if \(feedEl !== element \|\| selected !== session\) return;/u);
  assert.match(resource, /if \(tail && following\) writeTail\(\);\s*else queueReadingCapture\(\);/u);
  assert.match(source, /if \(!feedEl \|\| !visible\) return;/u);
  assert.match(source, /const current = \(\) => feedEl === element && selected === session;/u);
  assert.match(source, /before\.ref\.parentElement === feedEl/u);
});

test('Feed preserves reveal gates and consumes the shared rendering and path mechanisms (#134)', () => {
  assert.match(source, /\{#if selected && !roomReady\}\s*<div class="skel-wrap sk-feed" aria-hidden="true">/u);
  assert.match(source, /class="feed subtle-scroll" class:reveal-tail=\{justLoaded\}/u);
  assert.match(rule('.sk-feed'), /margin-top: auto/u);
  assert.match(source, /class="older-hint"/u, 'top-of-scrollback feedback stays in the view');
  assert.match(source, /class="older-hint older-more" onclick=\{loadOlder\}/u,
    'the short-page entry calls the same parent walk as the scroll handler');
  assert.match(source, /if \(top < 120 && roomReady\) loadOlder\(\);/u);
  assert.match(source, /\.m-state\.note \.st \{ width: 9px; height: 9px; \}/u);
  assert.doesNotMatch(source, /^\s*\.(st|note-dot|live-dot) \{/mu);
  assert.match(source, /import \{ renderMarkdown \} from '\.\.\/core\/markdown\.ts';/u);
  assert.match(source, /import 'katex\/dist\/katex\.min\.css';/u);
  assert.doesNotMatch(source, /marked\.parse|new Marked/u);
  assert.match(source, /handlePathLinkClick\(e, routePathRef\)/u);
  assert.match(source, /onauxclick=\{openPathRef\}/u);
  assert.match(source, /<ChatImage \{src\} alt=\{m\.from\} onview=\{onimage\} \/>/u);
});

test('the feed layers remain below the Composer stacking context', () => {
  // Board #1: the recipient menu opened UNDER a pinned bubble — .to-wrap's own
  // z-index:2 capped it below .ask-top's 6. The rule is decided ONCE at the
  // composer: it is a stacking context whose level beats the feed's layers
  // (pinned 6, actions overlay 8), so anything opening out of it — the
  // recipient menu, the / palette — wins without per-popover arithmetic.
  // Composer.source.test pins the other half at 15 after #133.
  const z = (css: string) => Number(/z-index:\s*(\d+)/u.exec(css)?.[1] ?? NaN);
  for (const sel of ['.msg.ask-top', '.msg.ask-bottom']) {
    const feedZ = z(rule(sel));
    assert.ok(Number.isFinite(feedZ), `${sel} still declares a z-index`);
    assert.ok(15 > feedZ, `Composer (15) must stack above ${sel} (${feedZ})`);
  }
});

test('the agent filter remains visible and leavable inside the feed (board #3)', () => {
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
  assert.match(source, /onclick=\{onclearfilter\}/u, 'the banner requests the parent-owned filter exit');
});

test('compact tail padding contains the floating command targets without opening-time layout changes (#166)', () => {
  const compact = /:global\(\.hub-root\.compact\) \.feed \{([^}]+)\}/u.exec(source)?.[1] ?? '';
  assert.match(compact, /padding: 14px 10px max\(6px, calc\(var\(--control-height\) \/ 2\)\)/u,
    'the absolute action overhang is reserved even before the row opens');
  assert.doesNotMatch(compact, /msgOpen/u);
});

test('a Han-bearing bubble declares its OWN language (board #97, round two)', () => {
  // A Chinese message in an ENGLISH UI drew Japanese-variant glyphs (骨/直/
  // 门…): glyph variants of a fallback face follow the nearest lang, and the
  // document said `en`. The content tags itself, UI language notwithstanding.
  assert.match(source, /<div class="m-body" lang=\{hanLang\(m\.body \?\? ''\)\}>/u,
    'the bubble body carries hanLang of its own text');
  assert.match(source, /import \{ t, i18n, hanLang \} from '\.\.\/core\/i18n\.svelte\.ts';/u,
    'the ONE tagger from i18n — no second Han detector');
});

test('the fold budget goes through foldLines, and the basis is the column', () => {
  // Board #4: the budget math lives in hub.ts (pure, tested) — Feed only
  // measures. Re-inlining `* 0.2` here would fork the mapping again, and
  // measuring the FEED instead of its parent is the composer-shrink bug
  // (owner, 2026-08-27) coming back.
  assert.match(source, /const heldLines = \$derived\(foldLines\(compact, heldBasis, heldLine\)\);/u,
    'one derivation, the pure mapping');
  assert.match(source, /heldBasis = feedEl\.parentElement\?\.clientHeight/u,
    'the basis is the chat COLUMN (the feed parent), which the composer cannot shrink');
  const script = source.slice(0, source.indexOf('</script>'));
  assert.ok(!/\*\s*0\.2/u.test(script), 'no inline fifth — the fraction lives in foldLines only');
  assert.match(source, /void withReadingAnchor\(measureHeld, before\);/u,
    'a real window resize still re-enters through the reading anchor');
});

test('reading decisions use the pure module while Feed owns DOM and scheduling (#116)', () => {
  // Parameterizing decisions must not move layout reads or reset the held state
  // on a jump: reset only affects pickAnchor's seed, as before the extraction.
  assert.match(source, /import \{ heldAnchor, readingDirection, refoldEligible, sameReadingSize \} from '\.\/hub-reading\.ts';/u);
  const scroll = source.slice(source.indexOf('function onFeedScroll'), source.indexOf('let scrollFrame'));
  assert.match(scroll, /const motion = readingDirection\(delta, askDir, askDirTravel\);\s*askDir = motion\.direction;\s*askDirTravel = motion\.travel;\s*syncAsk\(askDir\);/u,
    'the rAF callback applies direction before choosing the anchor');
  assert.match(scroll, /scrollFrame = requestAnimationFrame/u);
  const refold = source.slice(source.indexOf('function autoRefold'), source.indexOf('function syncAsk'));
  assert.match(refold, /el instanceof HTMLElement\s*&& refoldEligible\(\{ top: el\.offsetTop, height: el\.offsetHeight \}, top, bottom\)/u,
    'Feed owns missing-element checks and supplies real layout boxes');
  assert.match(refold, /withReadingAnchor\(\(\) => \{ expanded = next; \}\)/u);
  const sync = source.slice(source.indexOf('function syncAsk'), source.indexOf('// Coming back to the page'));
  assert.match(sync, /reset \? undefined : \{ key: askKey, edge: askEdge \}/u,
    'pickAnchor is still the sole selector, with the original jump reset');
  assert.match(sync, /heldAnchor\(picked, chosen,\s*\{ key: askKey, edge: askEdge, held: askHeld \},\s*feedEl\.scrollTop, feedEl\.scrollTop \+ feedEl\.clientHeight\)/u,
    'held hysteresis receives the existing state even after a jump');
  assert.match(sync, /askKey = anchor\.key;\s*askEdge = anchor\.edge;\s*askHeld = anchor\.held;/u);
  const neutralize = sync.indexOf("el.style.position = 'static'");
  const restore = sync.indexOf("removeProperty('position')");
  assert.ok(neutralize >= 0 && neutralize < sync.indexOf('const items ='));
  assert.ok(restore > neutralize && restore < sync.indexOf('const picked = pickAnchor('),
    'sticky neutralization ends before the pure decisions');
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
  assert.match(cmd, /var\(--font-mono\)/u);
  assert.match(cmd, /var\(--code-bg\)/u, 'the wash is the inline-code dialect, not a drawn frame');
  assert.doesNotMatch(cmd, /border:/u, 'no border on the command capsule');
  assert.match(cmd, /text-overflow:\s*ellipsis/u, 'a long command clips itself, not its neighbours');
  assert.doesNotMatch(source, /class="sys-verb cmd"/u, 'no second command dialect');
  // Per-row ellipsis lives on the text, so a long detail cannot eat the badge.
  assert.match(rule('.sysline .sys-text'), /text-overflow:\s*ellipsis/u);
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
  assert.match(source, /<button class="sys-item sys-jump"[\s\S]{0,300}?onboard\(Number\(bl\.id\)\)/u,
    'tapping a board line opens that issue');
});

test('a delivered prompt sheds its stamp and board deliveries wear the dialect (board #18)', () => {
  assert.match(source, /\{@const pp = promptParts\(b\.text\)\}/u, 'every prompt row goes through the reader');
  assert.match(source, /\{#if pp\.from\}<span class="p-from">\{pp\.from\}<\/span>\{\/if\}/u,
    'the sender joins the head — the stamp never renders');
  assert.match(source, /<span class="p-chip">#\{pp\.board\.id\}<\/span>/u, 'the issue chip');
  // Three shapes, ONE badge (board #172): review and reply both wear the
  // existing badge coloured by boardStatusColor — `reply` is not a status,
  // so it takes that function's default reading ink, no new colour.
  assert.match(source, /\{#if pp\.board\.tag\}<span class="p-badge" style:color=\{boardStatusColor\(pp\.board\.tag\)\}/u,
    'the badge speaks the one status language for every tagged shape');
  assert.doesNotMatch(source, /boardStatusColor\('review'\)/u, 'no shape is special-cased in the markup');
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
    'scrollToTail defers the physical scroll off-screen');
  // 3) the visible-restore effect settles Svelte, then forces the tail — but
  //    NEVER for a reader parked in history.
  const restore = source.match(/let hubWasVisible = false;\n\s*\$effect\(\(\) => \{[\s\S]*?\}\);/u)?.[0] || '';
  assert.match(restore, /if \(!visible\) \{ hubWasVisible = false; return; \}/u, 'hidden re-arms the restore');
  assert.match(restore, /if \(hubWasVisible\) return;/u, 'restore fires on the false→true edge only');
  assert.match(restore, /if \(!following\) return;/u, 'a history reader is never yanked to the bottom');
  assert.match(restore, /settled\(\)\.then\(\(\) => scrollToTail\(true\)\);/u,
    'settle first; scrollToTail(true) then rAFs, re-seeds the ask anchor and marks seen');
});

test('a tapped bubble reveals Copy/Raw under it — and still never an app context menu (board #48)', () => {
  // The owner's arc: "长按消息选中文字的时候 不应该出现选项卡" (2026-09-01) —
  // a touch hold is the SYSTEM's selection gesture; 2026-09-04 retired the
  // action row together with the right-click card; then 2026-09-07: "我只要
  // 消息气泡下边的这两个按钮，不要出现右键那种选项卡" — the tap-revealed
  // .m-acts row (Copy / Raw) RETURNS, the ContextMenu card stays retired.
  const bubble = source.slice(source.indexOf('<div class="bubble md"'),
    source.indexOf('<div class="m-body"'));
  // A plain tap toggles the row — after the path-reference route (board #99:
  // a tapped file link opens the preview, it must not ALSO flip the row), the
  // one-shot compatibility-click guard (a long-press's echo click) and the
  // live-selection fallback.
  assert.match(bubble,
    /onclick=\{\(e\) => \{ if \(openPathRef\(e\)\) return; if \(msgSelectionClicks\.consume\(key\)\) return; if \(typeof getSelection === 'function' && !\(getSelection\(\)\?\.isCollapsed \?\? true\)\) return; msgOpen = msgOpen === key \? '' : key; \}\}/u,
    'the synthetic click is consumed before the selection fallback and the row toggle');
  // The contextmenu handler only MARKS a touch-owned hold: no preventDefault,
  // no menu — native selection proceeds on touch, and a mouse right-click is
  // a no-op (the App-level guard suppresses browser chrome).
  assert.match(bubble, /oncontextmenu=\{\(e\) => \{ msgSelectionClicks\.mark\(e, key\); \}\}/u,
    'contextmenu stays native — it only arms the compatibility-click guard');
  assert.ok(!bubble.includes('openCtx'), 'a bubble never opens the ContextMenu card');
  assert.ok(!source.includes('msgItems'), 'no message verbs feed any context menu');
  // The row itself: the shared .m-acts atom with exactly Copy and Raw.
  assert.match(source, /\{#if msgOpen === key\}\n\s*<div class="m-acts appear">/u,
    'the action row is the shared .m-acts overlay, revealed per message');
  assert.match(source, /copyMsg\(m\.body\)/u, 'Copy writes the raw body');
  assert.match(source, /rawOpen = rawOpen === key \? '' : key/u, 'Raw toggles the source view');
  assert.match(source, /<pre class="raw">\{m\.body\}<\/pre>/u, 'raw view shows the bytes as written');
  // For the system gesture to have anything to select, the message body must
  // be selectable at all — the app shell's global user-select:none reaches it
  // otherwise (the Board's .n-text re-enable is the precedent).
  assert.match(source, /\.m-body \{[^}]*user-select: text;[^}]*-webkit-user-select: text;/u,
    'bubble text is selectable — both vendor forms, like the Board notes');
  // The time trailer is a real button again: the accessible route to the row
  // (the bubble is TEXT to assistive tech, its click a pointer convenience).
  assert.match(source, /<button class="m-meta" aria-label=\{t\('hubMsgActions'\)\}/u,
    'the meta trailer is the accessible actions trigger');
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
  assert.match(source, /max-width: var\(--msg-max\)/u,
    'the view consumes the existing parent width token; Hub.source pins its value');
  // The glyph half: the bubble font's own average, measured once per font on
  // a cached canvas — never a hardcoded px.
  assert.match(source, /gctx\.measureText\(sample\)\.width \/ sample\.length/u,
    'average glyph is measured from a representative sample');
  assert.match(source, /glyphCache\.get\(font\)/u, 'and cached per font string');
  // Both measurements ride the existing measureHeld path, so the resize
  // re-cut still routes through the reading anchor (2026-08-27 rule).
  assert.match(source, /void withReadingAnchor\(measureHeld, before\);/u,
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
  const tx = source.match(/async function withReadingAnchor\(mutate, before = null\) \{[\s\S]*?\n  \}/u)?.[0] || '';
  const step = String.raw`(?:\s*//[^\n]*\n)*\s*`; // why-comments allowed between steps, order is not
  const settledGuard = String.raw`await settled\(\);\n\s*if \(!current\(\)\) return;\n`;
  assert.match(tx, new RegExp(String.raw`mutate\(\);\n\s*${settledGuard}${step}measureHeld\(\);\n\s*${settledGuard}${step}writeTail\(\);`, 'u'),
    'the FOLLOWING branch re-measures and lets folds re-cut before taking the tail');
  assert.match(tx, new RegExp(String.raw`mutate\(\);\n\s*${settledGuard}${step}measureHeld\(\);\n\s*${settledGuard}${step}if \(before\?\.ref\?\.isConnected`, 'u'),
    'the HISTORY branch re-measures and lets folds re-cut before restoring the offset');
  // The width halves live in measureHeld, so ONE call is the whole re-read —
  // no second measurement dialect inside the transaction.
  assert.ok(!/withReadingAnchor[\s\S]*?heldWidth =/u.test(tx), 'the transaction never writes measurements directly');
});

test('feed-lane identity comparisons match the window NAME, never the pane index (board #123)', async () => {
  const source = await readFile(new URL('./Feed.svelte', import.meta.url), 'utf8');
  // Board #120 made a feed block's `window` the agent NAME (a string), while
  // HubAgent.window stays the numeric tmux index for terminal targeting.
  // Two comparisons kept matching name-against-number — always false — so
  // every lane label fell back to its costume and the tool lane's live pulse
  // never showed. The lane lookups must compare a.name.
  assert.match(
    source,
    /const windowName = \(w\) => agents\.find\(\(a\) => a\.name === w\)/u,
    'windowName matches on the name',
  );
  assert.match(
    source,
    /includes\(agents\.find\(\(a\) => a\.name === b\.window\)\?\.state\)/u,
    'isRunning matches on the name',
  );
  // The numeric index may not creep back into either lookup.
  assert.doesNotMatch(source, /a\.window === b\.window/u, 'no index-vs-name comparison in the feed lanes');
});

test('a prompt row folds through the ONE message mechanism — never a silent clip (board #172)', async () => {
  const source = await readFile(new URL('./Feed.svelte', import.meta.url), 'utf8');
  // Owner, 2026-09-11 ("有消息没有渲染"): the input row for a 601-char board
  // notice stopped mid-sentence after ~6 lines. `.p-body` carried its own
  // `max-height: 7.5em; overflow: hidden` — a second, SILENT fold beside
  // foldLines/elideTail, with no marker and no way to the rest. The row
  // now folds the TEXT like a user bubble and shows the same unfold control.
  const pBody = /\.p-body \{[^}]*\}/u.exec(source)?.[0] ?? '';
  assert.ok(pBody, 'the prompt body rule exists');
  assert.doesNotMatch(pBody, /max-height/u, 'no height cap on the prompt body');
  assert.doesNotMatch(pBody, /overflow: hidden/u, 'no clip on the prompt body');
  assert.match(source, /\{folded \? foldBody\(pp\.text\) : pp\.text\}/u,
    'the prompt text is cut by elideTail before render, and rendered WHOLE once unfolded');
  // One unfold control, rendered from ONE snippet in both the bubble and
  // the prompt row — a second button markup would be a second mechanism.
  assert.equal((source.match(/class="m-unfold"/gu) ?? []).length, 1, 'the unfold button markup exists once');
  assert.equal((source.match(/\{@render unfold\(key, folded\)\}/gu) ?? []).length, 2,
    'the bubble and the prompt row both render it');
});
