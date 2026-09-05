// Source-contract test for Settings hosting the agent configuration (board #10,
// owner 2026-08-29: "手机上的 Agent 设置页面应该归到 settings 里边的一个子页面,
// 不用单独在底下一行展示了，现在看着有点多底下的标签").
//
// The rule that must not rot: on a phone this category shows the REAL
// AgentsPage. A second implementation of the same editors is how the two
// devices start disagreeing about what an agent definition even has — and
// nothing would fail while they drifted.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Preferences.svelte', import.meta.url), 'utf8');
const style = source.match(/<style>[\s\S]*<\/style>/u)?.[0] ?? '';

test('the Agents category embeds the real AgentsPage, never a copy of it', () => {
  assert.match(source, /import AgentsPage from '\.\.\/hub\/AgentsPage\.svelte'/u);
  assert.match(source, /<AgentsPage\b/u, 'the page itself is mounted here');
  // A copy would have to talk to the registry directly. Settings must not.
  assert.doesNotMatch(source, /registrySave|registryDelete|registryList/u,
    'agent definitions are edited in AgentsPage and only there');
  assert.match(style, /\.agents-embed \{[^}]*flex: 1[^}]*min-height: 0/u,
    'it is a page (height:100%), so it takes the shell’s remaining height rather than sitting in the padded pane');
});

