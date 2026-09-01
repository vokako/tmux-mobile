import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Board.svelte', import.meta.url), 'utf8');
const appCss = await readFile(new URL('../../app.css', import.meta.url), 'utf8');

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
  // Cancel ASKS since board #15 ("当前状态没有保存，是否退出"): it routes
  // through the same guard every exit uses, and the dialog's confirm is the
  // one place that restores the base.
  assert.match(source, /onclick=\{\(\) => guard\(\(\) => \{\}\)\}/u, 'cancel asks before discarding');
  assert.match(source, /draft = \{ \.\.\.draftBase \};/u,
    'confirming restores the draft BASE — the server text the user started from, kept fresh by the rebase');
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
  assert.match(source, /async function dispatchAssign\(id: number, name: string, title = '', body = '', notes: \{ author: string; body: string; at: number \}\[\] = \[\]\)/u, 'the one dispatch function');
  assert.equal(source.split('hubPost(').length - 1, 1, 'exactly one delivery call site, inside dispatchAssign');
  // The brief carries the NOTE THREAD too (board #42): the delivery appends
  // assignNotes — chronological, authored, budget-capped in board.ts — so an
  // agent cannot miss the discussion under the issue.
  assert.match(source, /await hubPost\(cur, `@\$\{name\} \$\{msg\}\$\{assignNotes\(id, notes\)\}`\);/u,
    'the dispatch message ends with the assignNotes block');
  assert.match(source, /if \(assignee !== undefined\) await dispatchAssign\(sel\.id, assignee, draft\.title, draft\.body, Array\.isArray\(sel\.notes\) \? sel\.notes : \[\]\);/u,
    'a ✓-confirmed assignee change routes through it (board #15) and carries the OPEN issue\u2019s thread (board #42)');
  assert.match(source, /if \(wantAssign && created != null\) await dispatchAssign\(created, wantAssign, wantTitle, wantBody\);/u,
    'create-with-assignee dispatches too — a fresh issue HAS no notes, so none ride (board #42)');
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
  const body = /\.d-body-edit \{[^}]*min-height: (\d+)px;[\s\S]{0,400}?resize: vertical;/u.exec(style);
  assert.ok(body, 'the body field declares a min-height and grows');
  // The 200px fixed size was retired for autoGrow (owner, 2026-08-29: "有的框
  // 很大 有空白 应该自适应"): the min-height is a FLOOR under an adaptive box,
  // so it stays small — content, not the constant, makes the body big.
  assert.ok(Number(body![1]) >= 48 && Number(body![1]) <= 120, `the floor is small (${body![1]}px) — autoGrow does the sizing`);
});

test('the sidebar orders by conversation, and rows are summaries (reopened #11)', () => {
  // Same recipe as the Hub: hub_rooms feeds sortRows, newest talk first —
  // into the FULL list; the shown rows are the derived filter (board #39).
  assert.match(source, /allProjects = sortRows\(\(r\.projects \?\? \[\]\)\.filter\([\s\S]*?archived\), talkMap\);/u,
    'sortRows over the talk map, archived rows dropped');
  assert.match(source, /hubRooms\(\)\.catch/u, 'the talk map comes from hub_rooms, fail-soft');
  const row = source.slice(source.indexOf('class="side-row proj-row"'), source.indexOf('</aside>'));
  assert.match(row, /class="side-age"/u, 'rows carry the last-reply age');
  // The second line is COLUMN COUNTS in the shared chip dress — never the
  // Chat row's agent chips (board #39 kept the summary rule: no roster here).
  assert.ok(!row.includes('a.icon'), 'no agent chips — the Board row summarises the BOARD, not the roster');
  assert.match(row, /countColor\(st\)/u, 'the chips speak the board status language through the sidebar remap');
});

