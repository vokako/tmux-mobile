import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Board.svelte', import.meta.url), 'utf8');

test('the issue detail is a DRAFT: explicit save, clean cancel, guarded exits (board #11)', () => {
  // Only the explicit Save persists the text fields — the inputs bind the
  // draft, never sel, and the save routes through draftPatch so untouched
  // fields stay off the wire.
  assert.match(source, /<input class="d-title-input" bind:value=\{draft\.title\}/u, 'the title edits the draft');
  assert.match(source, /<textarea class="d-body-edit" bind:value=\{draft\.body\}/u, 'the body edits the draft');
  assert.match(source, /const patch = draftPatch\(draft, draftBase\);/u,
    'save diffs against the draft BASE — diffing the live issue ships stale untouched fields (#11 review)');
  assert.match(source, /onclick=\{saveDraft\}/u, 'save is a button, not a side effect');
  assert.match(source, /disabled=\{busy \|\| !draftValid\(draft\)\}/u, 'saving twice / a blank title is unclickable');
  assert.match(source, /onclick=\{cancelDraft\}/u, 'cancel is explicit too');
  assert.match(source, /draft = \{ \.\.\.draftBase \};/u,
    'cancel restores the draft BASE — the server text the user started from, kept fresh by the rebase');
  // Every exit that would drop a dirty draft goes through the confirm
  // dialect: the back button, Escape, the phone's back gesture, a sidebar
  // project switch. Nothing silently discards.
  assert.match(source, /onclick=\{\(\) => guard\(\(\) => \(sel = null\)\)\}/u, 'the back button is guarded');
  assert.match(source, /if \(sel && dirty\) \{ pendingDiscard = \(\) => \{ sel = null; \}; e\.stopPropagation\(\); return; \}/u,
    'Escape asks first while dirty');
  assert.match(source, /if \(sel && dirty\) \{ pendingDiscard = \(\) => \{ sel = null; \}; return true; \}/u,
    'the back gesture asks first while dirty');
  assert.match(source, /function pick\(s: string\) \{\s*\n\s*guard\(/u, 'a sidebar switch asks first');
  assert.match(source, /<ConfirmDialog open=\{!!pendingDiscard\} danger=\{false\}/u,
    'the shared confirm dialect, neutral tone — discarding an edit is not deleting a file');
  // A refetch REBASES three-way: untouched fields follow the server, touched
  // fields keep the user's text (#11 review) — never a blind draft reset.
  const refetch = source.slice(source.indexOf('async function refetchSel'), source.indexOf('// Assigning DOES'));
  assert.match(refetch, /const r = rebaseDraft\(draft, draftBase, draftOf\(sel\)\);/u, 'refetchSel rebases');
  assert.ok(!refetch.includes('draft = draftOf(sel)'), 'and never blindly resets the draft');
});

test('assignment is ONE dispatch — the detail picker and the create dialog share it (board #11)', () => {
  // dispatchAssign is the single carrier of assignment=dispatch semantics:
  // saving the assignee AND typing the brief into the agent's pane. Exactly
  // one hubPost call site proves nobody re-implements the delivery half.
  assert.match(source, /async function dispatchAssign\(id: number, name: string\)/u, 'the one dispatch function');
  assert.equal(source.split('hubPost(').length - 1, 1, 'exactly one delivery call site, inside dispatchAssign');
  assert.match(source, /await dispatchAssign\(id, name\);/u, 'the detail picker routes through it');
  assert.match(source, /if \(wantAssign && created != null\) await dispatchAssign\(created, wantAssign\);/u,
    'create-with-assignee dispatches too — never just a label');
  // The form closes the MOMENT the create succeeds — before the dispatch can
  // fail — because a retryable form after a successful create mints
  // duplicate issues (#11 review). Order in the source is the guarantee.
  const create = source.slice(source.indexOf('async function createIssue'), source.indexOf('async function addNote'));
  const closes = create.indexOf('creating = false;');
  const dispatches = create.indexOf('await dispatchAssign(created');
  assert.ok(closes > -1 && dispatches > -1 && closes < dispatches,
    'the form is gone before the dispatch is attempted');
  assert.ok(create.indexOf('busy = false;\n      return;') < closes,
    'only a FAILED create keeps the form for an honest retry');
});

test('the layout hierarchy: one compact title line, the body visibly bigger (board #11)', () => {
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.d-title-input \{[^}]*padding: 7px 10px;/u, 'the title stays a single compact line');
  assert.ok(!/\.d-title-input \{[^}]*min-height/u.test(style), 'no tall title');
  const body = /\.d-body-edit \{[^}]*min-height: (\d+)px;[^}]*resize: vertical;/u.exec(style);
  assert.ok(body, 'the body field declares a min-height and grows');
  assert.ok(Number(body![1]) >= 160, `the body is the BIG field (${body![1]}px)`);
});