test('the category exists only where Agents is not a page of its own', () => {
  // showAgents is App's call (nav-state's agentsLivesInSettings && hubEligible):
  // the desktop rail keeps Agents as a page with its own draggable icon.
  assert.match(source, /showAgents = false/u, 'off by default, so a host that says nothing gets the old Settings');
  // Four rows on the phone (owner, 2026-09-02: "把 team agent mcp skill 分开几个
  // 二级设置页面吧"), each the REAL page narrowed by `section` — since
  // 2026-09-05 as their own labelled AGENT group.
  assert.match(source, /\.\.\.\(showAgents \? \[\{\s*id: 'agent', label: \(\) => t\('settingsGroupAgent'\), rows: \[\s*\{ id: 'agents', label: \(\) => t\('agentsTitle'\) \},\s*\{ id: 'teams', label: \(\) => t\('teamsTitle'\) \},\s*\{ id: 'skills', label: \(\) => t\('skillsTitle'\) \},\s*\{ id: 'mcp', label: \(\) => t\('mcpTitle'\) \},\s*\],\s*\}\] : \[\]\)/u,
    'four rows, each labelled with its section’s own name, in one AGENT group');
  assert.match(source, /<AgentsPage\s+section=\{tab\}/u, 'the one instance is narrowed by the category, never copied');
  // The two-group order (owner, 2026-09-05: "分成两组：1. 关于本身应用层面的
  // 一些设置 2. 关于 Agent 层面的设置"): the APP group — Appearance,
  // Notifications, Terminal, (Shortcuts,) Connection — precedes the AGENT
  // group. Connection ends the APP group as its way out; it no longer trails
  // the agent rows (the pre-group "Connection stays last" rule, superseded).
  const list = source.match(/const groups = \$derived\(\[[\s\S]*?\]\);/u)?.[0] ?? '';
  const order = ['appearance', 'notifications', 'terminal', 'shortcuts', 'connection', 'agents', 'teams', 'skills', 'mcp']
    .map((id) => list.indexOf(`id: '${id}'`));
  assert.ok(order.every((i) => i >= 0), 'every category sits in the grouped list');
  assert.deepEqual([...order].sort((a, b) => a - b), order, 'app group first, Connection last inside it, then the agent group');
  assert.match(source, /const tabs = \$derived\(groups\.flatMap\(\(g\) => g\.rows\)\);/u,
    'the flat list every consumer reads is derived FROM the groups — one source of order');
  // A restored category that does not exist here must not leave a blank pane.
  assert.match(source, /if \(!showAgents && AGENT_TABS\.includes\(tab\)\) \{ tab = 'appearance';/u);
  assert.doesNotMatch(source, /if \(!showAgents && AGENT_TABS\.includes\(tab\)\) selectTab/u,
    'a correction must not DRILL into a category the user never tapped');
});

test('groups label themselves only when there are two, in the sidebar\u2019s own header voice (owner, 2026-09-05)', () => {
  // "分成两组" — the labels exist FOR the contrast: a desktop list with no
  // agent group is one group, and one labelled group is noise. The label is
  // the shared .side-h dialect (group-label like Sessions/Projects), never a
  // new species; no scoped rule may re-declare its box or type
  // (ui/sidebar.source.test.ts owns that drift).
  assert.match(source, /\{#each groups as g \(g\.id\)\}\s*\{#if groups\.length > 1\}<div class="group-label side-h">\{g\.label\(\)\}<\/div>\{\/if\}/u,
    'a label per group, only when a second group exists');
  assert.match(source, /\{#each g\.rows as item \(item\.id\)\}/u, 'the rows render inside their group');
  assert.doesNotMatch(style, /\.group-label/u, 'no scoped .group-label rule — .side-h owns the box and type');
});

test('the category is restorable, and it opens on request', () => {
  assert.match(source, /AGENT_TABS\.includes\(storedTab\)/u, 'a reload comes back to the category you were in');
  assert.match(source, /let openedRequest = 0;/u);
  assert.match(
    source,
    /const req = openRequest;\s*if \(!req \|\| req\.n === openedRequest\) return;[\s\S]*?if \(tabs\.some\(\(x\) => x\.id === req\.tab\)\) selectTab\(req\.tab\)/u,
    'a one-shot {tab, n} request, ignored when that category does not exist here',
  );
});

test('back peels the embedded page first, then the category', () => {
  // Same order the page uses on its own: dialog, editor, then out. Getting this
  // backwards drops the user out of Settings mid-edit.
  assert.match(
    source,
    /onGoBack\?\.\(\(\) => \{[\s\S]*?if \(AGENT_TABS\.includes\(tab\) && agentsBack\?\.\(\)\) return true;[\s\S]*?if \(catOpen && isCompact\(\)\) \{ closeCat\(\); return true; \}/u,
  );
  assert.match(source, /onGoBack=\{\(fn: \(\) => boolean\) => agentsBack = fn\}/u, 'the embedded page hands its chain up');
});

test('one head at a time: Settings yields its own to the editor', () => {
  // The embedded editor brings a .page-head of its own; two stacked title bars
  // is most of a phone's first screenful.
  assert.match(source, /\{#if !\(AGENT_TABS\.includes\(tab\) && agentsDrilled\)\}\s*<div class="page-head">/u);
  assert.match(source, /onDrilled=\{\(d: boolean\) => agentsDrilled = d\}/u);
});

test('a tapped address wears the running cue until the socket settles (2026-09-03)', () => {
  // The dot is the app-wide class from app.css, never a local re-implementation.
  assert.match(source, /<span class="addr-dot" class:live-dot=\{pending\}><\/span>/u);
  assert.match(source, /\{@const pending = address === pendingAddress\}/u);
  const dotRules = style.match(/[^{}]*addr-dot[^{]*\{[^}]*\}/gu)?.join('\n') ?? '';
  assert.ok(dotRules, 'the dot has rules');
  assert.doesNotMatch(dotRules, /animation|box-shadow|@keyframes/u, 'no local re-implementation — .live-dot is the one running cue');
  // At rest achromatic; the current and the dialing row accent.
  assert.match(style, /\.addr-dot\{[^}]*background:var\(--status-sleep\)/u);
  assert.match(style, /\.address-list button\.active \.addr-dot,\.address-list button\.pending \.addr-dot\{background:var\(--accent\)\}/u);
});

test('category and address rows explain themselves with the one hover card (motion.md §1.16, board #86)', () => {
  // A category row's label is terse; the card says what is inside (one i18n
  // hint per category, kept OUTSIDE `tabs` so the pinned list shape holds).
  assert.match(source, /class="side-row" class:open=\{tab === item\.id\} onclick=\{\(\) => selectTab\(item\.id\)\}\s*use:hoverInfo=\{\(\) => \(\{ title: item\.label\(\), text: TAB_HINTS\[item\.id\]/u);
  for (const id of ['appearance', 'notifications', 'terminal', 'shortcuts', 'agents', 'teams', 'skills', 'mcp', 'connection']) {
    assert.match(source, new RegExp(`${id}: 'settings[A-Za-z]+Hint'`, 'u'), `${id} has a hint`);
  }
  // An address row: the address and its state (current / dialing / alternate);
  // the dialing cue used to be a native title — the card replaces it.
  const addr = source.match(/<button class:active=\{address === activeAddress\} class:pending[\s\S]*?onclick=/u)?.[0] ?? '';
  assert.match(addr, /use:hoverInfo=\{\(\) => \(\{ title: address, lines: \[pending/u);
  assert.doesNotMatch(addr, /title=/u);
});

test('the category list and a switched category unfold instead of flashing (motion.md §1.15, board #86)', () => {
  // First paint only for the list: on compact the drill display-toggles the
  // sidebar, and a re-shown list must not replay the unfold over the
  // drill-back slide (one motion per view).
  assert.match(source, /<div class="side-scroll subtle-scroll" class:reveal=\{!drillAnim\} use:scrollFade>/u);
  // The pane is keyed on the category so a switch remounts it and its cards rise in.
  assert.match(source, /\{#key tab\}\s*<div class="pref-content reveal">/u);
  assert.match(source, /<\/div>\s*\{\/key\}\s*\{\/if\}\s*<\/div>\s*<\/section>/u, 'the key closes with the pane');
});

test('the phone reaches the server registry from the top of Settings (2026-09-03)', () => {
  // The row exists only when App hands over the opener (touch layout); it is a
  // .side-row like the categories, not a new species, and it opens the SAME
  // registry popover the desktop rail opens.
  assert.match(source, /\{#if onServers\}\s*<button class="side-row server-row"[^>]*aria-haspopup="menu"/u);
  assert.match(source, /onclick=\{\(e\) => onServers\?\.\(e\)\}/u);
  assert.match(source, /<span class="quarter-turn" class:on=\{serversOpen\}><Icon name="swap-h" size=\{14\} \/><\/span>/u,
    'the symmetric swap glyph turns 90°, not an invisible 180°');
  assert.match(source, /<span class="r-label">\{serverName\}<\/span>/u, 'the authenticated/fallback HOSTNAME, never the raw address');
  assert.match(source, /onServers = null,/u, 'off by default — the desktop rail has its own control');
});


test('settings controls use labels and values, not a subtitle under every row (board #87)', () => {
  for (const key of [
    'themeHint', 'languageHint', 'layoutHint', 'hubFeedLevelHint', 'hubStepsRowsHint',
    'uiFontBodyHint', 'uiFontDisplayHint', 'uiZoomHint', 'hubNotifyHint',
    'hubNotifyLevelHint', 'hubNotifyTestHint', 'fontFamilyHint', 'fontSizeHint',
    'lineHeightHint', 'shortcutGlobalScope', 'shortcutTerminalScope', 'debugHint',
    'agentNotificationsHint',
  ]) {
    assert.doesNotMatch(source, new RegExp(`<small>\\{t\\('${key}'\\)\\}<\\/small>`, 'u'),
      `${key} must not become persistent tutorial copy`);
  }
  assert.match(source, /notifyPerm === 'denied'[\s\S]{0,120}<small>\{t\('hubNotifyDenied'\)\}<\/small>/u,
    'an actual permission problem still explains itself');
  assert.match(source, /class="font-error appear"/u, 'validation errors remain visible');
});