import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Roster.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`(?:^|\\n)\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('cards, Add and disclosure share the strip centre without moving the expanded control (#186)', () => {
  assert.match(rule('.cards'), /align-items: center/u);
  const toggle = rule('.roster-toggle');
  assert.match(toggle, /align-items: center/u);
  assert.match(toggle, /height: calc\(var\(--roster-paint-height\) \+ 2 \* var\(--control-paint-inset\)\)/u,
    'the disclosure wrapper matches the card target height, without a spare row above it');
  assert.match(rule('.cards'), /padding: 0 2px/u,
    '#237: the old top scrollport inset is gone, while the 44px touch target stays intact');
  assert.match(toggle, /align-self: end/u, 'expanding upward keeps the collapse control reachable in place');
  assert.doesNotMatch(toggle, /padding-block-end/u);
});

test('context meters surround equal circular avatars with room inside the card (#180)', () => {
  assert.match(source, /style:--ctx-color=\{ctxColor\(a\.vitals\.context_pct\)\}/u);
  assert.match(rule('.acard'), /position: relative/u);
  const bar = rule('.ctx-ring');
  assert.match(bar, /position: absolute/u);
  assert.match(bar, /left: 0; top: 0/u, 'the ring is positioned in the same box as the icon, not independently in the card');
  assert.match(rule('.avatar-slot'), /position: relative/u);
  assert.match(bar, /width: var\(--roster-ring-size\); height: var\(--roster-ring-size\)/u);
  assert.match(rule('.roster'), /--roster-ring-size: 24px/u);
  assert.match(rule('.roster'), /--roster-paint-height: 30px/u);
  assert.match(source, /\.roster \{ --roster-paint-height: 32px; \}/u,
    '#237: the phone row is 32px — the 44px touch floor lives on .agent-select, not the paint');
  assert.match(bar, /pointer-events: none/u);
  assert.match(bar, /conic-gradient/u);
  assert.doesNotMatch(bar, /transition/u, 'colour thresholds never pass through intermediate hues');
  assert.match(rule('.ava'), /border-radius: 50%/u);
  assert.doesNotMatch(source, /ac-bar|roster-meter-height/u, 'edge bar removed whole');
});

test('everyone leaves the roster whole for the sole Composer command (#180)', async () => {
  assert.doesNotMatch(source, /acard all|acard\.all|all-ava|allPending|broadcast-glyph/u);
  const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /\.hub-root \.acard\.all/u, 'the obsolete corner exception leaves with the old card');
});

test('card paint keeps its inset and the single measured scrolling edge (#180)', () => {
  assert.match(rule('.acard::before'), /inset: var\(--control-paint-inset\) 0/u);
  assert.match(rule('.acard::before'), /pointer-events: none/u);
  // The tab strip has only its scrollport inset; the field retains its inset.
  assert.match(rule('.roster'), /padding: 0 var\(--composer-inset\) 0 0/u);
  assert.match(rule('.roster'), /gap: 0/u);
  assert.match(source, /class="cards edge-fade"[^>]*use:scrollEdges=\{!expanded\}/u);
});

test('no invisible action slots widen the card; Stop stands ON the dot instead (#180 → #205)', () => {
  assert.doesNotMatch(source, /agent-watch|card-quick|onwatch/u);
  assert.doesNotMatch(rule('.agent-stop'), /opacity: 0/u, 'never an invisible reserved slot');
  assert.match(rule('.agent-stop'), /position: absolute; z-index: 1; left: var\(--dot-x, 50%\); top: var\(--dot-y, 50%\); transform: translate\(-50%, -50%\)/u,
    '#205: placed over the dot by overDot, taking no width — the card never changes size');
  assert.match(rule('.acard.stop-shown .ac-top'), /visibility: hidden/u, 'the dot yields while the Stop stands on it');
  assert.doesNotMatch(rule('.acard'), /grid-template-columns/u);
  assert.match(source, /icon="stop" variant="danger" iconOnly/u);
});

test('Stop is red, hidden until hover/focus or a pending interrupt, and never on touch (board #205)', () => {
  // Owner 2026-09-20: "终止按钮应该是红色的吧，更符合语义，而且默认不显示，只有鼠标移到上边，
  // 把状态的小圆点变为终止按钮。手机端就不要了，节省空间，让用户用选项卡终止就好".
  assert.match(source, /const coarse = typeof window !== 'undefined' && window\.matchMedia\('\(any-pointer: coarse\)'\)\.matches;/u);
  assert.match(source, /const showStop = \(name\) => !coarse && busyNames\.includes\(name\) && \(armed === name \|\| interrupting\.includes\(name\)\);/u);
  assert.match(source, /function arm\(name, pointerType = 'mouse'\) \{ if \(!coarse && pointerType !== 'touch'\) armed = name; \}/u);
  assert.match(source, /onpointerenter=\{\(e\) => arm\(a\.name, e\.pointerType\)\} onpointerleave=\{\(\) => disarm\(a\.name\)\}/u);
  assert.match(source, /onfocusin=\{\(\) => arm\(a\.name\)\} onfocusout=\{\(e\) => \{ if \(!e\.currentTarget\.contains\(e\.relatedTarget\)\) disarm\(a\.name\); \}\}/u);
  assert.match(source, /\{#if showStop\(a\.name\)\}/u);
  assert.match(source, /class="agent-stop compact-tools" class:pending use:overDot/u);
  assert.doesNotMatch(source, /renderedStops|pressStops|has-stop/u, 'the resident slot and its press-hold layout are gone');
});

test('the dot reserves the Stop\'s room, constant, and the Stop wears no wash (board #211)', () => {
  // Owner 2026-09-20: "agent 卡片状态小点右侧好像没有留边距，包括我鼠标悬浮的时候，停止按钮都
  // 超出 agent 卡片框了。而且停止按钮就不用加背景了，就红色方块我直接点就行".
  // The 28px slot (hit box = focus-ring extent; 20px paint inside) centred on
  // the 6px dot reaches 14px past its centre; 12px = 28/2 − 6/2 + the 1px
  // inset border ends the slot at the border's inner edge.
  assert.match(rule('.roster'), /--roster-dot-reserve: 12px/u);
  assert.match(rule('.acard:not(.off) .agent-select'), /padding-inline-end: var\(--roster-dot-reserve\)/u,
    'a live card reserves it at rest — revealing the Stop must not reflow the row');
  assert.doesNotMatch(rule('.agent-select'), /padding-inline-end/u, 'a stopped card has no dot and keeps the plain inset');
  assert.match(source, /icon="stop" variant="danger" iconOnly bare/u, 'no rest/hover/press paint: the red glyph is the control');
});

test('one controlled roster replaces the delayed tap menu whole (#168)', () => {
  assert.match(source, /import \{ ALL_TARGET \} from '\.\/hub-composer\.ts'/u);
  assert.match(source, /const ranked = \$derived\(sortAgentsForRoster\(managedAgents\)\)/u,
    'turn-edge recency stays the one sorting authority');
  assert.match(source, /const groups = \$derived\(rosterGroups\(orderedAgents\)\)/u,
    'grouping follows the hover-held order instead of re-sorting identities');
  assert.match(source, /onselect: setRecipient = \(\) => \{\}/u);
  assert.match(source, /function selectTarget\(name\) \{\s*setRecipient\(name\);/u, '#196: a click selects; Record only in the menu deselects');
  assert.doesNotMatch(source, /selectTarget\(ALL_TARGET\)/u);
  assert.match(source, /onclick=\{\(e\) => clickAgent\(e, a\.name\)\}/u);
  assert.doesNotMatch(source.slice(source.indexOf('} = $props();')), /\brecipient\s*=(?!=)/u,
    'selection emits intent without locally committing recipient');
  assert.doesNotMatch(source, /menuFor|bind:cardsEl|cardTimer|cardDbl|cardClick|toggleAgentMenu|menuAnchor|menuPos|vitalsFor|menuAgent|a-menu|am-who|am-vitals|groupRoster|tgroup/u);
  // Owner 08:41 (#173) adds Watch as a direct intent, not a local action menu.
  assert.doesNotMatch(source, /onstart|onrestart|onaction|onconfigure|setTimeout|addEventListener|popstate|history\./u);
});

test('Stop consumes parent busy and pending sets through the shared command', () => {
  assert.match(source, /busyNames = \[\], interrupting = \[\]/u);
  assert.match(source, /\{#if showStop\(a\.name\)\}/u);
  assert.match(source, /\{pending\} disabled=\{pending\}/u);
  assert.doesNotMatch(source, /interrupting\.includes\(ALL_TARGET\)/u,
    'pending contains captured member names, never a second all-job sentinel');
  assert.match(source, /interrupting\.includes\(a\.name\)/u);
  assert.equal([...source.matchAll(/icon="stop" variant="danger" iconOnly/gu)].length, 1);
  assert.equal([...source.matchAll(/e\.stopPropagation\(\); interrupt\(/gu)].length, 1);
  assert.doesNotMatch(source, /hubAgentStop|hubAgentInterrupt|busyTargetsFor|\.state\s*===\s*'(?:running|working|waiting|blocked)'/u,
    'the parent owns target membership and dispatch, not a second local busy classifier');
});

test('body mentions use chipExtras, separate from selected state and Stop', () => {
  assert.match(source, /const extras = \$derived\(chipExtras\(composerText, recipient, managedNames\)\)/u);
  assert.match(source, /extras\.includes\(ALL_TARGET\)/u);
  assert.match(source, /extras\.includes\(a\.name\)/u);
  assert.match(source, /class="agent-mention"[^>]*>@<\/span>/u);
  assert.doesNotMatch(source, /mentionTokens|new RegExp|\.split\(['"]@|name="check"/u);
  assert.doesNotMatch(rule('.agent-mention'), /border[^;\n]*dashed/u,
    '#173 permits a reading-filter outline, never a mention outline');
});

test('native selection owns hover and context; stopped slots never select or resume', () => {
  assert.match(source, /<button type="button" class="agent-select"\s+aria-pressed=\{isAddressed\(a\.name\)\}/u);
  assert.match(source, /const isAddressed = \(name\) => recipient === ALL_TARGET \|\| recipient === name/u);
  assert.match(source, /class:sel=\{isAddressed\(a\.name\)\}/u, '#186: paint and accessibility read the same aggregate selection');
  assert.match(source, /use:hoverInfo=\{\(\) => cardInfo\(a\)\}/u);
  assert.match(source, /use:hoverInfo=\{\(\) => offCardInfo\(name\)\}/u);
  // #223: a touch open carries the hover card's facts (a finger has no
  // hover); touchInfo returns null on a fine pointer, keeping those menus
  // verbs-only.
  assert.match(source, /const touchInfo = \(get\) => \(coarsePointer\(\) \? get\(\) : null\);/u);
  assert.match(source, /oncontextmenu=\{\(e\) => \{ e\.preventDefault\(\); oncontext\(pointOf\(e\), a\.name, touchInfo\(\(\) => cardInfo\(a\)\)\); \}\}/u);
  assert.match(source, /use:longpress=\{\{ onlongpress: \(at\) => oncontext\(at, a\.name, touchInfo\(\(\) => cardInfo\(a\)\)\) \}\}/u);
  const off = source.slice(source.indexOf('class="acard off"'), source.indexOf('{/each}', source.indexOf('class="acard off"')));
  assert.match(off, /onclick=\{\(e\) => stoppedMenu\(e, name\)\}/u);
  assert.match(off, /disabled=\{acting\}/u);
  assert.doesNotMatch(off, /selectTarget|interrupt\(|startAgent|a-start|aria-pressed/u);
  assert.doesNotMatch(source, /role="button"|onkeydown|position:\s*fixed/u);
  for (const fact of ['modelLabel(a.vitals.model)', 'fmtElapsed(a.since, tick)', 'stateTone(a.state)', "t('hubHoverTarget')", "t('hubHoverPath')", "t('hubToDmLong')", "t('hubToAlsoHint')"]) {
    assert.ok(source.includes(fact), `retained hover fact: ${fact}`);
  }
});

test('double-click focuses live agents without delay and uses the existing filter intent (#173)', () => {
  assert.match(source, /if \(!coarsePointer\(\) && event\.detail > 1\) return;/u);
  assert.match(source, /if \(!stopped && recipient !== name\) setRecipient\(name\);/u);
  assert.match(source, /onfilter\(name\);/u);
  assert.match(source, /anchor: anchorOf\(trigger\), align: 'left', trigger, keepTriggerClear: true/u);
  assert.match(source, /class:filtered=\{filterAgent === a\.name\}/u);
  assert.match(rule('.acard.filtered::after'), /border: 1px dashed var\(--text2\)/u);
  assert.match(rule('.acard.filtered::after'), /inset: var\(--control-paint-inset\) 0/u,
    'the compact filter border must stay outside the avatar instead of crossing its lower pixels');
  assert.doesNotMatch(source, /setTimeout|clearTimeout/u);
});

test('identity, readiness and motion retain their existing authorities', () => {
  assert.match(source, /\{#if selected\}\s*<div class="roster"/u);
  assert.match(source, /\{#if !roomReady\}\s*<div class="skel-wrap sk-cards" aria-hidden="true">/u);
  assert.match(source, /class:reveal=\{justLoaded\}/u);
  assert.match(source, /class:appear-pop=\{!!rosterBase && !rosterBase\.has\(a\.name\)\}/u);
  assert.match(source, /class:live-dot=\{stateIsLive\(a\.state\)\} style:background=\{stateDotColor\(a\.state\)\}/u);
  assert.doesNotMatch(source, /<span>\{stateLabel\(a\.state\)\}<\/span>|agent-state|stateNeedsYou/u,
    'owner 07:09 retains state wording in accessible/hover facts only');
  assert.match(source, /\{#each groups as group \(group\.key\)\}/u);
  assert.match(source, /\{#each group\.members as a \(a\.name\)\}/u);
  assert.equal([...source.matchAll(/animate:flip=\{\{ duration: moveMs\(\) \}\}/gu)].length, 3);
  assert.doesNotMatch(source, /^\s*\.st \{/mu, 'the shared Hub dot box is not copied');
  assert.match(source, /const slotBackend = \(name\) => \(selectedRow\?\.slots \?\? \[\]\)\.find\(\(s\) => s\.window_name === name\)\?\.command;/u);
  assert.match(source, /img class="ava dim" src=\{backendIcon\(backend\)\}/u);
  assert.match(rule('img.ava.dim'), /filter: grayscale\(1\); opacity: 0\.55/u);
  assert.match(source, /onclick=\{onadd\}/u);
});

test('density lives in local tokens; full names and native targets do not shrink', () => {
  const roster = rule('.roster');
  assert.match(roster, /--roster-avatar-size: 18px/u);
  assert.match(roster, /--roster-expanded-max: min\(240px, calc\(32dvh \/ var\(--ui-zoom, 1\)\)\)/u);
  assert.match(rule('.agent-select'), /min-height: var\(--control-height\)/u);
  assert.match(rule('.roster.compact .agent-select'), /padding-block-start: calc\(2 \* var\(--ui-gap\)\)/u,
    'move content and its shared ring box within the unchanged hit target');
  assert.match(rule('.roster.compact .all-choice :global(.command-button.icon-only)'), /padding-block-start: calc\(2 \* var\(--ui-gap\)\)/u,
    'the pinned All glyph follows the same compact tab alignment');
  assert.match(rule('.acard'), /width: max-content/u, 'only actual content sets card width');
  assert.match(rule('.cards'), /overflow-x: auto/u);
  assert.match(rule('.cards.expanded'), /overflow-y: auto/u);
  assert.match(rule('.cards.expanded'), /max-height: var\(--roster-expanded-max\)/u);
  assert.match(rule('.cards.expanded'), /flex-wrap: wrap/u);
  assert.doesNotMatch(rule('.cards.expanded .acard'), /width: auto/u);
  assert.match(rule('.cards.expanded .a-name'), /overflow-wrap: anywhere/u);
  assert.match(rule('.cards.expanded .a-name'), /padding-block: var\(--control-paint-inset\)/u);
  assert.match(rule('.agent-marks.unmarked'), /display: none/u);
  assert.doesNotMatch(source, /\.agent-marks:empty/u,
    'Svelte leaves conditional whitespace; CSS :empty kept 12.5px reserved and split ordinary phone names');
  assert.match(rule('.a-name'), /white-space: nowrap/u);
  assert.doesNotMatch(rule('.a-name'), /max-width|ellipsis|overflow: hidden/u);
  assert.doesNotMatch(source, /data-density|URLSearchParams|location\.search/u);
});

test('one in-flow disclosure controls one list without remounting its cards', () => {
  assert.equal([...source.matchAll(/class="cards edge-fade"/gu)].length, 1);
  assert.match(source, /icon="chevron-up" variant="icon"/u);
  assert.match(source, /\{expanded\} controls=\{cardsId\} disabled=\{!roomReady\} onclick=\{onexpand\}/u);
  assert.match(source, /const holdOrder = \$derived\(hovering \|\| focused \|\| pressing\)/u);
  assert.match(source, /onpointerdown=\{beginPress\} onpointerup=\{clearPress\} onlostpointercapture=\{clearPress\}/u);
  assert.doesNotMatch(source, /requestAnimationFrame|cancelAnimationFrame|setTimeout/u,
    'order releases on input lifecycle events, not a timer');
  assert.match(source, /focused = !!cardsEl\?\.contains\(document\.activeElement\)/u,
    'removed controls can lose focus without emitting focusout');
  assert.doesNotMatch(rule('.cards') + rule('.cards.expanded') + rule('.roster'), /transition:[^;\n]*(?:height|width)/u,
    'list geometry never animates; the absolute context fill may change width (#173)');
  assert.doesNotMatch(source, /@keyframes|position: fixed/u);
});

test('idle cards keep no padding for absent commands (#180)', () => {
  assert.doesNotMatch(source, /padding-right: calc|grid-column: 1 \/ 3/u);
  assert.match(rule('.agent-select'), /padding: 0 var\(--roster-card-inset\)/u);
});

test('the stop is a quiet dense action and the dot sits clear of the name, on the x-height (board #195)', () => {
  // Owner 2026-09-13: "agent卡片，也要紧凑一点，尤其是停止按钮，又大颜色也不好看，状态小点
  // 稍微有点挨得近了，而且上下不居中". Measured 390 coarse before: stop 44×44 in a
  // 46px card (card 111px wide), name→dot gap 1.3–2px, dot on the line-box centre.
  assert.match(source, /class="agent-stop compact-tools" class:pending use:overDot/u, 'the shared dense slot, not a private size');
  assert.doesNotMatch(source, /variant="warn"/u, 'the amber-mixed ink is gone with its variant');
  assert.match(source, /<span class="a-name">\{a\.name\}<span class="ac-top"><span class="st"/u, 'the dot is in the name\'s line');
  assert.match(rule('.ac-top'), /vertical-align: middle; margin-inline-start: 5px;/u);
});

test("a second click on the recipient's card opens its menu at the card, never deselects (board #196)", () => {
  // Owner 2026-09-13: "agent选中卡片时，再次点击不是取消选中，而且展开选项卡".
  assert.match(source, /if \(recipient === name\) \{\n\s*const a = managedAgents\.find\(\(x\) => x\.name === name\);\n\s*oncontext\(cardAnchor\(event\.currentTarget\), name, touchInfo\(\(\) => \(a \? cardInfo\(a\) : offCardInfo\(name\)\)\)\);\n\s*return;\n\s*\}/u);
  assert.match(source, /const cardAnchor = \(trigger\) => \(\{ anchor: anchorOf\(trigger\), align: 'left', trigger, keepTriggerClear: true \}\);/u);
  assert.doesNotMatch(source, /recipient === name \? '' : name/u, 'the click toggle is gone');
  assert.match(source, /use:longpress=\{\{ onlongpress: \(at\) => oncontext\(at, a\.name, touchInfo\(\(\) => cardInfo\(a\)\)\) \}\}/u, 'a hold forwards the element anchor longpress hands it');
});

test('Everyone is the PINNED tab: always at the head, previewing on hover (board #236, supersedes #204)', () => {
  // Owner, 2026-09-22: "不用隐藏，我不展开就看不到吧"; "重新给我设计一个好看的
  // 图案…画成 Agent 类似的 Logo" (people and orbiting dots both failed the
  // glance test — the agents' own bot mark says "the agents"); "按钮有点大" —
  // 16px glyph under the avatars' 20px; "鼠标悬停…所有的 Agent 被选中或者激活".
  assert.match(source, /<span class="all-choice acard" class:sel=\{recipient === ALL_TARGET\} role="presentation"/u);
  assert.match(source, /<CommandButton variant="icon" icon="bots" label=\{t\('hubEveryone'\)\} pressed=\{recipient === ALL_TARGET\} bare/u,
    '#237: a small CROWD of the bot mark — "可以多画几个机器人"');
  assert.match(rule('.all-choice'), /--control-icon-size: calc\(var\(--roster-avatar-size\) \+ var\(--roster-gap\)\)/u,
    'the small crowd needs a little more than one avatar; derive its legible size from the token');
  assert.match(source, /\.all-choice :global\(\.command-icon svg\) \{ width: 100%; height: 100%; \}/u,
    'the svg fills the sized box — the atom hands Icon no size prop');
  // #237: the leading tab gives back the width its square box spent. Tight by
  // DEFAULT (desktop, and any device that reports no pointer); the coarse
  // branch restores the square, because on touch the box IS the target — and
  // it must come after the tight rule, which carries the same specificity.
  assert.match(source, /\.all-choice :global\(\.command-button\.icon-only\) \{ width: auto; min-width: 0; padding-inline: 2px; \}/u);
  const tight = source.indexOf('.all-choice :global(.command-button.icon-only) { width: auto;');
  const square = source.indexOf('.all-choice :global(.command-button.icon-only) { width: var(--control-height)');
  assert.ok(tight > 0 && square > tight, 'the coarse square must be declared after the tight default');
  assert.doesNotMatch(source, /\{#if expanded\}\s*\n\s*<span class="all-choice"/u, 'the expanded-only gate is gone');
  assert.doesNotMatch(source, /data-agent="all"/u, '#180: never a card');
  // Hover/focus previews the choice — the STRIP lights as one enclosure,
  // exactly as clicking would (round 5); touch never previews.
  assert.match(source, /let allPreview = \$state\(false\);/u);
  assert.match(source, /const allLit = \$derived\(recipient === ALL_TARGET \|\| allPreview\);/u);
  assert.match(source, /<div class="tabs" class:all-lit=\{allLit\}>/u);
  assert.match(source, /onpointerenter=\{\(e\) => \{ if \(e\.pointerType !== 'touch'\) allPreview = true; \}\}/u);
  assert.doesNotMatch(source, /class:preview=/u, 'round 5: no per-card preview paint — the strip is the one enclosure');
});

test('a card is a TAB wearing the agent bubble, and multi-select is ONE enclosure (board #236, round 5)', () => {
  // Owner: "做成类似 Chrome tab 栏的样式"; round 5 — "会不会有点过亮了？…把其他
  // 地方变得更暗", "和 Agent 返回给我的消息框的亮度色彩差不多就可以", "整体加一个
  // 稍微淡白色的边", "如果是选择多个 Agent，就用一个大的包边…不要有很多线拐来拐去".
  assert.match(rule('.acard'), /--card-paint: transparent; --card-line: transparent/u, 'at rest a card is a name, not a block');
  assert.match(source, /\.acard:hover \{ --card-paint: var\(--surface2\); \}/u, 'hover is the one quiet wash');
  assert.match(source, /\.acard\.sel \{ --card-paint: var\(--bubble-in\); --card-line: var\(--bubble-line\); \}/u,
    'the lit tab wears the agent bubble: its brightness and its faint edge');
  // The lit tab overlaps the band's top edge by 1px with the SAME fill, so the
  // outline breaks exactly at the junction — no seam, no stub of line.
  assert.match(source, /\.cards:not\(\.expanded\) \.acard\.sel \{ z-index: 1; \}/u);
  assert.match(source, /\.cards:not\(\.expanded\) \.acard\.sel::before \{[\s\S]{0,240}?inset: var\(--control-paint-inset\) 0 -1px; border-bottom: 0;/u);
  assert.match(rule('.cards:not(.expanded) .acard.sel::before'), /border-radius: var\(--ui-radius-panel\) var\(--ui-radius-panel\) 0 0/u,
    'Chrome-smooth top corners use the existing panel radius while the bottom stays open');
  assert.match(rule('.roster.compact .cards:not(.expanded) .acard.sel::before'), /inset-block-start: var\(--roster-gap\)/u,
    'the compact tab uses existing gap geometry instead of an extra coarse top gutter');
  // Multi-select: the STRIP is the lit tab — one fill, one edge, no lines
  // between siblings, and the per-card paint switches off by construction.
  assert.match(rule('.tabs.all-lit'), /background: var\(--bubble-in\)/u);
  assert.match(rule('.tabs.all-lit'), /border: 1px solid var\(--bubble-line\); border-bottom: 0/u);
  assert.match(rule('.tabs.all-lit'), /border-radius: var\(--ui-radius-panel\) var\(--ui-radius-panel\) 0 0/u);
  assert.match(rule('.tabs.all-lit'), /margin-bottom: -1px; padding-bottom: 1px; position: relative; z-index: 1/u);
  assert.match(source, /\.tabs\.all-lit \.acard::before \{ background: transparent; border-color: transparent; \}/u);
  // Round 6: the enclosure belongs to the DESTINATIONS group, sized to its
  // content — the +, a stopped identity and the space behind them are not
  // destinations (owner: "不要把加号后面的这些区域也都框出来").
  assert.match(rule('.tabs'), /display: flex; align-items: center; gap: var\(--roster-gap\); flex: none/u,
    'the group must NOT shrink in the scrolling strip: its flex:none cards would spill out of it and the + would draw on top (review 2026-09-22)');
  assert.match(rule('.cards.expanded .tabs'), /flex: 0 1 auto; min-width: 0; flex-wrap: wrap/u,
    'only the wrapped list shrinks the group — there it wraps inside itself');
  assert.doesNotMatch(source, /\.cards\.all-lit/u, 'the row-wide enclosure is gone');
  assert.match(rule('.acard::before'), /transition: background var\(--t-move\) ease, border-color var\(--t-move\) ease,\n\s*border-radius var\(--t-move\) ease, inset var\(--t-move\) ease/u,
    'switching tabs is movement: the swap crossfades and reshapes, never snaps');
  assert.doesNotMatch(source, /\.roster::after/u, 'the hairline model stays dead');
  assert.match(rule('.roster'), /background: var\(--hub-tab-frame\)/u, 'the contrast comes from the frame going darker');
});

test('team tabs use the reference pill, broken baseline and raised member contour (#238)', () => {
  assert.match(source, /\{@const named = !!group\.team && group\.members\.length > 1\}/u);
  assert.match(source, /class:team=\{named\} data-team=\{named \? group\.team : undefined\}/u);
  assert.match(source, /role=\{named \? 'group' : undefined\} aria-label=\{named \? `\$\{t\('teamsTitle'\)\} \$\{group\.team\}` : undefined\}/u);
  assert.match(rule('.roster-cluster'), /display: flex; align-items: center; gap: var\(--roster-gap\); flex: none/u,
    'one non-shrinking unit prevents its tabs from spilling over neighbours on horizontal scroll');
  assert.match(rule('.cards:not(.expanded) .roster-cluster.team::before'), /position: absolute; inset: auto var\(--roster-gap\) 0/u,
    'the baseline spans the group floor, not the tray or the top edge');
  assert.match(rule('.cards:not(.expanded) .roster-cluster.team::before'), /height: calc\(var\(--roster-gap\) \/ 2\); background: var\(--text2\)/u,
    'one neutral pixel uses existing spacing and adds no row height');
  assert.match(rule('.team-label'), /min-height: var\(--roster-ring-size\)/u, 'the pill fits within the existing tab row');
  assert.match(rule('.team-label'), /border-radius: var\(--ui-radius-row\)/u);
  assert.match(rule('.team-label'), /background: var\(--control-surface\)/u);
  assert.match(rule('.roster.compact .team-label'), /display: none/u, 'long team names never consume a whole tab on the phone');
  assert.match(rule('.roster.compact .cards.expanded .team-label'), /display: inline-flex/u,
    'the expanded list has room for the team pill on both screens');
  assert.match(rule('.cards:not(.expanded) .tabs:not(.all-lit) .roster-cluster.team .acard.sel'), /--card-line: var\(--text2\)/u,
    'only a lit grouped tab has a raised neutral contour; solos and All keep their original paint');
  assert.match(rule('.tabs.all-lit .roster-cluster.team::before'), /display: none/u, 'All owns the only visible enclosure');
  assert.match(rule('.cards.expanded .roster-cluster.team'), /flex: 0 1 100%; min-width: 0; flex-wrap: wrap/u,
    'the expanded group may wrap without overflowing the narrow list');
  assert.doesNotMatch(source, /onclick=\{[^}]*group\.team/u, 'a team label is not a second recipient command');
});
