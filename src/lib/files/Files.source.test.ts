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

test('an OS drag onto the listing uploads into the CURRENT directory (board #22)', () => {
  // ONE destination rule and ONE encoder for every upload entry point — the
  // toolbar picker and both drop transports route through the same helpers,
  // so they cannot disagree about where a file lands.
  assert.match(source, /const uploadDest = \(name\) => cwd\.replace\(\/\\\/\$\/, ''\) \+ '\/' \+ name;/u,
    'one destination rule');
  assert.match(source, /await fsUpload\(uploadDest\(file\.name\), b64\);/u, 'browser files route through it');
  assert.match(source, /await fsUpload\(uploadDest\(name\), bytesToB64\(bytes\)\);/u, 'tauri paths route through it');
  assert.equal(source.split('fsUpload(').length - 1, 2,
    'exactly the two helpers call fsUpload — no side channel');
  // The browser transport: HTML5 events on the listing, files only (an app-
  // internal drag carries no Files type and must pass through untouched).
  assert.match(source, /ondragover=\{onListDragOver\} ondragleave=\{onListDragLeave\} ondrop=\{onListDrop\}/u,
    'the listing is the drop target');
  assert.match(source, /Array\.from\(e\.dataTransfer\?\.types \|\| \[\]\)\.includes\('Files'\)/u,
    'only real OS file drags engage');
  assert.match(source, /if \(e\.currentTarget\.contains\(e\.relatedTarget\)\) return;/u,
    'entering a child row is not leaving the listing');
  // The compiled app's transport: the webview INTERCEPTS native drags, so
  // DataTransfer never carries files there — the drop arrives as the
  // webview's drag-drop event with PATHS, gated by a hit-test on the
  // listing's rect, physical→CSS px via devicePixelRatio, and
  // checkVisibility so the parked page-layer instance cannot claim it.
  assert.match(source, /onDragDropEvent/u, 'the webview event is subscribed in the app');
  assert.match(source, /pos\.x \/ dpr/u, 'physical pixels are converted before the rect test');
  assert.match(source, /checkVisibility/u, 'a hidden instance never wins the hit-test');
  // A missed drop must not navigate the tab away (that tears down the app):
  // stray drags are neutralized at the window while Files is visible, browser
  // only — and removed when it is not.
  assert.match(source, /if \(!visible \|\| isTauri\) return;/u, 'the guard is visible-gated and browser-only');
  assert.match(source, /window\.removeEventListener\('drop', block\);/u, 'and it cleans up after itself');
});
