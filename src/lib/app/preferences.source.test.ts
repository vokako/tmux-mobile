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
  assert.match(source, /\.\.\.\(showAgents \? \[\{ id: 'agents', label: \(\) => t\('agentsTitle'\) \}\] : \[\]\)/u,
    'one row, labelled with the page’s own name');
  // Connection stays LAST — it is the way out (disconnect lives there).
  const list = source.match(/const tabs = \$derived\(\[[\s\S]*?\]\);/u)?.[0] ?? '';
  assert.ok(list.indexOf("id: 'agents'") < list.indexOf("id: 'connection'"), 'Agents sits before Connection');
  // A restored category that does not exist here must not leave a blank pane.
  assert.match(source, /if \(!showAgents && tab === 'agents'\) \{ tab = 'appearance';/u);
  assert.doesNotMatch(source, /if \(!showAgents && tab === 'agents'\) selectTab/u,
    'a correction must not DRILL into a category the user never tapped');
});

test('the category is restorable, and it opens on request', () => {
  assert.match(source, /storedTab === 'agents'/u, 'a reload comes back to the category you were in');
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
    /onGoBack\?\.\(\(\) => \{[\s\S]*?if \(tab === 'agents' && agentsBack\?\.\(\)\) return true;[\s\S]*?if \(catOpen && isCompact\(\)\) \{ closeCat\(\); return true; \}/u,
  );
  assert.match(source, /onGoBack=\{\(fn: \(\) => boolean\) => agentsBack = fn\}/u, 'the embedded page hands its chain up');
});

test('one head at a time: Settings yields its own to the editor', () => {
  // The embedded editor brings a .page-head of its own; two stacked title bars
  // is most of a phone's first screenful.
  assert.match(source, /\{#if !\(tab === 'agents' && agentsDrilled\)\}\s*<div class="page-head">/u);
  assert.match(source, /onDrilled=\{\(d: boolean\) => agentsDrilled = d\}/u);
});
