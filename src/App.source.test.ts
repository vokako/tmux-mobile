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
  const aside = source.match(/<aside class="term-side"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /<SideHandle \/>/u, 'the sidebar carries the shared handle');
  assert.doesNotMatch(source, /grid-template-columns: 280px/u);
});

test('the session list lives inside the Terminal page, sheeted on a phone', () => {
  // One Sessions instance, mounted as the terminal page's sidebar; on a phone
  // it slides over and a pick closes it.
  const mounts = source.match(/<Sessions\b/gu) ?? [];
  assert.equal(mounts.length, 1, 'exactly one Sessions mount');
  const aside = source.match(/<aside class="term-side"[\s\S]*?<\/aside>/u)?.[0] ?? '';
  assert.match(aside, /class:sheet=\{layout\.isTouchDevice\}/u);
  assert.match(aside, /class:open=\{sessListOpen\}/u);
  assert.match(aside, /onPick=\{\(\) => sessListOpen = false\}/u);
  // The terminal's session chip opens that same sheet on a phone.
  assert.match(source, /onOpenSessions=\{layout\.isTouchDevice \? \(\) => sessListOpen = true : null\}/u);
  // A phone back gesture closes the sheet instead of leaving the app.
  assert.match(source, /page === 'terminal' && sessListOpen.*sessListOpen = false/u);
});
