// Source-contract test for the shared "back to the tail" button (board #49).
//
// Every scrolling record in this app — the chat feed, the terminal — floats
// the SAME "jump to the newest" button: `.to-tail` in app.css. It used to be
// two species (owner, board #49: "terminal和chat一键滚动最下的下箭头按钮风格
// 不一致"): a 36px rounded square in glass rgba with an always-accent arrow
// and a span dot, next to a 38px surface circle with a hover-accent arrow
// and an ::after dot. One look now lives in app.css; a component's scoped
// rule may only PLACE its instance (bottom/right/z-index) — a scoped
// re-declaration outranks the shared class silently (0,2,0 vs 0,1,0), which
// is exactly how the two drifted apart in the first place.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const SRC = new URL('../../', import.meta.url); // src/

/** Every wearer: [file, the scoped placement class it may still have]. */
const WEARERS: Array<[string, string]> = [
  ['lib/hub/Hub.svelte', 'to-bottom'],
  ['lib/terminal/Terminal.svelte', 'scroll-btn'],
];

test('the to-tail atoms live in app.css, once', async () => {
  const css = await readFile(new URL('app.css', SRC), 'utf8');
  assert.equal(css.match(/^\.to-tail \{/gmu)?.length, 1, 'one definition');
  const block = /\.to-tail \{([\s\S]*?)\}/u.exec(css)?.[1] ?? '';
  assert.match(block, /width: 38px; height: 38px; border-radius: 50%/u, 'one circle');
  assert.match(block, /background: var\(--surface\); border: 1px solid var\(--border\); color: var\(--text2\)/u,
    'surface ground, quiet ink — tokens, not rgba literals');
  assert.match(css, /\.to-tail:hover \{ color: var\(--accent\); border-color: var\(--accent\); \}/u,
    'the arrow answers hover in accent');
  assert.match(css, /\.to-tail:active \{ transform: scale\(0\.9\); \}/u, 'the tactile press');
  assert.match(css, /\.to-tail::before \{ content: ''; position: absolute; inset: -3px; \}/u,
    'the 44px touch overlay travels with the class');
  // ONE "something arrived below" cue: the news dot, in the status language.
  assert.match(css, /\.to-tail\.news::after \{[\s\S]*?background: var\(--status-danger\); border: 2px solid var\(--bg\);/u,
    'the news dot is the shared ::after, coloured by token');
  // A true circle takes no part in the squircle corner policy: the retired
  // .scroll-btn entry left app.css entirely.
  assert.ok(!css.includes('.scroll-btn'), 'the retired .scroll-btn entry left the squircle list');
});

test('both records wear the shared class and keep only placement locally', async () => {
  for (const [file, placeClass] of WEARERS) {
    const raw = await readFile(new URL(file, SRC), 'utf8');
    assert.match(raw, /class="to-tail [a-z-]+" class:news=/u, `${file}: wears .to-tail with the news cue`);
    assert.match(raw, /class="to-tail [a-z-]+"[\s\S]{0,220}?aria-label=/u, `${file}: the floating action is labelled`);
    assert.match(raw, /class="to-tail [a-z-]+"[\s\S]{0,220}?<Icon name="arrow-down" size=\{16\} \/>/u,
      `${file}: one arrow, one size`);
    const style = /<style>([\s\S]*)<\/style>/.exec(raw)?.[1] ?? '';
    const css = style.replace(/\/\*[\s\S]*?\*\//gu, '');
    // The scoped rule may place, never restyle: no box, no ground, no ink,
    // no second dot — those atoms have one home.
    const scoped = css.match(new RegExp(`^\\s*\\.${placeClass}[^{]*\\{([^}]*)\\}`, 'gmu')) ?? [];
    for (const rule of scoped) {
      assert.ok(!/width|height|border-radius|background|color|box-shadow|::after|::before/u.test(rule),
        `${file}: scoped .${placeClass} places only — found a style atom in ${rule.trim()}`);
    }
    assert.ok(!/\.new-dot|has-new/u.test(raw), `${file}: the span-dot species is retired`);
  }
});
