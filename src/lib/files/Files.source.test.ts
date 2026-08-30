import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Files.svelte', import.meta.url), 'utf8');

test('back retraces the USER\u2019s steps — a history, not a parent walk (board #17)', () => {
  // Every user navigation pushes where they WERE…
  assert.match(source, /function navTo\(path\) \{\s*\n\s*if \(cwd && path !== cwd\) dirHist\.push\(cwd\);/u,
    'one navigate-with-history helper');
  for (const site of [
    /navTo\(entry\.path\);/u,          // entering a directory
    /navTo\(parent\);/u,               // the up button
    /onclick=\{\(\) => navTo\('\/'\)\}/u, // the root crumb
    /navTo\(bc\.path\)/u,              // a crumb
    /navTo\(bm\);/u,                   // a bookmark
  ]) assert.match(source, site, `user navigation pushes: ${site}`);
  // …back pops exactly that path; an empty stack falls to the FLOOR, where
  // App returns a chat jump to the chat (rule 1) instead of climbing to /.
  assert.match(source, /if \(popDir\(\)\) return true;\s*\n\s*return false;/u,
    'the back gesture\u2019s directory step is the pop, and the entry point is the floor');
  assert.ok(!source.includes('if (cwd !== \'/\') { goUp(); return true; }'), 'the parent walk to / is retired');
  // External moves are new ENTRY POINTS, not steps: they reset the history.
  const resets = source.split('dirHist = []').length - 1;
  assert.equal(resets, 4, 'the declaration + session switch, cwd follow rule, and navRequest handoff resets');
});