test('notes are a timeline: author + time header, content box below (reopened #11)', () => {
  // The time became a real <button> with board #46 (the accessible action
  // trigger) but keeps its place and clothes: right of the author, one line.
  assert.match(source, /<div class="n-head">\s*<span class="n-author">\{n\.author\}<\/span>[\s\S]*?<button class="n-at"/u,
    'author left, time right, one header line');
  assert.match(source, /<div class="n-text" onclick=\{\(\) => toggleNoteActs\(i\)\}>\{n\.body\.trim\(\)\}<\/div>/u,
    'the content is its own box below, trimmed');
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
  const head = source.slice(source.indexOf('<div class="page-head">'), source.indexOf('</div>', source.indexOf('boardNew')));
  assert.match(head, /class="icon-btn go" title=\{t\('boardNew'\)\}/u, 'new-issue is the head\u2019s top-right action');
  assert.ok(!source.includes('class="new-issue"'), 'the bottom button is gone');
  // The drawer speaks the Chat/Terminal dialect: hamburger toggle, sheet,
  // scrim, and the back gesture closes the drawer FIRST.
  assert.match(source, /class="icon-btn side-toggle"[^>]*\n?[^>]*onclick=\{\(\) => \(sideOpen = !sideOpen\)\}/u,
    'the hamburger toggles the drawer');
  assert.match(source, /<aside class="sidebar" class:side-sheet=\{narrowVp\} class:open=\{narrowVp && sideOpen\}>/u,
    'the sidebar wears the SHARED sheet dialect, CLASS-driven by the page\u2019s own narrow condition');
  assert.match(source, /class="side-scrim" onclick=\{\(\) => \(sideOpen = false\)\}/u, 'the scrim dismisses');
  assert.match(source, /if \(sideOpen\) \{ sideOpen = false; return true; \}/u, 'back closes the drawer first');
  assert.match(source, /sideOpen = false; \/\/ choosing closes the drawer/u, 'picking a project closes it');
  const style = source.slice(source.indexOf('<style>'));
  assert.match(appCss, /\.side-sheet\.side-sheet\.side-sheet \{[\s\S]{0,400}?transform: translateX\(-100%\);[\s\S]{0,200}?transition: transform var\(--t-move\)/u,
    'the sheet parks off-canvas and slides — the shared motion grammar, in app.css');
  assert.ok(!/@media[^{]*\{[^{}]*\n\s*\.side-sheet \{/u.test(appCss),
    'the sheet is CLASS-driven, never media-gated — a media gate disagreed with the Hub\u2019s wider compact (owner, 2026-08-30)');
  assert.ok(!style.includes('translateX'), 'the component does not re-declare the shared geometry');
  assert.ok(!style.includes('.board-root.picked'), 'the second-page drilldown is retired');
});

test('a parked drawer casts no shadow back onto the page (board #14, now the SHARED sheet)', () => {
  // The guarantee moved into app.css with the one drawer dialect (owner,
  // 2026-08-30): parked = no cast, depth only while open.
  const parked = /\.side-sheet\.side-sheet\.side-sheet \{[\s\S]*?\n\}/u.exec(appCss)?.[0] ?? '';
  assert.match(parked, /transform: translateX\(-100%\);/u, 'the closed drawer is parked off-canvas');
  const shadows = [...parked.matchAll(/box-shadow:\s*([^;]+);/gu)].map((m) => m[1]?.trim());
  assert.deepEqual(shadows, ['none'], 'the closed rule has exactly one shadow declaration, and it is invisible');
  assert.match(appCss, /\.side-sheet\.side-sheet\.side-sheet\.open \{ transform: none; box-shadow: 10px 0 30px rgba\(0, 0, 0, 0\.22\); \}/u,
    'depth appears only while open');
  // The TRIPLED selector is load-bearing: each page's scoped base rule
  // (.sidebar.svelte-hash — 0,2,0) beat a single-class shared rule, the
  // sheet stayed in flow and took the top half of the screen (owner,
  // 2026-08-30: "上半部分是抽屉的内容，上下给分开了").
  assert.match(parked, /position: fixed/u, 'the sheet leaves the flow');
  assert.ok(!/\n\.side-sheet \{/u.test(appCss), 'no weaker single-class copy that a scoped rule could beat');
  // And no component keeps a private copy that could drift.
  const style = source.slice(source.indexOf('<style>'));
  assert.ok(!style.includes('box-shadow: 10px'), 'the Board carries no private sheet cast');
})

test('the sheet keeps its compositor layer WITHOUT becoming a fixed containing block (board #21)', async () => {
  // The blink: the open state is `transform: none`, so without a standing
  // compositing hint the WebView drops the sheet's layer at transitionend and
  // re-rasterizes it into the parent — a blank frame at the exact moment the
  // drawer finishes opening. Android Chrome hides that seam; the compiled
  // APK's System WebView showed it (owner, 2026-08-30).
  const parked = /\.side-sheet\.side-sheet\.side-sheet \{[\s\S]*?\n\}/u.exec(appCss)?.[0] ?? '';
  assert.match(parked, /will-change: opacity;/u,
    'the sheet is promoted for its whole mounted life, not just while the transform transitions');
  // The hint must NOT come from the containing-block family: a standing
  // transform/perspective/filter hint re-anchors position:fixed DESCENDANTS to
  // the 300px sheet (the .page lesson, and the design-language rule) — and the
  // Terminal sheet's tree really has them, verified structurally below.
  assert.ok(!/will-change:[^;]*(transform|perspective|filter)/u.test(parked),
    'no standing containing-block hint on the sheet');
  // The evidence, kept live so the constraint cannot silently expire: the
  // term-side sheet mounts Sessions, and Sessions renders dialogs whose
  // backdrop/dialog are position:fixed — they must keep the VIEWPORT.
  const app = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');
  assert.match(app, /<aside class="term-side" class:side-sheet=\{[^}]+\}[\s\S]{0,200}?<Sessions /u,
    'the Terminal sheet mounts Sessions inside the aside');
  const sessions = await readFile(new URL('../sessions/Sessions.svelte', import.meta.url), 'utf8');
  assert.match(sessions, /<CreateProjectDialog /u, 'Sessions renders the create dialog in its tree');
  assert.match(sessions, /<ConfirmDialog /u, 'Sessions renders the confirm dialog in its tree');
  for (const rel of ['../projects/CreateProjectDialog.svelte', '../ui/ConfirmDialog.svelte']) {
    const dlg = await readFile(new URL(rel, import.meta.url), 'utf8');
    assert.match(dlg, /position: fixed/u, `${rel} is a fixed overlay — it must anchor to the viewport`);
  }
})

test('a feed jump opens its issue in its OWN session (board #13 follow-up)', () => {
  // The request names the session because a manual pick may have parked the
  // page elsewhere and issue ids are session-gated; the dirty-draft guard
  // still stands between the jump and an open edit.
  assert.match(source, /issueRequest = null/u, 'the request arrives as a prop');
  assert.match(source, /if \(req\.session && req\.session !== cur\) \{ cur = req\.session; picked = !embedded; \}/u,
    'the jump re-aims the page at the issue\u2019s session — embedded keeps following the room');
  assert.match(source, /guard\(\(\) => \{\s*\n\s*if \(req\.session/u, 'the dirty guard wraps the jump');
  assert.match(source, /void openIssue\(req\.id\);/u, 'and the issue detail opens');
  assert.match(source, /req\.n === issueReqSeen/u, 'each request fires once — the n makes the newest win');
});

test('the board embeds in the Hub drawer without its sidebar (board #13 follow-up)', () => {
  assert.match(source, /embedded = false/u, 'embedded is a prop');
  assert.match(source, /<div class="board-root" class:embedded>/u, 'the root wears it');
  assert.match(source, /\{#if !embedded\}\s*\n\s*<aside class="sidebar"/u, 'no project sidebar in the drawer');
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.board-root\.embedded \{ grid-template-columns: minmax\(0, 1fr\); \}/u,
    'one column — the drawer names the project');
});

test('the boxes ADAPT to their content (owner, 2026-08-29)', () => {
  // Editors grow with what is typed OR what openIssue swaps in; the display
  // box hugs its content instead of banding full-width blank.
  assert.match(source, /function autoGrow\(el: HTMLTextAreaElement/u, 'one grow helper');
  assert.match(source, /bind:value=\{draft\.body\} use:autoGrow=\{draft\.body\}/u, 'the body editor wears it');
  // The create form's body does NOT autoGrow: it FILLS the leftover height
  // and scrolls inside itself (owner, 2026-08-29 follow-up).
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.d-body-edit \{[^}]*min-height: 72px/u, 'min-height is a floor, not a size');
  assert.match(style, /\.n-text \{[^}]*width: fit-content/u, 'a short note is a small box');
  assert.ok(!style.includes('.n-body'), 'the display/input class collision is retired');
});

test('the create form: one-line growing title, assign, then a filling body (owner, 2026-08-29)', () => {
  const form = source.slice(source.indexOf("{:else if creating}"), source.indexOf("{:else}", source.indexOf("{:else if creating}")));
  const iTitle = form.indexOf('class="n-title one-line"');
  const iAssign = form.indexOf('<Select value={nAssignee}');
  const iBody = form.indexOf('class="d-body-edit fill"');
  assert.ok(iTitle >= 0 && iAssign > iTitle && iBody > iAssign, 'title, then assign, then the body');
  assert.match(form, /rows="1"[^>]*bind:value=\{nTitle\}[\s\S]{0,80}?use:autoGrow=\{nTitle\}/u,
    'the title starts as ONE line and grows with its text');
  const style = source.slice(source.indexOf('<style>'));
  assert.match(style, /\.n-title\.one-line \{ flex: none; resize: none; overflow: hidden; \}/u,
    'the title never stretches in the column and has no resize handle');
  assert.match(style, /\.d-body-edit\.fill \{ flex: 1;[^}]*overflow-y: auto/u,
    'the body takes the leftover height and scrolls INSIDE');
});

test('the detail speaks board #15: project-named page, status slider, confirmed changes', () => {
  // ① The page names the PROJECT, not itself — from the FULL list since
  // board #39: an empty board is hidden from the sidebar but keeps its name.
  assert.match(source, /<h1>\{boardTitle\(allProjects, cur\) \?\? \(cur \|\| t\('board'\)\)\}<\/h1>/u,
    'the title is the project name');
  // ② The status is a segmented SLIDER editing the draft: sweep (pointer
  // capture + move) or tap; nothing reaches the server until the ✓.
  assert.match(source, /onpointerdown=\{segDown\}/u, 'the track takes the pointer');
  assert.match(source, /onpointermove=\{\(e\) => segDrag && segPick\(e\)\}/u, 'sweeping slides the pick');
  assert.match(source, /onclick=\{\(\) => \(draft\.status = st\)\}/u, 'tapping a stop picks it — into the DRAFT');
  assert.ok(!source.includes('Select value={sel.status}'), 'the status Select is retired');
  // ③ The assignee edits the draft too — dirty raises the head\u2019s ✓/undo,
  // and only saveDraft dispatches.
  assert.match(source, /<Select value=\{draft\.assignee\}/u, 'the picker shows the draft');
  assert.match(source, /onchange=\{\(v: string\) => \(draft\.assignee = v\)\}/u, 'changing it is an edit, not a write');
});

test('the bar IS Chat\u2019s page-head, and the name appears once (board #15 reopen)', () => {
  // The shared app.css dialect carries height/padding/border/type — the
  // component may not re-style it (the drift that split the sidebars once).
  assert.match(source, /<div class="page-head">/u, 'the shared class, not a scoped .head');
  const style = source.slice(source.indexOf('<style>'));
  assert.ok(!style.includes('.page-head'), 'no scoped re-style of the shared bar');
  assert.ok(!/\.head \{/u.test(style), 'the old scoped head rules are retired');
  assert.ok(!source.includes('h-session'), 'the project name is written ONCE — the session chip retired');
});

test('embedded, the Board brings no head of its own — the drawer head is the head (board #23)', () => {
  // The embedded page-head only repeated the project name the drawer head
  // already carries ("不需要 project 单独显示一行了"): the WHOLE bar is gated,
  // not just the hamburger.
  assert.match(source, /\{#if !embedded\}\s*\n\s*<div class="page-head">/u,
    'the page-head renders only on the standalone page');
  // Creation still has one entry point per surface: the drawer head's + sends
  // a request, and the guard between it and a dirty draft still stands.
  assert.match(source, /const req = createRequest;[\s\S]{0,200}?guard\(\(\) => \{ sel = null; creating = true; \}\);/u,
    'a create request routes through the same dirty-draft guard as every other jump');
});

test('four areas tile as 1, 2 or 4 — never 3 — and the BOARD is the ruler (board #27)', () => {
  // auto-fit packed as many 170px tracks as fit, so a ~560–700px board (a
  // routine drawer width) showed 3 across with the fourth orphaned on its
  // own row — neither the side-by-side reading nor the stack. The count is
  // now a ladder of container queries against the BOARD itself, so the
  // standalone page and the Hub drawer obey the same thresholds by
  // construction and the viewport plays no part.
  assert.match(source, /\.board \{[^}]*container-type: inline-size/su, 'the board is an inline-size container');
  assert.match(source, /\.board \{[^}]*container-name: board/su, 'named, so the ladder cannot silently re-anchor');
  assert.ok(!/repeat\(auto-(?:fit|fill)/u.test(source), 'no auto-packing — that is where 3 came from');
  // The complete set of shapes .cols may take: 1 (the base — since board
  // #33 a COLUMN FLEX so sparse/dense areas can size independently), 2, 4.
  assert.match(source, /\.cols \{[^}]*display: flex;\s*flex-direction: column;/su, 'the base is ONE column (flex, adaptive)');
  assert.match(source, /@container board \(min-width: 350px\) \{\s*\.cols \{[^}]*display: grid;[^}]*grid-template-columns: repeat\(2, minmax\(0, 1fr\)\);/su,
    'the 2-step RESTORES the grid');
  assert.match(source, /@container board \(min-width: 710px\) \{\s*\.cols \{ grid-template-columns: repeat\(4, minmax\(0, 1fr\)\); \}\s*\}/u);
  assert.equal([...source.matchAll(/@container board/g)].length, 2, 'exactly the two steps — a third step is a fifth shape');
  assert.ok(!/repeat\(3/u.test(source), 'three columns exist nowhere');
  // The thresholds keep the cards' own 170px minimum honest (gap 10):
  // 2×170+10 = 350, 4×170+3×10 = 710 — and the 4-step sits above the 2-step.
  const steps = [...source.matchAll(/@container board \(min-width: (\d+)px\)/g)].map((m) => Number(m[1]));
  const [two = 0, four = 0] = steps;
  assert.ok(two < four, 'the ladder ascends');
  assert.ok(two >= 2 * 170 + 10 && four >= 4 * 170 + 3 * 10, 'each step affords its tracks at 170px min');
  // GRID modes (2-col stacks two rows) still share the height equally so
  // every area keeps its own scroller (the page holds still); since #33 the
  // equal-rows rule lives INSIDE the query — the 1-col base is adaptive.
  assert.match(source, /@container board \(min-width: 350px\) \{\s*\.cols \{[^}]*grid-auto-rows: minmax\(0, 1fr\);/su,
    'equal rows are a GRID rule, scoped to the ≥2-col steps');
  assert.ok(!/\.cols \{[^}]*grid-auto-rows[^}]*\}\s*@container/su.test(source.slice(0, source.indexOf('@container'))),
    'the base declares no equal rows — sparse areas must be free to hug content');
});

test('the 1-column base sizes areas by their CONTENT class — sparse hugs, dense shares (board #33)', () => {
  // Four unconditionally equal areas squeezed every real column to a
  // quarter-screen while empty ones stood stretched. The mechanism is ONE
  // class computed from the data, no fixed card heights, no JS measuring.
  assert.match(source, /\{@const items = col\(s\)\}/u, 'each area computes its items ONCE');
  assert.match(source, /class:sparse=\{items\.length <= 1\}/u, '0 or 1 card marks the area sparse');
  // The two flex behaviors, and only in the base (grid items ignore flex):
  assert.match(source, /\.colm\.sparse \{ flex: none; \}/u, 'sparse: header + content, nothing stretched');
  assert.match(source, /\.colm:not\(\.sparse\) \{ flex: 1 1 0; \}/u, 'dense areas share the leftover equally');
  // The count and the list read the SAME computation (no double col(s) call
  // that could disagree mid-poll).
  assert.match(source, /<span class="col-n">\{items\.length\}<\/span>/u, 'the header count reads items');
  assert.match(source, /\{#each items as i \(i\.id\)\}/u, 'the cards read items');
  // Every column keeps its own scroller in all shapes (#27's rule survives).
  assert.match(source, /\.col-scroll \{\s*overflow-y: auto;/su, 'the internal scroller stays');
});

test('the note reply wraps and grows — one autoGrow, chat keyboard semantics (board #28)', () => {
  // "发送消息如果消息过长要自动帮我换行，现在是一直在一行里，前边都看不到了":
  // the reply was a single-line <input> that scrolled horizontally. It is now
  // a textarea in the input's exact clothes — soft wrap is the element's
  // default, autoGrow raises it, one line at rest.
  assert.match(source, /<textarea class="note-input" rows="1"[^>]*bind:value=\{noteText\}/su,
    'the reply is a one-line-at-rest textarea');
  assert.match(source, /class="note-input"[^>]*use:autoGrow=\{noteText\}/su, 'it grows through the SHARED action');
  assert.ok(!/<input[^>]*noteText/su.test(source), 'the single-line input is gone');
  assert.equal([...source.matchAll(/function autoGrow/g)].length, 1, 'ONE autoGrow — no second copy');
  // Keyboard: Enter sends, Shift+Enter is a real newline (no preventDefault
  // on that path), and an IME composition's Enter commits the composition,
  // never the note. preventDefault on the send path keeps the sent text free
  // of a trailing newline.
  assert.match(source,
    /note-input[^>]*onkeydown=\{\(e\) => \{ if \(e\.key === 'Enter' && !e\.shiftKey && !e\.isComposing\) \{ e\.preventDefault\(\); addNote\(\); \} \}\}/su,
    'Enter sends; Shift+Enter and IME Enter do not');
  // Sending clears the bound value, and autoGrow's update refits — that is
  // the shrink-back path, so both halves must exist.
  assert.match(source, /noteText = ''; await refetchSel\(\);/u, 'send clears the value');
  assert.match(source, /update: \(_v: string\) => fit\(\)/u, 'the action refits when the bound value changes');
  // The dress is the input's own (shared rule with the create title), the
  // box never shows a scrollbar while measuring, and the send button rides
  // the LAST line instead of stranding mid-text.
  assert.match(source, /\.note-input, \.n-title \{/u, 'one dress, shared');
  assert.match(source, /\.note-input \{ resize: none; overflow: hidden; \}/u);
  assert.match(source, /\.note-add \{ display: flex; gap: 6px; align-items: flex-end; \}/u,
    'the button pins to the bottom line');
});

test('the create form submits from the keyboard: title Enter, body Cmd/Ctrl+Enter, IME never (board #36)', () => {
  // Owner (2026-09-01): "我在 board 填写完 issue 描述后，可以 cmd+enter 直接
  // 提交确认". The body is MULTI-LINE, so a bare Enter must stay a newline —
  // the submit chord is Cmd+Enter (mac) / Ctrl+Enter (everywhere else), the
  // pair every chat product speaks. Both modifiers, one handler: metaKey OR
  // ctrlKey, so the contract holds cross-platform.
  assert.match(source,
    /d-body-edit fill[^>]*onkeydown=\{\(e\) => \{ if \(e\.key === 'Enter' && \(e\.metaKey \|\| e\.ctrlKey\) && !e\.isComposing\) \{ e\.preventDefault\(\); createIssue\(\); \} \}\}/su,
    'the create body submits on Cmd/Ctrl+Enter only — a bare or Shift Enter falls through to a real newline');
  // The IME guard is load-bearing on BOTH create inputs: a composition's
  // Enter commits the candidate text, never the issue (the note box set the
  // precedent, board #28). The title's plain-Enter submit shipped without
  // the guard — a CJK title committed by Enter would have created the issue
  // mid-composition.
  assert.match(source,
    /n-title one-line[^>]*onkeydown=\{\(e\) => \{ if \(e\.key === 'Enter' && !e\.shiftKey && !e\.isComposing\) \{ e\.preventDefault\(\); createIssue\(\); \} \}\}/su,
    "the title's Enter submit carries the same isComposing guard");
  // Same createIssue as the ✓ button — the chord is a trigger, not a second
  // submit path (createIssue itself holds the not-contentless + busy gates,
  // so a submit with neither field filled is a no-op from any trigger).
  const triggers = [...source.matchAll(/e\.preventDefault\(\); createIssue\(\);/g)].length;
  assert.equal(triggers, 2, 'exactly two keyboard trigger sites (title, body) — the button references the fn');
  assert.match(source, /onclick=\{createIssue\}/u, 'the ✓ button shares the one submit');
  // The DETAIL editor deliberately does NOT take the chord (lead, board #36:
  // create body is the scope; the docs mandate no uniformity): its save is
  // the diffed, guarded saveDraft button.
  assert.ok(!/d-body-edit" [^>]*onkeydown/su.test(source), 'the detail body editor binds no keydown');
});

test('the sidebar consumes ONE bulk counts read and reacts to local writes at once (board #39)', () => {
  // The bulk read rides loadProjects' parallel batch — never a per-project
  // boardList walk, which is the N+1 hub_board_counts exists to prevent.
  assert.match(source, /boardCounts\(\)\.catch\(\(\) => null\)/u,
    'boardCounts joins the Promise.all batch, fail-soft');
  assert.equal([...source.matchAll(/boardList\(/g)].length, 1,
    'exactly ONE boardList call site — the current board\'s own load(), no sidebar walk');
  // The sidebar list is DERIVED through the pure filter: only boards with
  // issues render (total>0), and a local counts change re-filters without
  // waiting for any poll.
  assert.match(source, /const projects = \$derived\(visibleBoards\(allProjects, countsMap\)\);/u,
    'the shown rows are derived from the full list + counts');
  // load() folds the fresh issues into the counts map — create-first appears
  // and delete-last disappears NOW, not at the 20 s poll.
  const load = source.slice(source.indexOf('async function load()'), source.indexOf('let agents ='));
  assert.match(load, /countsMap = applyCounts\(countsMap, s, issues\);/u,
    'a successful boardList refreshes this session\'s counts immediately (keyed by the frozen session)');
  // The page-head names the CURRENT board from the FULL list: an empty board
  // is hidden from the sidebar yet still shows its name, and its main area
  // can create the first issue.
  assert.match(source, /<h1>\{boardTitle\(allProjects, cur\) \?\? \(cur \|\| t\('board'\)\)\}<\/h1>/u,
    'the h1 reads the unfiltered list with the old fallbacks');
  // No session to follow → the first NON-EMPTY board, i.e. the first row the
  // sidebar actually shows.
  assert.match(source, /if \(!cur\) cur = visibleBoards\(allProjects, countsMap\)\[0\]\?\.project\.session \?\? '';/u,
    'the default selection is the first visible board');
  // Each row's second line is the four columns in FIXED order, zeros
  // included, wearing the shared chip atoms + the one board status language.
  assert.match(source, /\{#each STATUSES as st \(st\)\}[\s\S]*?side-win-dot" style:background=\{countColor\(st\)\}[\s\S]*?\{statusLabel\(st\)\}[\s\S]*?\{c\?\.\[st as keyof BoardCountRow\] \?\? 0\}/u,
    'four chips, fixed vocabulary order, count present even at 0');
});

test('the four count chips adapt 4×1 / 2×2 / 1×4 — never a ragged 3+1 (board #39, owner)', () => {
  // Owner (2026-09-01), twice: "不要 3 1 这样的布局，列之间左侧点点要对齐",
  // then "如果一行能 放下放一行也行，甚至两行放不下，就放一列，动态适配".
  // The columns are EQUAL (1fr), so fitting is exact arithmetic on the
  // WIDEST chip — chipCols (pure, tested in board.test.ts) maps every width
  // into {4, 2, 1}. The measurement comes from a hidden GHOST row (the
  // composer's mirror-div pattern): same structure as a real row, height 0,
  // nowrap chips at natural width, carrying the sidebar's widest count so
  // digits are priced in.
  assert.match(source, /<div class="side-row proj-row mirror" aria-hidden="true">/u,
    'the ghost row measures, and assistive tech never hears it');
  assert.match(source, /bind:clientWidth=\{winsW\}/u, 'the ghost reports the row width');
  assert.match(source, /bind:clientWidth=\{chipWs\[i\]\}/u, 'each ghost chip reports its natural width');
  assert.match(source, /chipCols\(winsW, Math\.max\(\.\.\.chipWs\), CHIP_GAP_X\)/u,
    'the column count is the pure function of the measurements');
  assert.match(source, /<span class="side-wins grid" style:grid-template-columns=\{`repeat\(\$\{cols\}, minmax\(0, 1fr\)\)`\}>/u,
    'every real row wears the measured column count');
  // The shared base rule stays: 2×2 is the pre-measure fallback every width
  // can wear without a 3+1 flash.
  assert.match(appCss, /\.side-wins\.grid \{ display: grid; grid-template-columns: repeat\(2, minmax\(0, 1fr\)\); \}/u,
    'the app.css base: two equal columns until the mirror reports');
  // The ghost's own clothes are load-bearing: without them it renders as a
  // VISIBLE empty row and its chips wrap — which both shows a duplicate and
  // measures the wrapped (wrong) widths.
  assert.match(appCss, /\.proj-row\.mirror \{ height: 0; padding-top: 0; padding-bottom: 0; overflow: hidden; visibility: hidden; pointer-events: none; \}/u,
    'the ghost is zero-height, invisible and inert');
  assert.match(appCss, /\.proj-row\.mirror \.side-wins \{ flex-wrap: nowrap; \}[\s\S]{0,10}\.proj-row\.mirror \.side-win \{ flex: none; \}/u,
    'ghost chips sit nowrap at natural width — that IS the measurement');
});

test("the sidebar count chips wear the owner's four categorical colours — locally, never in the language (board #39, owner)", () => {
  // Owner (2026-09-01), third ruling on these chips: "done 和 todo 颜色又不
  // 一样了，四个设为 红 橙 黄 紫四个颜色吧" — the two near-greys (todo's
  // --text3 vs done's --status-sleep) read as inconsistent — then the FOURTH
  // (same day): "红 蓝 黄紫 吧，橙色和红色区分不明显" — at a 5px dot the
  // red/orange pair blurred, so doing swaps to BLUE. Four clearly-distinct
  // CATEGORICAL colours in board order: red, blue, yellow, purple; blue is
  // var(--accent), the app's own blue and incidentally the colour the global
  // language already gives doing. All theme tokens (a literal hex would only
  // be right in one theme), none of them the live dot's green — the first
  // ruling stands by construction. The remap stays deliberately scoped
  // (lead): the feed's "→ done" badge keeps boardStatusColor's language.
  assert.match(source,
    /const COUNT_COLORS: Record<string, string> = \{\s*todo: 'var\(--status-danger\)',\s*doing: 'var\(--accent\)',\s*review: 'var\(--status-warn\)',\s*done: 'var\(--status-purple\)',\s*\};/u,
    'the four-colour map: 红蓝黄紫 in board order, tokens only');
  assert.match(source, /const countColor = \(st: string\) => COUNT_COLORS\[st\] \?\? boardStatusColor\(st\);/u,
    'unknown statuses still delegate to the one language');
  // Purple is the only NEW token in the four-colour map. Pin BOTH theme
  // definitions: the component contract alone would stay green if one theme
  // silently lost the variable and rendered a transparent Done dot.
  const purples = [...appCss.matchAll(/--status-purple:\s*#([0-9a-f]{6})/giu)].map((m) => String(m[1]).toLowerCase());
  assert.deepEqual(purples, ['a78bfa', '7c3aed'], 'purple is defined once per theme: dark violet-400, light violet-600');
  assert.ok(!/side-win-dot" style:background=\{boardStatusColor\(st\)\}/.test(source),
    'the chip dot never bypasses the remap');
  // Colour is a garnish, not the identity: every chip carries its LABEL and
  // COUNT, so the four columns stay readable with no colour at all.
  assert.match(source, /\{statusLabel\(st\)\}<\/span>\s*<span class="b-count">/u,
    'label + count on every chip — non-colour-identifiable');
});

test('load() is pinned to the board it was asked FOR — a stale response never writes the new board (board #39 review)', () => {
  // The race: load() awaits boardList while the user switches projects; with
  // live `cur` the OLD board's response then paints its issues into the NEW
  // board and — since the counts fold — applies the old list's counts to the
  // new session, hiding/showing the wrong project in the sidebar. So the
  // session is FROZEN at entry, every RPC asks for the frozen name, and
  // every await re-checks identity before touching state.
  const load = source.slice(source.indexOf('async function load()'), source.indexOf('let agents ='));
  assert.match(load, /const s = cur;/u, 'the session is frozen at entry');
  assert.match(load, /await boardList\(s\)/u, 'the board read asks for the frozen session');
  assert.match(load, /await hubAgents\(s\)/u, 'the roster read asks for the frozen session');
  assert.ok(!/boardList\(cur\)|hubAgents\(cur\)/.test(load), 'no RPC in load() reads the LIVE cur');
  // Both awaited branches guard before writing — including the CATCH: an
  // error about a board we already left must not paint on the new one.
  const guards = [...load.matchAll(/if \(cur !== s\) return;/g)].length;
  assert.ok(guards >= 3, `every await (boardList ok/err, hubAgents ok) re-checks identity — found ${guards} guards`);
  assert.match(load, /countsMap = applyCounts\(countsMap, s, issues\);/u,
    'the counts fold names the FROZEN session, never live cur');
});

test('every destructive/discarding path confirms through the SHARED dialog (board #29)', () => {
  // Delete: the button only REQUESTS; nothing reaches boardDelete before the
  // confirm, and the executor uses the session+issue CAPTURED at request
  // time — a poll refetch, a selection change or a project switch while the
  // dialog stands open cannot redirect the delete.
  assert.match(source, /onclick=\{requestDelete\}/u, 'the delete button requests, never deletes');
  assert.match(source, /pendingDelete = \{ session: cur, id: sel\.id, title: issueRef\(sel\) \};/u,
    'the request captures its target — named via issueRef, a titleless issue confirms by its body (board #31)');
  assert.equal([...source.matchAll(/boardDelete\(/g)].length, 1, 'ONE call site, inside the confirm executor');
  assert.match(source, /await boardDelete\(cap\.session, cap\.id\);/u, 'the executor deletes the CAPTURED target');
  assert.match(source, /const cap = pendingDelete;\n\s*if \(!cap \|\| busy\) return;/u, 'busy blocks a double confirm');
  assert.match(source, /if \(cur === cap\.session\) \{\n\s*if \(sel\?\.id === cap\.id\) sel = null;/u,
    'success cleans only the MATCHING view');
  // The danger dialog is the shared component in the shared shape.
  assert.match(source, /<ConfirmDialog open=\{!!pendingDelete\} danger compact=\{narrowVp\} \{busy\}/u,
    'delete confirms in danger tone, phone-sheet aware, busy-held');
  assert.match(source, /t\('boardDeleteConfirmTitle'\)\.replace\('\{title\}', pendingDelete\?\.title \?\? ''\)/u,
    'the dialog names the captured issue — not whatever is selected now');
  // Discard: typed-but-uncreated create data is unsaved work; every exit
  // (explicit cancel, Escape, back, sidebar pick via guard) asks through the
  // SAME neutral dialog, a confirmed discard truly clears the form, and a
  // clean form navigates silently.
  assert.match(source, /const createDirty = \$derived\(creating && !!\(nTitle\.trim\(\) \|\| nBody\.trim\(\) \|\| nAssignee\)\);/u);
  assert.match(source, /if \(dirty \|\| createDirty\) pendingDiscard = action;/u, 'ONE guard covers both kinds of unsaved work');
  assert.match(source, /aria-label=\{t\('cancel'\)\} onclick=\{\(\) => guard\(\(\) => \(creating = false\)\)\}/u,
    'the create form\u2019s explicit cancel goes through the guard');
  assert.match(source, /if \(creating && createDirty\) \{ pendingDiscard = \(\) => \{ creating = false; \}; e\.stopPropagation\(\); return; \}/u,
    'Escape asks before dropping create data');
  assert.match(source, /if \(creating && createDirty\) \{ pendingDiscard = \(\) => \{ creating = false; \}; return true; \}/u,
    'the back gesture asks too');
  assert.match(source, /if \(pendingDelete\) \{ pendingDelete = null; return true; \}/u, 'back DISMISSES an open delete dialog, never confirms');
  assert.match(source, /pendingDiscard = null; draft = \{ \.\.\.draftBase \}; nTitle = ''; nBody = ''; nAssignee = '';/u,
    'a confirmed discard actually clears the create fields');
  assert.match(source, /<ConfirmDialog open=\{!!pendingDiscard\} danger=\{false\} compact=\{narrowVp\}/u,
    'discard confirms in neutral tone, phone-sheet aware');
  assert.match(source, /note=\{creating \? t\('boardCreateDiscardNote'\) : t\('boardDiscardNote'\)\}/u,
    'the words say WHAT is lost — an uncreated issue is not an unsaved edit');
  // No second confirmation species: no browser confirm(), no hand-rolled modal.
  assert.ok(!/window\.confirm|[^.\w]confirm\(/u.test(source), 'no browser confirm');
  assert.equal([...source.matchAll(/<ConfirmDialog /g)].length, 2, 'exactly the two shared dialogs');
  // The EXTERNAL session follow is a cur write point too (#29 review): the
  // last-touched session moving under typed create data would reset the view
  // and wipe it with no dialog at all. The gate blocks on BOTH kinds of
  // unsaved work and resumes the moment they clear; the other cur writes are
  // guard-wrapped (pick, the feed jump) or fire only while no board shows.
  assert.match(source, /\$effect\(\(\) => \{ if \(session && \(!picked \|\| !cur\) && !dirty && !createDirty\) cur = session; \}\);/u,
    'the follow gate blocks dirty AND createDirty');
});

test('titles are optional, and the WIRING honors it — not just the pure helpers (board #31)', () => {
  // The lead's review point: draftValid/issueRef being green proves the
  // helpers, not the component. These pins hold the four joints where a
  // regression would silently restore the title-only world.

  // 1) The create ENTRY and the create BUTTON both speak title||body — a
  //    body-only issue must be creatable from either path.
  assert.match(source, /if \(!\(nTitle\.trim\(\) \|\| nBody\.trim\(\)\) \|\| busy\) return;/u,
    'createIssue gates on title OR body');
  assert.match(source, /disabled=\{!\(nTitle\.trim\(\) \|\| nBody\.trim\(\)\) \|\| busy\} onclick=\{createIssue\}/u,
    'the create button disables only when BOTH are empty');

  // 2) The card's title is the shared fallback, never the raw field…
  assert.match(source, /<span class="c-title">\{issueRef\(i\)\}<\/span>/u,
    'a titleless card wears its body excerpt via issueRef');
  assert.ok(!/<span class="c-title">\{i\.title\}<\/span>/u.test(source),
    'the raw i.title rendering must not return');

  // 3) …and a titleless card does NOT repeat the same body as a preview.
  assert.match(source, /\{#if i\.body && i\.title\?\.trim\(\)\}<span class="c-body">/u,
    'the preview renders only under a real title — the same text twice reads as a bug');

  // 4) The wiring exists at all: issueRef is imported from the pure module.
  assert.match(source, /import \{[^}]*\bissueRef\b[^}]*\} from '\.\/board\.ts';/u,
    'issueRef comes from board.ts — no component-local copy');
});

test('locked issue text is static selectable prose; the workflow stays live (board #43)', () => {
  // Exactly TWO editable branches — the title and the body. editable comes
  // from the server (assignee empty AND no agent ever touched it, 842f970);
  // the frontend only consumes it.
  const branches = [...source.matchAll(/\{#if sel\.editable\}([\s\S]*?)\{\/if\}/gu)];
  assert.equal(branches.length, 2, 'title + body, nothing else forks on editable');

  // Editable keeps the REAL inputs; locked renders the ORIGINAL text as a
  // static div — never a disabled input (an input is an edit affordance
  // whether or not it accepts keys).
  assert.match(branches[0]![1]!, /<input class="d-title-input" bind:value=\{draft\.title\}/u, 'editable title is the input');
  assert.match(branches[0]![1]!, /\{:else if sel\.title\.trim\(\)\}\s*<div class="d-title-static">\{sel\.title\}<\/div>/u, 'locked title is static text');
  assert.match(branches[1]![1]!, /<textarea class="d-body-edit" bind:value=\{draft\.body\}/u, 'editable body is the textarea');
  // trim() may only GATE the render — the output is the original, verbatim:
  // a CLI-written body's leading/trailing newlines are history too, and
  // pre-wrap makes them real (review blocker on ebda03b).
  assert.match(branches[1]![1]!, /\{:else if sel\.body\.trim\(\)\}[\s\S]*<div class="d-body-static">\{sel\.body\}<\/div>/u, 'locked body renders RAW');
  assert.ok(!branches[1]![1]!.includes('d-body-static">{sel.body.trim()}'), 'the display layer never rewrites the record');
  assert.ok(!/d-title-input[^>]*\bdisabled\b/u.test(source) && !/d-body-edit[^>]*\bdisabled\b/u.test(source),
    'locked is a different ELEMENT, not a disabled input');

  // The workflow controls live OUTSIDE both branches: a locked issue still
  // slides status, picks an assignee, and takes note replies — the draft
  // machinery keeps patching those (with no input bound, a patch can never
  // carry title/body, matching the server's refusal).
  for (const [, inner] of branches) {
    assert.ok(!inner!.includes('class="seg"') && !inner!.includes('<Select') && !inner!.includes('note-input'),
      'status slider / assignee / note reply never fork on editable');
  }
  assert.match(source, /class="seg" role="radiogroup"/u, 'the slider is still there');
  assert.match(source, /class="note-input"/u, 'the note composer is still there');

  // Selectability is EXPLICIT (the app shell's global user-select:none only
  // opts inputs back in): locked title/body and every historical note body
  // carry the full trio.
  const style = source.slice(source.indexOf('<style>'));
  for (const cls of ['.d-title-static', '.d-body-static', '.n-text']) {
    const at = style.indexOf(cls);
    assert.ok(at > -1, `${cls} styled`);
    const block = style.slice(at, style.indexOf('}', at));
    assert.match(block, /user-select: text/u, `${cls} selectable`);
    assert.match(block, /-webkit-user-select: text/u, `${cls} selectable in WebKit`);
    assert.match(block, /cursor: text/u, `${cls} reads as text`);
  }
});

test('a note bubble reveals ONE Copy action in Chat\u2019s own dialect (board #46)', async () => {
  // ONE state variable answers "which row is open" — unique by construction:
  // the toggle closes the same index and switches to another.
  assert.match(source, /let noteOpen = \$state\(-1\);/u, 'one open-row state');
  assert.equal(source.split('{#if noteOpen === i}').length - 1, 1, 'exactly one action-row render site');
  assert.match(source, /noteOpen = noteOpen === i \? -1 : i;/u, 'tap the same note to close, another to switch');

  // The clipboard gets the RAW n.body — the display trims, the record does
  // not (the #43 verbatim rule, applied to what leaves the app).
  assert.match(source, /onclick=\{\(\) => copyNote\(n\.body\)\}/u, 'copy carries the raw body');
  assert.ok(!source.includes('copyNote(n.body.trim()'), 'never the trimmed rendering');
  assert.match(source, /navigator\.clipboard\.writeText\(body \?\? ''\)/u, 'the one clipboard write');
  // The Copied beat, then self-dismiss — Chat's 1.5 s.
  assert.match(source, /setTimeout\(\(\) => \{ if \(noteCopied\) \{ noteCopied = false; noteOpen = -1; \} \}, 1500\);/u,
    'copied → put the row away');

  // Close semantics: outside pointerdown, Escape as the topmost peel, and
  // every context switch (issue open, project change) resets.
  assert.match(source, /if \(!el\?\.closest\?\.\('\.m-acts, \.n-wrap, \.n-at'\)\) \{ noteOpen = -1; noteCopied = false; \}/u,
    'outside pointerdown closes');
  // Escape closes via a WINDOW-capture listener (the note text is a div, so
  // focus stays on <body> and a .bmain-scoped handler never hears the key),
  // standing down for open dialogs and stopping the press it consumes.
  assert.match(source, /if \(e\.key !== 'Escape' \|\| pendingDiscard \|\| pendingDelete\) return;\n\s+noteOpen = -1; noteCopied = false; e\.stopPropagation\(\);/u,
    'Escape peels the action row before the board\u2019s other layers');
  assert.match(source, /window\.addEventListener\('keydown', onEsc, true\);/u, 'at window capture, while a row is open');
  assert.match(source, /noteOpen = -1; noteCopied = false; \/\/ a different issue, a fresh slate/u, 'openIssue resets');
  assert.match(source, /pendingDelete = null; noteOpen = -1; noteCopied = false;/u, 'a project switch resets');

  // #43 must survive: a drag-selection's tail click never toggles the row.
  assert.match(source, /if \(typeof getSelection === 'function' && !\(getSelection\(\)\?\.isCollapsed \?\? true\)\) return;/u,
    'a live selection wins over the action row');

  // The dialect is SHARED, not copied: the atoms live in app.css (lifted
  // verbatim from Hub), and neither wearer re-styles them — the same
  // anti-drift rule the sidebar atoms follow.
  assert.match(appCss, /\.m-acts \{\n  position: absolute; z-index: 8; bottom: -13px; right: 10px;/u,
    'app.css owns the action row, out of the flow (absolute overlay)');
  assert.match(appCss, /\.m-acts button \{/u, 'and its buttons');
  assert.match(appCss, /--bubble-in: color-mix\(in srgb, var\(--bg\) 92%, white 8%\);/u,
    'the bubble surface token is global — the buttons render outside Hub');
  assert.ok(!/^\s*\.m-acts/mu.test(source.slice(source.indexOf('<style>'))), 'Board carries no scoped .m-acts rule');
  const hub = await readFile(new URL('./Hub.svelte', import.meta.url), 'utf8');
  assert.ok(!/^\s*\.m-acts/mu.test(hub.slice(hub.indexOf('<style>'))), 'Hub carries no scoped .m-acts rule either');
  assert.ok(!hub.includes("--bubble-in: color-mix"), 'Hub no longer re-declares the promoted token');

  // Accessibility: the note text stays TEXT (no role=button on it); the time
  // is the real focusable trigger, Chat's meta-trailer pattern.
  assert.ok(!/n-text[^>]*role=/u.test(source), 'the note body is text to assistive tech');
  assert.match(source, /<button class="n-at" aria-label=\{t\('hubMsgActions'\)\}\n\s+onclick=\{\(e\) => \{ e\.stopPropagation\(\);/u,
    'the time is the accessible trigger and never bubbles into the body toggle');

  // The overlay anchors to the bubble, not the row: relative fit-content
  // wrapper in Board's own clothes.
  assert.match(source, /\.n-wrap \{ position: relative; width: fit-content; max-width: 100%; \}/u,
    'the wrapper pins the overlay to the bubble corner');
  assert.match(source, /<div class="n-wrap">/u, 'and the markup wears it');
});
