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
  assert.match(source, /async function dispatchAssign\(id: number, name: string, title = '', body = ''\)/u, 'the one dispatch function');
  assert.equal(source.split('hubPost(').length - 1, 1, 'exactly one delivery call site, inside dispatchAssign');
  assert.match(source, /if \(assignee !== undefined\) await dispatchAssign\(sel\.id, assignee, draft\.title, draft\.body\);/u,
    'a ✓-confirmed assignee change routes through it (board #15: the picker edits the DRAFT)');
  assert.match(source, /if \(wantAssign && created != null\) await dispatchAssign\(created, wantAssign, wantTitle, wantBody\);/u,
    'create-with-assignee dispatches too — never just a label, and the brief CARRIES the issue');
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
  assert.match(source, /<\/div>\s*<div class="n-text">\{n\.body\.trim\(\)\}<\/div>/u, 'the content is its own box below, trimmed');
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
  // ① The page names the PROJECT, not itself.
  assert.match(source, /<h1>\{projects\.find\(\(p\) => p\.project\.session === cur\)\?\.project\.name \?\? \(cur \|\| t\('board'\)\)\}<\/h1>/u,
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
  // The complete set of shapes .cols may take: 1 (the base), 2, 4.
  assert.match(source, /\.cols \{[^}]*grid-template-columns: minmax\(0, 1fr\);/su, 'the base is ONE column');
  assert.match(source, /@container board \(min-width: 350px\) \{\s*\.cols \{ grid-template-columns: repeat\(2, minmax\(0, 1fr\)\); \}\s*\}/u);
  assert.match(source, /@container board \(min-width: 710px\) \{\s*\.cols \{ grid-template-columns: repeat\(4, minmax\(0, 1fr\)\); \}\s*\}/u);
  assert.equal([...source.matchAll(/@container board/g)].length, 2, 'exactly the two steps — a third step is a fifth shape');
  assert.ok(!/repeat\(3/u.test(source), 'three columns exist nowhere');
  // The thresholds keep the cards' own 170px minimum honest (gap 10):
  // 2×170+10 = 350, 4×170+3×10 = 710 — and the 4-step sits above the 2-step.
  const steps = [...source.matchAll(/@container board \(min-width: (\d+)px\)/g)].map((m) => Number(m[1]));
  const [two = 0, four = 0] = steps;
  assert.ok(two < four, 'the ladder ascends');
  assert.ok(two >= 2 * 170 + 10 && four >= 4 * 170 + 3 * 10, 'each step affords its tracks at 170px min');
  // Stacked modes share the height equally so EVERY area keeps its own
  // scroller (the page holds still — reopened #11's rule survives 1/2-col).
  assert.match(source, /\.cols \{[^}]*grid-auto-rows: minmax\(0, 1fr\);/su, 'rows split the height; content cannot content-size them');
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