test('the sidebar orders by conversation, and rows are summaries (reopened #11)', () => {
  // Same recipe as the Hub: hub_rooms feeds sortRows, newest talk first.
  assert.match(source, /projects = sortRows\(\(r\.projects \?\? \[\]\)\.filter\([\s\S]*?archived\), talkMap\);/u,
    'sortRows over the talk map, archived rows dropped');
  assert.match(source, /hubRooms\(\)\.catch/u, 'the talk map comes from hub_rooms, fail-soft');
  const row = source.slice(source.indexOf('class="side-row"'), source.indexOf('</aside>'));
  assert.match(row, /class="side-age"/u, 'rows carry the last-reply age');
  assert.ok(!row.includes('side-win'), 'no agent chips — the Board row is a SUMMARY');
});

test('notes are a timeline: author + time header, content box below (reopened #11)', () => {
  assert.match(source, /<div class="n-head">\s*<span class="n-author">\{n\.author\}<\/span>\s*<span class="n-at">/u,
    'author left, time right, one header line');
  assert.match(source, /<\/div>\s*<div class="n-body">\{n\.body\}<\/div>/u, 'the content is its own box below');
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.n-at \{[^}]*margin-left: auto/u, 'the time right-aligns');
  assert.match(style, /\.n-author \{ color: var\(--accent\)/u, 'the author wears the accent ink');
  assert.match(source, /\{t\('boardOpenedBy'\)\} <span class="m-name">\{sel\.created_by\}<\/span>/u,
    'the opened-by name is highlighted too');
});

test('columns scroll alone; the page holds still (reopened #11)', () => {
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.board \{[^}]*overflow: hidden/u, 'the page is not the scroller');
  assert.match(style, /\.cols \{[^}]*flex: 1;\s*\n\s*min-height: 0;/u, 'the column grid takes the available height');
  assert.match(style, /\.col-scroll \{\s*\n\s*overflow-y: auto;/u, 'each column\u2019s cards area is its own scroller');
  assert.match(source, /<div class="col-scroll subtle-scroll">/u, 'the scroller wraps the cards, not the header');
  assert.match(style, /\.detail \{[^}]*overflow-y: auto/u, 'the detail view brings its own scroller');
});

test('create lives in the head, and compact gets the hamburger drawer (reopened #11)', () => {
  const head = source.slice(source.indexOf('<div class="head">'), source.indexOf('</div>', source.indexOf('boardNew')));
  assert.match(head, /class="icon-btn go" title=\{t\('boardNew'\)\}/u, 'new-issue is the head\u2019s top-right action');
  assert.ok(!source.includes('class="new-issue"'), 'the bottom button is gone');
  // The drawer speaks the Chat/Terminal dialect: hamburger toggle, sheet,
  // scrim, and the back gesture closes the drawer FIRST.
  assert.match(source, /class="icon-btn side-toggle"[^>]*\n?[^>]*onclick=\{\(\) => \(sideOpen = !sideOpen\)\}/u,
    'the hamburger toggles the drawer');
  assert.match(source, /<aside class="sidebar" class:open=\{sideOpen\}>/u, 'the sidebar is the sheet');
  assert.match(source, /class="side-scrim" onclick=\{\(\) => \(sideOpen = false\)\}/u, 'the scrim dismisses');
  assert.match(source, /if \(sideOpen\) \{ sideOpen = false; return true; \}/u, 'back closes the drawer first');
  assert.match(source, /sideOpen = false; \/\/ choosing closes the drawer/u, 'picking a project closes it');
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /transform: translateX\(-100%\); transition: transform var\(--t-move\)/u,
    'the sheet parks off-canvas and slides — the shared motion grammar');
  assert.ok(!style.includes('.board-root.picked'), 'the second-page drilldown is retired');
});

test('a feed jump opens its issue in its OWN session (board #13 follow-up)', () => {
  // The request names the session because a manual pick may have parked the
  // page elsewhere and issue ids are session-gated; the dirty-draft guard
  // still stands between the jump and an open edit.
  assert.match(source, /issueRequest = null/u, 'the request arrives as a prop');
  assert.match(source, /if \(req\.session && req\.session !== cur\) \{ cur = req\.session; picked = true; \}/u,
    'the jump re-aims the page at the issue\u2019s session');
  assert.match(source, /guard\(\(\) => \{\s*\n\s*if \(req\.session/u, 'the dirty guard wraps the jump');
  assert.match(source, /void openIssue\(req\.id\);/u, 'and the issue detail opens');
  assert.match(source, /req\.n === issueReqSeen/u, 'each request fires once — the n makes the newest win');
});
