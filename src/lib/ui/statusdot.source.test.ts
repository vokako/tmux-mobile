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
//   3. That halo is FOCUSED. Its first cut spent its light on reach — a
//      zero-blur spread ring plus a wide blur that also carried spread — and a
//      hard edge is read as the object's own boundary, so around a 6px dot it
//      said "bigger dot", not "live dot" (owner, 2026-08-29: "整个光晕看起来这
//      个点有点大 不是特别和谐 … 做得更加聚焦一点"). So every layer must be
//      blurred, no layer may carry spread, and the extents are capped: the
//      light gets brighter close in, never wider. The breathe amplitude is
//      capped with them, because `transform` scales the shadow too.
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

/** The comma-separated layers of a `box-shadow`, split at TOP level so the
    commas inside `color-mix(in srgb, …)` do not fake a layer boundary. */
const shadowLayers = (decls: string) => {
  const value = /box-shadow:([^;]*)/u.exec(decls)?.[1] ?? '';
  const out: string[] = [];
  let depth = 0, cur = '';
  for (const ch of value) {
    if (ch === '(') depth++;
    else if (ch === ')') depth--;
    if (ch === ',' && depth === 0) { out.push(cur); cur = ''; } else cur += ch;
  }
  if (cur.trim()) out.push(cur);
  return out.map((l) => l.trim()).filter(Boolean);
};

/** The leading length list of one layer: offset-x, offset-y, [blur], [spread].
    A `var(--x)` counts as a length and is resolved from the same rule. */
const lengths = (layer: string, vars: Map<string, number>) => {
  const out: number[] = [];
  for (const tok of layer.split(/\s+/u)) {
    const v = /^var\(\s*(--[\w-]+)\s*\)$/u.exec(tok);
    if (v) { const n = vars.get(String(v[1])); if (n === undefined) break; out.push(n); continue; }
    const n = /^(-?[\d.]+)(?:px)?$/u.exec(tok);
    if (!n) break;
    out.push(Number(n[1]));
  }
  return out;
};

/** Custom properties declared in a rule that hold a plain px length. */
const pxVars = (decls: string) => {
  const m = new Map<string, number>();
  for (const [, name, val] of decls.matchAll(/(--[\w-]+):\s*([\d.]+)px/gu)) m.set(String(name), Number(val));
  return m;
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

  // `[^{}]*` between stops, not `[^{]*`: the old class ran past the block's own
  // closing brace into whatever keyframes followed it (motion.md's intro set).
  const kf = /@keyframes\s+dot-breathe\s*\{(?:[^{}]*\{[^}]*\}\s*)+\}/u.exec(appCss)?.[0] ?? '';
  assert.ok(kf, 'dot-breathe must be defined');
  assert.match(kf, /transform:\s*scale/u, 'breathe is a SCALE — presence, per design-language.md');
  assert.ok(!/opacity/u.test(kf), 'an opacity fade converges on the idle grey; that is the old bug');

  // Every looping animation stills under reduced motion (design-language.md).
  const stilled = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.live-dot\s*\{([^}]*)\}/u.exec(appCss);
  assert.ok(stilled, 'the loop must still under prefers-reduced-motion');
  assert.match(String(stilled[1]), /animation:\s*none/u, 'reduced motion stops the loop');
  assert.ok(
    !/box-shadow/u.test(String(stilled[1])),
    'reduced motion must NOT touch the halo — with the loop stopped it is the whole cue',
  );
});

