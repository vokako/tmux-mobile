// Source-contract test for the RUNNING cue (see docs/conventions/testing.md).
//
// The status colour language (design-language.md §Colour) says accent = in
// motion, grey = at rest — and at 5–7px the colour alone was not carrying it:
// the owner reported running and idle as hard to tell apart twice (2026-08-26,
// again 2026-08-29). Two things fixed it and both regress silently:
//
//   1. `--status-sleep` is ACHROMATIC. It was a blue-leaning grey sitting in
//      the accent's own hue family — in light theme within 3° of it. A grey
//      with a blue cast is the bug, so the token's channels must be equal-ish.
//   2. The motion is a SCALE breathe with a halo, never an opacity fade. The
//      retired `s-pulse` faded a running dot to opacity 0.35, which composited
//      it toward the card: measured on the dark card, the trough sat 22 L*
//      points DARKER than the idle grey, so half of every cycle the running dot
//      read LESS alive than a resting one. An opacity keyframe on a state dot
//      is therefore banned outright, not merely discouraged.
//
// And the cue is ONE mechanism: defined once in app.css, worn by class. A
// component that re-implements it is how the dialects split.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';

const SRC = new URL('../../', import.meta.url);   // src/
const appCss = await readFile(new URL('app.css', SRC), 'utf8');

/** The declaration block of the first rule whose selector matches exactly.
    Comments are stripped first — a comment CLOSER right above a selector is
    otherwise indistinguishable from being mid-rule. */
const rule = (css: string, selector: string) => {
  const re = new RegExp('(?:^|[};])\\s*' + selector.replace(/\./gu, '\\.') + '\\s*\\{([^}]*)\\}', 'u');
  return re.exec(css.replace(/\/\*[\s\S]*?\*\//gu, ''))?.[1] ?? '';
};

test('at-rest grey is achromatic in both themes — never an accent neighbour', () => {
  const sleeps = [...appCss.matchAll(/--status-sleep:\s*#([0-9a-f]{6})/giu)].map((m) => String(m[1]));
  assert.equal(sleeps.length, 2, 'one --status-sleep per theme, dark and light');
  for (const hex of sleeps) {
    const ch = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
    // A neutral grey has near-equal channels. The tolerance allows the slight
    // cool lift the tokens carry; the retired #7c8698 / #94a0b0 spanned 28.
    const spread = Math.max(...ch) - Math.min(...ch);
    assert.ok(spread <= 12, 'sleep grey ' + hex + ' has a ' + spread + '-point colour cast — at-rest must be grey');
  }
});

test('the running cue is defined once in app.css: halo + breathe, and it stills', () => {
  const live = rule(appCss, '.live-dot');
  assert.ok(live, '.live-dot must be defined in app.css — it is the app-wide cue');
  assert.match(live, /box-shadow:/u, 'the halo is the greyscale/reduced-motion-proof half');
  assert.match(live, /--live-hue/u, 'the halo takes the dot’s OWN hue, so no new colour species');
  assert.match(live, /animation:\s*dot-breathe/u, 'the motion is the shared breathe');

  const kf = /@keyframes\s+dot-breathe\s*\{(?:[^{]*\{[^}]*\}\s*)+\}/u.exec(appCss)?.[0] ?? '';
  assert.ok(kf, 'dot-breathe must be defined');
  assert.match(kf, /transform:\s*scale/u, 'breathe is a SCALE — presence, per design-language.md');
  assert.ok(!/opacity/u.test(kf), 'an opacity fade converges on the idle grey; that is the old bug');

  // Every looping animation stills under reduced motion (design-language.md).
  assert.match(
    appCss,
    /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.live-dot\s*\{\s*animation:\s*none/u,
    'the loop must still under prefers-reduced-motion (the halo carries it alone then)',
  );
});

test('no component re-implements the cue, and the retired fade stays retired', async () => {
  const offenders: string[] = [];
  const files: URL[] = [];
  const walk = async (dir: URL) => {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const child = new URL(entry.name + (entry.isDirectory() ? '/' : ''), dir);
      if (entry.isDirectory()) await walk(child);
      else if (/\.svelte$/u.test(entry.name)) files.push(child);
    }
  };
  await walk(SRC);
  for (const file of files) {
    const rel = file.href.slice(SRC.href.length);
    const text = await readFile(file, 'utf8');
    if (/s-pulse/u.test(text)) offenders.push(`${rel}: names the retired s-pulse fade`);
    // A component may set --live-hue (Team's amber/orange states) but must not
    // redeclare the cue itself.
    const style = /<style>([\s\S]*)<\/style>/u.exec(text)?.[1] ?? '';
    const body = rule(style, '.live-dot');
    if (/box-shadow|animation/u.test(body)) offenders.push(`${rel}: restyles .live-dot`);
  }
  assert.deepEqual(offenders, []);
});
