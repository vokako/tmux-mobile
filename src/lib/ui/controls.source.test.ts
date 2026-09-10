import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');

test('configuration controls have two named sizes without retuning the legacy terminal metric (#155)', () => {
  assert.match(css, /--control-height:\s*32px/u);
  assert.match(css, /--control-icon-size:\s*16px/u);
  assert.match(css, /--config-width:\s*860px/u);
  assert.match(css, /@media \(any-pointer: coarse\)[\s\S]*?--control-height:\s*44px/u);
  assert.match(css, /--ui-control-height:\s*24px/u, 'legacy consumers migrate explicitly, never via a global size change');
  assert.match(css, /@supports \(-webkit-touch-callout: none\)[\s\S]*?@media \(any-pointer: coarse\)[\s\S]*?\.config-input \{ font-size: var\(--fs-input-touch\); \}/u,
    'the shared native field keeps the iOS no-auto-zoom exception');
});

test('configuration rectangles join the existing continuous-corner policy (#155)', () => {
  const policy = css.slice(css.indexOf(':is(\n  /* shared vocabulary'), css.indexOf('corner-shape: squircle;'));
  for (const selector of ['.command-button', '.config-input', '.segmented', '.segmented .slide-pill', '.check-box']) {
    assert.ok(policy.includes(selector), `${selector} uses the one global corner policy`);
  }
});

function color(body: string, name: string): number[] {
  const hex = new RegExp(`${name}:\\s*#([\\da-f]{6})`, 'iu').exec(body)?.[1];
  assert.ok(hex, `${name} has an explicit, auditable color`);
  return [0, 2, 4].map(i => parseInt(hex.slice(i, i + 2), 16));
}
function luminance(rgb: number[]): number {
  return rgb.map(c => {
    c /= 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }).reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
}
function contrast(a: number[], b: number[]): number {
  const x = luminance(a), y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

test('command and supporting-text tokens meet the accepted contrast floors in both themes (#155)', () => {
  for (const theme of ['light', 'dark']) {
    const body = new RegExp(`html\\[data-theme="${theme}"\\] \\{([\\s\\S]*?)\\n\\}`, 'u').exec(css)?.[1];
    assert.ok(body);
    const ink = color(body, '--accent-fill-ink');
    for (const name of ['--accent-fill', '--danger-fill']) {
      const fill = color(body, name);
      assert.ok(contrast(fill, ink) >= 4.5, `${theme} ${name} text`);
      const hover = fill.map(c => c * 0.96 + 255 * 0.04);
      assert.ok(contrast(hover, ink) >= 4.5, `${theme} ${name} hover text`);
    }
    assert.ok(contrast(color(body, '--text2'), color(body, '--bg2')) >= 4.5, `${theme} supporting text`);
    assert.ok(contrast(color(body, '--control-border'), color(body, '--bg')) >= 3, `${theme} control boundary`);
  }
});
