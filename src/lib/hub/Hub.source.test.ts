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

  // The time stays pinned while the argument slides under it, and it needs the
  // lane's own background or the text would show through.
  const ts = rule('.step .st-ts');
  assert.match(ts, /position:\s*sticky/u);
  assert.match(ts, /right:\s*0/u);
  assert.match(ts, /background:\s*var\(--surface\)/u);
});

test('the row cap is still expressed in rows, not pixels', () => {
  // If this becomes a pixel height it stops following the type scale, and the
  // "ten rows" promise silently becomes "some height".
  assert.match(rule('.s-body.capped'), /max-height:\s*calc\(var\(--steps-rows\)/u);
});
