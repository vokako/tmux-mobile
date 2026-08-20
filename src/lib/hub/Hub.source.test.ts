// Source-contract test for the tool lane's layout (see docs/conventions/testing.md).
//
// A lane row is ONE line per tool call, and the 10-row cap that keeps a live run
// from growing the conversation is a max-height calculated in single lines
// (`--steps-rows * 1.5em`). Two ways to break that silently:
//
//  - `white-space: pre` on the argument. A tool detail routinely contains real
//    newlines (a heredoc, a multi-line shell command), so `pre` turns one call
//    into a three-line row: the cap then shows four calls instead of ten, and
//    "one row per call" stops being true. `nowrap` collapses those newlines,
//    which is why it is the one allowed value here.
//  - letting the argument wrap or ellipsize again. The argument is the half of a
//    tool call worth reading and it is routinely wider than the lane, so the LANE
//    pans horizontally instead (owner, 2026-08-20: "这些参数应该左右可以滑动，查看
//    完整的参数"). An ellipsis put back would restore exactly the thing that made
//    the interesting text unreadable.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Hub.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('a tool-lane row stays one line, and the lane pans instead of truncating', () => {
  const text = rule('.step .st-text');
  assert.match(text, /white-space:\s*nowrap/u, 'nowrap keeps a multi-line detail on one row');
  assert.doesNotMatch(text, /white-space:\s*pre\b/u, 'pre would make a heredoc a three-line row');
  assert.doesNotMatch(text, /text-overflow:\s*ellipsis/u, 'the argument is what you opened the lane to read');
  assert.doesNotMatch(text, /overflow:\s*hidden/u);

  // The row is as wide as its content, but never narrower than the lane — or a
  // short row's time would sit mid-row and the time column would go ragged.
  const step = rule('.step');
  assert.match(step, /width:\s*max-content/u);
  assert.match(step, /min-width:\s*100%/u);

  // One scroller for the whole block, so every row pans together.
  assert.match(rule('.s-body'), /overflow-x:\s*auto/u);

  // THREE columns: the tool name pinned left, the time pinned right, and only the
  // argument between them moves. Both pinned columns stick at the lane's OWN
  // padding — not at 0 — or they jump sideways the moment you start panning, and
  // both need the lane's background or the argument would slide out from under
  // them (owner, 2026-08-20: "工具明固定保持在最左侧 … 相当于三列").
  const name = rule('.step .tname');
  assert.match(name, /position:\s*sticky/u);
  assert.match(name, /left:\s*var\(--lane-indent\)/u);
  // The background has to be OPAQUE or the argument shows straight through it:
  // `--surface` is a 3% wash, so `--lane-bg` (canvas + the same wash) is the value
  // both pinned columns must paint. This is the bug the owner saw as "文字都叠加在
  // 一块了".
  assert.match(name, /background:\s*var\(--lane-bg\)/u);
  assert.doesNotMatch(name, /background:\s*var\(--surface\)/u);

  const ts = rule('.step .st-ts');
  assert.match(ts, /position:\s*sticky/u);
  assert.match(ts, /right:\s*var\(--lane-pad-r\)/u);
  assert.match(ts, /background:\s*var\(--lane-bg\)/u);
  assert.doesNotMatch(ts, /background:\s*var\(--surface\)/u);

  // A flex gap would be a transparent strip the panning argument shows through,
  // right next to a column whose job is to cover it — so the separation lives
  // inside the pinned columns, as padding they paint.
  assert.match(rule('.step'), /gap:\s*0/u);
  assert.match(name, /padding-right:/u);
  assert.match(ts, /padding-left:/u);

  // The offsets the two columns pin at ARE the lane's padding: one number, or the
  // columns and the rows disagree about where the row starts.
  // The lane's colour and its two offsets live on `.steps`, the element every
  // part of the lane inherits from: the body, both pinned columns and the "show
  // all" button all measure from the same numbers. A second copy anywhere is how
  // the column and the rows drift apart.
  const steps = rule('.steps');
  assert.match(steps, /--lane-bg:\s*linear-gradient\(var\(--surface\), var\(--surface\)\), var\(--chat-canvas\)/u);
  assert.match(steps, /--lane-indent:\s*30px/u);
  assert.match(steps, /--lane-pad-r:\s*10px/u);
  assert.match(rule('.s-body'), /padding:\s*5px var\(--lane-pad-r\) 6px var\(--lane-indent\)/u);
  assert.match(rule('.s-all'), /padding:\s*2px var\(--lane-pad-r\) 5px var\(--lane-indent\)/u);
});

test('the row cap is still expressed in rows, not pixels', () => {
  // If this becomes a pixel height it stops following the type scale, and the
  // "ten rows" promise silently becomes "some height".
  assert.match(rule('.s-body.capped'), /max-height:\s*calc\(var\(--steps-rows\)/u);
});
