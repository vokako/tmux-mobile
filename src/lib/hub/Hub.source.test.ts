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

test('a held ask caps only its EXPANDED body, never the bubble', () => {
  // The bubble itself must stay uncapped: a max-height on `.held`'s flow box is
  // what fed Chromium's scroll anchoring and produced the infinite blink
  // (measured 2026-08-19). The one legitimate cap is the BODY, in the one state
  // where the user explicitly expanded a long ask while it is pinned — a sticky
  // bubble ignores the feed's scrolling, so without its own scroller the bottom
  // half of a screen-tall message is unreachable (owner, 2026-08-20).
  const heldMsg = rule('.msg.held');
  const heldBubble = rule('.msg.held .bubble');
  assert.doesNotMatch(heldMsg, /max-height/u);
  assert.doesNotMatch(heldBubble, /max-height/u);

  const body = rule('.msg.held .m-body.held-scroll');
  assert.match(body, /max-height/u);
  assert.match(body, /overflow-y:\s*auto/u);
  assert.match(body, /overscroll-behavior:\s*contain/u, 'its wheel must not fling the feed');

  // And the state is entered by an explicit click, not by the boundary test:
  // the class hangs off heldExpanded === key in the markup.
  assert.match(source, /class:held-scroll=\{heldScroll\}/u);
  assert.match(source, /heldExpanded === key/u);
});

test('the drawer opens and closes through the reading anchor, everywhere', () => {
  // The drawer regrids the columns and every message rewraps; without the anchor
  // the reader's message drifts (owner, 2026-08-20). One open path and one close
  // path, both wrapped — a bare `termOpen = true/false` outside selectProject is
  // a trigger someone forgot to route.
  const bare = [...source.matchAll(/termOpen = (?:true|false)/g)].length;
  assert.equal(bare, 3, 'selectProject reset + the two wrapped mutations, nothing else');
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = true; \}\)/u);
  assert.match(source, /withReadingAnchor\(\(\) => \{ termOpen = false; \}\)/u);
  // The reference skips every sticky variant: a pinned rect does not move with
  // the flow, so anchoring to it restores nothing.
  assert.match(source, /!el\.classList\.contains\('held'\)/u);
  assert.match(source, /!el\.classList\.contains\('ask-top'\)/u);
  assert.match(source, /!el\.classList\.contains\('ask-bottom'\)/u);
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
});
