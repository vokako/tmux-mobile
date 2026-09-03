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
  const still = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.chev,\s*\.flip,\s*\.state-ctl\s*\{\s*transition:\s*none;\s*\}\s*\.appear,\s*\.appear-rise,\s*\.appear-pop[^}]*\{\s*animation:\s*none;/u;
  assert.match(appCss, still, 'the atoms still under prefers-reduced-motion');
});

test('no component re-implements an atom or reaches for svelte/transition', () => {
  for (const file of components) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(root.length);
    assert.doesNotMatch(src, /\n\s*\.(chev|flip)\s*\{/u, `${rel}: .chev/.flip are app.css atoms — add the class, do not redefine it`);
    assert.doesNotMatch(src, /@keyframes\s+(fade-in|rise-in|pop-in)\b/u, `${rel}: the intro keyframes live in app.css`);
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
