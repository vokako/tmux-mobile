// Source contract for the micro-motion vocabulary (design-language.md §1
// "Micro-motion"): ONE set of atoms in app.css, on the two tempo tokens, stilled
// under reduced motion, and no component re-implements them.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('../../..', import.meta.url).pathname;
const appCss = readFileSync(join(root, 'src/app.css'), 'utf8');
const motionTs = readFileSync(join(root, 'src/lib/ui/motion.ts'), 'utf8');

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}
const components = walk(join(root, 'src'));

test('the tempo tokens and the JS mirror agree', () => {
  const tokens = /--t-fast:\s*(\d+)ms;\s*--t-move:\s*(\d+)ms;/u.exec(appCss);
  assert.ok(tokens, 'app.css declares --t-fast and --t-move together');
  assert.equal(motionTs.includes(`T_FAST_MS = ${tokens![1]}`), true, 'T_FAST_MS mirrors --t-fast');
  assert.equal(motionTs.includes(`T_MOVE_MS = ${tokens![2]}`), true, 'T_MOVE_MS mirrors --t-move');
});

test('the four atoms exist once, on the tokens, and still under reduced motion', () => {
  assert.match(appCss, /\.chev,\s*\.flip\s*\{[^}]*transition:\s*transform var\(--t-move\)/u, 'a turning glyph moves on --t-move');
  assert.match(appCss, /\.chev\.open\s*\{\s*transform:\s*rotate\(90deg\)/u);
  assert.match(appCss, /\.flip\.on\s*\{\s*transform:\s*rotate\(180deg\)/u);
  for (const kf of ['fade-in', 'rise-in', 'pop-in']) {
    assert.match(appCss, new RegExp(`@keyframes ${kf}\\s*\\{\\s*from\\s*\\{[^}]*\\}\\s*\\}`, 'u'), `${kf} is an intro-only keyframe`);
    assert.doesNotMatch(appCss, new RegExp(`@keyframes ${kf}[^}]*(height|width|top|left|margin)`, 'u'), `${kf} animates transform/opacity only`);
  }
  assert.match(appCss, /\.appear\s*\{\s*animation:\s*fade-in var\(--t-fast\)/u);
  assert.match(appCss, /\.appear-rise\s*\{\s*animation:\s*rise-in var\(--t-move\)/u);
  assert.match(appCss, /\.appear-pop\s*\{\s*animation:\s*pop-in var\(--t-fast\)/u);
  assert.match(appCss, /\.state-ctl\s*\{\s*transition:[^}]*border-color var\(--t-fast\)[^}]*background var\(--t-fast\)[^}]*color var\(--t-fast\)/u);
  const still = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.chev,\s*\.flip,\s*\.state-ctl[^{]*\{\s*transition:\s*none;\s*\}\s*\.appear,\s*\.appear-rise,\s*\.appear-pop[^}]*\{\s*animation:\s*none;/u;
  assert.match(appCss, still, 'the atoms still under prefers-reduced-motion');
});

test('the wave-2 atoms: popover intro, sliding indicator, reveal, skeleton', () => {
  assert.match(appCss, /\.pop-layer\s*\{[^}]*opacity:\s*0;[^}]*transform:\s*scale\(0\.96\);[^}]*transform-origin:\s*var\(--pop-origin/u, 'a popover is invisible and slightly small until placed');
  assert.match(appCss, /\.pop-layer\.ready\s*\{[^}]*transform:\s*none;[^}]*transition:[^}]*var\(--t-fast\)/u, 'it grows on --t-fast and rests with no transform');
  assert.match(appCss, /\.slide-pill\.ready\s*\{\s*transition:[^}]*transform var\(--t-move\)/u, 'the indicator glides on --t-move, only once placed');
  assert.doesNotMatch(appCss, /\.slide-ind\b/u, 'the bar indicator is retired — the wash behind the item travels (owner, 2026-09-04)');
  assert.match(appCss, /\.slide-pill\.soft \{ box-shadow: none; \}/u, 'an icon bar wants only the wash');
  assert.match(appCss, /\.slide-pill\.inset \{[^}]*--ind-inset/u, 'the inset pill hugs icon + label');
  assert.match(appCss, /\.reveal > \*,\s*\.reveal-tail > \*\s*\{\s*animation:\s*rise-in var\(--t-move\) ease-out backwards;/u, 'a loaded list unfolds with backwards fill only');
  assert.match(appCss, /\.reveal-tail > :nth-last-child\(2\)/u, 'a feed unfolds from its newest row');
  assert.match(appCss, /\.skel::after\s*\{[^}]*animation:\s*shimmer/u);
  assert.match(appCss, /\.skel-wrap\s*\{[^}]*animation-delay:\s*150ms/u, 'a skeleton waits 150ms so a fast load never flashes one');
  const still = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[^@]*\.pop-layer\.ready,\s*\.slide-pill\.ready\s*\{\s*transition:\s*none;[^@]*\.reveal > \*,\s*\.reveal-tail > \*,\s*\.skel::after,\s*\.skel-wrap\s*\{\s*animation:\s*none;/u;
  assert.match(appCss, still, 'the wave-2 atoms still under prefers-reduced-motion');
});

test('every placed popover wears .pop-layer and no component keeps a private opacity gate', () => {
  const popovers = ['src/lib/ui/ContextMenu.svelte', 'src/lib/ui/Select.svelte', 'src/lib/sessions/PanePicker.svelte', 'src/lib/ui/HoverCard.svelte'];
  for (const rel of popovers) {
    const src = readFileSync(join(root, rel), 'utf8');
    assert.match(src, /class="[^"]*\bpop-layer\b[^"]*" class:ready=/u, `${rel}: the placed layer wears .pop-layer with the ready gate`);
    assert.match(src, /style:--pop-origin=/u, `${rel}: tells the atom which corner it grows from`);
  }
  for (const rel of ['src/lib/hub/Hub.svelte', 'src/App.svelte']) {
    const src = readFileSync(join(root, rel), 'utf8');
    assert.match(src, /class="(a-menu|server-menu) pop-layer" class:ready=/u, `${rel}: its menu wears .pop-layer`);
  }
  for (const file of components) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(root.length);
    assert.doesNotMatch(src, /\.(ctx|sel-menu|a-menu|server-menu|picker|hover-card)\.ready\s*\{\s*opacity:\s*1/u, `${rel}: the .ready opacity gate is the atom's, not the component's`);
  }
});

test('no component re-implements an atom or reaches for svelte/transition', () => {
  for (const file of components) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(root.length);
    assert.doesNotMatch(src, /\n\s*\.(chev|flip)\s*\{/u, `${rel}: .chev/.flip are app.css atoms — add the class, do not redefine it`);
    assert.doesNotMatch(src, /@keyframes\s+(fade-in|rise-in|pop-in|sheet-up|drill-in-right|drill-in-left)\b/u, `${rel}: the shared keyframes live in app.css — reference them by name`);
    assert.doesNotMatch(src, /from ['"]svelte\/transition['"]/u, `${rel}: intros are the .appear* classes, exits are a snap — no svelte/transition`);
    if (/animate:flip/u.test(src)) {
      assert.match(src, /moveMs\(\)/u, `${rel}: animate:flip takes its duration from moveMs() (reduced-motion gate)`);
    }
  }
});

test('every looping animation in a component stills under reduced motion', () => {
  for (const file of components) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(root.length);
    const loops = [...src.matchAll(/animation:\s*([\w-]+)[^;]*\binfinite\b/gu)].map((m) => m[1]);
    if (!loops.length) continue;
    const stilled = /@media\s*\(prefers-reduced-motion:\s*reduce\)/u.test(src);
    assert.ok(stilled, `${rel}: loops ${loops.join(', ')} need a prefers-reduced-motion rule`);
  }
});
