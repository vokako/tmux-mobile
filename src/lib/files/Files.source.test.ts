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
  // …back pops exactly that path FIRST; the user's own steps always outrank
  // the parent climb below them (board #47 reopened the climb — see the next
  // test — but never above the pop, and never through navTo).
  assert.match(source, /if \(popDir\(\)\) return true;[\s\S]{0,700}?if \(!jumped/u,
    'the back gesture\u2019s directory step is the pop, before the gated climb');
  assert.ok(!source.includes('if (cwd !== \'/\') { goUp(); return true; }'),
    'the UNGATED history-pushing parent walk stays retired (goUp/navTo would bounce)');
  // External moves are new ENTRY POINTS, not steps: they reset the history.
  const resets = source.split('dirHist = []').length - 1;
  assert.equal(resets, 4, 'the declaration + session switch, cwd follow rule, and navRequest handoff resets');
});

test('an OS drag onto the listing uploads into the CURRENT directory (board #22)', () => {
  // ONE destination rule for every upload entry point — and the DIR is a
  // PARAMETER bound at the gesture, never a live read: navigating away while
  // a batch uploads must not re-route the remaining files (review of
  // 82da1c9). Both helpers snapshot cwd on entry, every fsUpload routes
  // through uploadDest(dir, …), and nothing inside the loops re-reads cwd.
  assert.match(source, /const uploadDest = \(dir, name\) => dir\.replace\(\/\\\/\$\/, ''\) \+ '\/' \+ name;/u,
    'one destination rule, dir as a parameter');
  const helper = (name: string) => {
    const m = new RegExp(`async function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n  \\}`, 'u').exec(source)?.[0] ?? '';
    assert.ok(m, `${name} exists`);
    return m;
  };
  for (const name of ['uploadBlobFiles', 'uploadTauriPaths']) {
    const body = helper(name);
    assert.match(body, /const dir = cwd; \/\/ the batch's target, fixed at the gesture/u,
      `${name} snapshots its target ONCE, at entry`);
    assert.match(body, /uploadDest\(dir, /u, `${name} uploads to the snapshot`);
    assert.match(body, /refreshAfterBatch\(dir\);/u, `${name} refreshes via the snapshot rule`);
    assert.equal(body.split('cwd').length - 1, 1,
      `${name} reads cwd exactly once — the snapshot; nothing in the loop can see a navigation`);
  }
  // The refresh rule: still looking at the target → show the arrivals; moved
  // on → touch NOTHING (reloading the snapshot dir would hijack the view
  // back, reloading the new dir would announce files that landed elsewhere).
  assert.match(source, /const refreshAfterBatch = \(dir\) => \{ if \(cwd === dir\) loadDir\(dir\); \};/u,
    'refresh only the directory the user is still in');
  // The browser transport: HTML5 events on the listing, files only (an app-
  // internal drag carries no Files type and must pass through untouched).
  assert.match(source, /ondragover=\{onListDragOver\} ondragleave=\{onListDragLeave\} ondrop=\{onListDrop\}/u,
    'the listing is the drop target');
  assert.match(source, /Array\.from\(e\.dataTransfer\?\.types \|\| \[\]\)\.includes\('Files'\)/u,
    'only real OS file drags engage');
  assert.match(source, /if \(e\.currentTarget\.contains\(e\.relatedTarget\)\) return;/u,
    'entering a child row is not leaving the listing');
  // The compiled app's transport: the webview INTERCEPTS native drags, so the
  // drop arrives as the webview's event with PATHS. The listener exists ONLY
  // while this instance is visible — the real gate against the parked
  // page-layer twin, because bare checkVisibility() does NOT check the
  // visibility property (visibilityProperty defaults FALSE — review of
  // 82da1c9) and .page-layer.hidden is exactly visibility:hidden.
  assert.match(source, /if \(!isTauri \|\| !visible\) return;[\s\S]{0,700}?onDragDropEvent/u,
    'the webview listener mounts only while visible, and unmounts with it');
  assert.match(source, /checkVisibility\(\{ visibilityProperty: true, checkVisibilityCSS: true \}\)/u,
    'the defense-in-depth check names the option — a bare call ignores visibility:hidden');
  assert.match(source, /getComputedStyle\(el\)\.visibility !== 'hidden'/u,
    'and falls back to computed style where the API is missing');
  assert.match(source, /pos\.x \/ dpr/u, 'physical pixels are converted before the rect test');
  // A missed drop must not navigate the tab away (that tears down the app):
  // stray drags are neutralized at the window while Files is visible, browser
  // only — and removed when it is not.
  assert.match(source, /if \(!visible \|\| isTauri\) return;/u, 'the guard is visible-gated and browser-only');
  assert.match(source, /window\.removeEventListener\('drop', block\);/u, 'and it cleans up after itself');
});

test('below its own path, a tab visit climbs to the parent — never the terminal (board #47)', () => {
  // popDir retraces the user's OWN steps (board #17); when that stack is
  // exhausted a TAB visit climbs parent directories via loadDir — NEVER
  // navTo, which would push the child back onto DIR history for the next
  // back to bounce down. The synthetic climb has no forward browser entry,
  // so it must replenish the APP entry consumed by this pop; otherwise a
  // deep path stalls after the pre-existing entries run out. Only a
  // chat-jumped visit falls through to App's return slot (the conversation).
  assert.match(source, /if \(popDir\(\)\) return true;[\s\S]{0,900}?if \(!jumped && cwd && cwd !== '\/'\) \{\s*navAnim\('back'\);\s*navPush\(\);\s*loadDir\(cwd\.replace\(\/\\\/\[\^\/\]\+\\\/\?\$\/, ''\) \|\| '\/'\);\s*return true;\s*\}/u,
    'the climb sits under the user-path pop, replenishes app history, and loads without pushing dir history');
});
