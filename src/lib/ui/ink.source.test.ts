// Board #321 (owner 2026-10-09: "左侧大侧边栏的灰色 icon 图标…拐折处的那一点比较
// 亮"): a translucent ink composites once PER drawn element, so wherever two
// strokes of one icon overlap the overlap is brighter. Ink tokens are opaque;
// --text3 is the old translucent ink composited on --bg, so its rest colour is
// unchanged where it was always seen.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
const hex = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const over = (ink: number[], a: number, bg: number[]) => ink.map((c, i) => Math.round(bg[i]! + a * (c - bg[i]!)));

test('ink tokens are opaque in both themes, so overlapping strokes cannot double (#321)', () => {
  const themes = [...css.matchAll(/--bg: (#[0-9a-f]{6});[^]*?--text: (#[0-9a-f]{6}); --text2: ([^;]+); --text3: ([^;]+);/gu)];
  assert.equal(themes.length, 2, 'dark and light both declare the ink triplet');
  for (const [, , text, text2, text3] of themes) {
    for (const ink of [text, text2, text3]) assert.match(ink!, /^#[0-9a-f]{6}$/u, `${ink} must be an opaque hex`);
  }
  // The values are the retired rgba inks over each theme's canvas.
  const [dark, light] = themes;
  assert.deepEqual(hex(dark![4]!), over([226, 232, 240], 0.45, hex(dark![1]!)), 'dark: rgba(226,232,240,.45) on --bg');
  assert.deepEqual(hex(light![4]!), over([26, 26, 46], 0.35, hex(light![1]!)), 'light: rgba(26,26,46,.35) on --bg');
});