test('the halo is FOCUSED: every layer blurred, none spread, extents capped', () => {
  const live = rule(appCss, '.live-dot');
  const vars = pxVars(live);
  const layers = shadowLayers(live);
  assert.ok(layers.length >= 2, 'the halo is a bloom against the dot plus an outer falloff');

  let reach = 0;
  for (const layer of layers) {
    const len = lengths(layer, vars);
    assert.ok(len.length >= 3, `halo layer "${layer}" has no blur — offsets alone paint a hard copy of the dot`);
    const [dx, dy, blur, spread = 0] = len as [number, number, number, number?];
    assert.equal(dx, 0, 'a state dot glows in place; an offset halo reads as a shadow');
    assert.equal(dy, 0, 'a state dot glows in place; an offset halo reads as a shadow');
    assert.ok(blur > 0, `halo layer "${layer}" has blur 0 — a crisp edge is read as the DOT's boundary, so it enlarges it`);
    assert.equal(spread, 0, `halo layer "${layer}" carries ${spread}px of spread — spread pushes light outward, the opposite of focus`);
    // A CSS blur of b reaches ~b/2 past the edge it is centred on.
    reach = Math.max(reach, spread + blur / 2);
  }
  // 2.5px past a 5–7px dot's edge. The retired ring+wide-blur reached 4px and
  // the owner read the result as one oversized dot.
  assert.ok(reach <= 2.5, `the halo reaches ${reach}px past the dot — over 2.5px it stops being a rim and becomes a blob`);

  // The knobs themselves are capped so a later edit cannot re-inflate through
  // them, and the dense sidebar chip may only shrink what it overrides.
  const ring = vars.get('--live-ring'), glow = vars.get('--live-glow');
  assert.ok(ring !== undefined && glow !== undefined, '--live-ring / --live-glow stay the two named knobs');
  assert.ok(Number(ring) <= 2, `--live-ring is ${ring}px — the inner bloom hugs the dot`);
  assert.ok(Number(glow) <= 5, `--live-glow is ${glow}px — the outer falloff stays close`);

  const chip = rule(appCss, '.side-win .side-win-dot.live-dot');
  assert.ok(chip, 'the 5px sidebar chip keeps its proportional trim');
  for (const [, name, val] of chip.matchAll(/(--live-(?:ring|glow)):\s*([\d.]+)px/gu)) {
    assert.ok(
      Number(val) <= Number(vars.get(String(name))),
      `${name} is ${val}px on the dense chip, larger than the ${vars.get(String(name))}px base — the smallest dot cannot wear the widest halo`,
    );
  }

  // transform scales the box-shadow with the dot, so the peak amplitude is
  // part of the footprint, not just of the tempo.
  const scales = [...(/@keyframes\s+dot-breathe\s*\{(?:[^{]*\{[^}]*\}\s*)+\}/u.exec(appCss)?.[0] ?? '')
    .matchAll(/scale\(([\d.]+)\)/gu)].map((m) => Number(m[1]));
  assert.ok(scales.length >= 2, 'the breathe has a rest and a peak');
  const peak = Math.max(...scales);
  assert.ok(peak > 1, 'the breathe must actually move — the dynamic is the half a reader notices first');
  assert.ok(peak <= 1.35, `the breathe peaks at ${peak}× and swells the halo with it`);
});

test('every dot painted with stateDotColor also wears class:live-dot — colour alone is not the cue', async () => {
  // The drawer's window pills carried the colour without the motion (review C,
  // 2026-09-03): a running agent's pill read as a resting one. The rule is
  // general — the two halves of the cue travel together on the SAME element.
  const offenders: string[] = [];
  const walk = async (dir: URL) => {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const child = new URL(entry.name + (entry.isDirectory() ? '/' : ''), dir);
      if (entry.isDirectory()) { await walk(child); continue; }
      if (!/\.svelte$/u.test(entry.name)) continue;
      const rel = child.href.slice(SRC.href.length);
      const text = await readFile(child, 'utf8');
      for (const m of text.matchAll(/<[^<>]*style:background=\{stateDotColor\([^<>]*>/gu)) {
        if (!/class:live-dot=/u.test(m[0])) offenders.push(`${rel}: ${m[0].slice(0, 80)}`);
      }
    }
  };
  await walk(SRC);
  assert.deepEqual(offenders, []);
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
