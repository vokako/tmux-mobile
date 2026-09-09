import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Roster.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('roster and fixed menu share one lifetime with live parent authority (#132)', () => {
  assert.match(source, /menuFor = \$bindable\(''\), cardsEl = \$bindable\(null\)/u);
  assert.match(source, /onselect: setRecipient/u);
  assert.match(source, /onfilter: toggleFilter/u);
  assert.match(source, /onwatch: openDrawer/u);
  assert.match(source, /onaction: askAction/u);
  assert.match(source, /const rosterGroups = \$derived\(groupRoster\(managedAgents\)\)/u);
  const selected = source.indexOf('{#if selected}');
  const menu = source.indexOf('{#if menuFor}');
  assert.ok(selected >= 0 && menu > selected);
  assert.match(source.slice(selected, menu), /<\/div>\s*<\/div>\s*\{\/if\}\s*$/u,
    'the selected gate ends before the fixed menu');
  assert.doesNotMatch(source, /addEventListener|popstate|history\./u,
    'the existing Hub dismissal listeners stay in their original order');
  assert.doesNotMatch(source, /^\s*\.st \{/mu, 'the shared Hub dot box is not copied');
  assert.match(source, /:global\(\.hub-root\.compact\) \.a-menu button \{ min-height: 44px; \}/u);
});

test('the card keeps select, deferred options, and double-click filter semantics', () => {
  const live = source.slice(source.indexOf('class="acard" class:sel'), source.indexOf('class="acard off"'));
  assert.match(live, /onclick=\{\(e\) => cardClick\(a\.name, e\.currentTarget\)\}/u);
  assert.match(live, /ondblclick=\{\(\) => cardDbl\(a\.name\)\}/u);
  assert.match(live, /onkeydown=[^\n]*cardClick\(a\.name, e\.currentTarget\)/u);
  const machine = source.slice(source.indexOf('function cardClick'), source.indexOf('let menuAnchor'));
  assert.match(machine, /if \(recipient !== name\) \{ setRecipient\(name\); return; \}/u);
  assert.match(machine, /if \(cardTimer\) \{\s*clearTimeout\(cardTimer\); cardTimer = null;\s*if \(cardTimerFor === name\) return;/u);
  assert.match(machine, /cardTimerFor = name;\s*cardTimer = setTimeout\([\s\S]*?toggleAgentMenu\(name, el\); \}, 260\)/u);
  assert.match(machine, /menuFor = '';\s*setRecipient\(name\);\s*toggleFilter\(name\)/u,
    'the parent toggles the filter after the original clear/address sequence');
  assert.doesNotMatch(live, /class="a-more"|name="dots"/u);
  const menu = source.slice(source.indexOf('{#if menuFor}'), source.indexOf('<style>'));
  assert.match(menu, /onclick=\{\(\) => toggleFilter\(menuFor\)\}/u);
  assert.match(menu, /restartAgent\(n\)[\s\S]*?t\('hubRestart'\)/u);
  assert.equal([...source.matchAll(/oncontext\(/g)].length, 4, 'live/stopped right-click and long-press share parent verbs');
});

test('hover facts and ready/reveal gates follow the roster markup', () => {
  const live = source.slice(source.indexOf('class="acard" class:sel'), source.indexOf('class="acard off"'));
  assert.match(live, /use:hoverInfo=\{\(\) => cardInfo\(a\)\}/u);
  assert.match(live, /aria-label=\{\[`\$\{a\.name\} · \$\{stateLabel\(a\.state\)\}`, a\.detail, vitalsLine\(a\.vitals\)\]/u);
  assert.doesNotMatch(live, /<div class="acard" class:sel[^>]*\stitle=/u);
  assert.match(source, /use:hoverInfo=\{\(\) => offCardInfo\(name\)\}/u);
  const cardFns = source.slice(source.indexOf('function cardInfo'), source.indexOf('</script>'));
  assert.doesNotMatch(cardFns, /\bnote\s*:/u);
  for (const fmt of ['modelLabel(a.vitals.model)', 'fmtElapsed(a.since, tick)', 'stateTone(a.state)']) assert.ok(cardFns.includes(fmt));
  assert.match(source, /\{#if !roomReady\}\s*<div class="skel-wrap sk-cards" aria-hidden="true">/u);
  assert.match(source, /class="cards" class:chips=\{compact\} class:reveal=\{justLoaded\}/u);
  assert.match(source, /class:appear-pop=\{!!rosterBase && !rosterBase\.has\(a\.name\)\}/u);
});

test('the add-agent entry is present in empty and closed selected projects', () => {
  const gate = source.match(/\{#if ([^}]*)\}\s*(?:<!--[\s\S]*?-->\s*)*<div class="roster">/u)?.[1];
  assert.equal(gate, 'selected');
  assert.match(source, /(?:<!--[\s\S]*?-->\s*)<button class="acard add"/u);
  assert.match(source, /onclick=\{onadd\}/u);
  assert.doesNotMatch(source, /\{#if liveSelected\}/u);
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


test('a stopped agent keeps its backend face, greyed — not an anonymous letter (owner, 2026-09-05)', () => {
  // "关闭的 Agent 卡片是灰色的，但它的头像…只是一个字母头像。这个头像应该
  // 使用我们正常设定的 Agent 头像，并且变成灰色" — the slot declares its
  // backend (`command`), so the stopped card can wear the SAME icon the live
  // card wears; grey comes from a filter, never a second icon set. The
  // letter tile remains only as the fallback a backend without an icon
  // already has.
  assert.match(source, /const slotBackend = \(name\) => \(selectedRow\?\.slots \?\? \[\]\)\.find\(\(s\) => s\.window_name === name\)\?\.command;/u,
    'the declared slot is the identity source');
  assert.match(source, /\{#each stopped as name \(name\)\}\s*\{@const backend = slotBackend\(name\)\}/u,
    'each stopped card resolves its declared backend');
  assert.match(source,
    /\{#if backendIcon\(backend\)\}<img class="ava dim" src=\{backendIcon\(backend\)\} alt=\{backend\} \/>\{:else\}<span class="ava dim">\{name\.slice\(0, 1\)\.toUpperCase\(\)\}<\/span>\{\/if\}/u,
    'the stopped card resolves the icon exactly like the live card, with the letter tile as fallback');
  assert.match(source, /img\.ava\.dim \{ background: none !important; filter: grayscale\(1\); opacity: 0\.55; \}/u,
    'the icon greys by filter — identity stays, colour goes');
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
