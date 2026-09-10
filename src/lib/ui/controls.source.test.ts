import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');

test('compact controls separate paint from hit geometry without retuning terminal metrics (#161)', () => {
  // Owner selected #160, then asked for lower dropdowns. These replace #155's
  // painted 32/44px boxes, not its native touch-target or keyboard contracts.
  assert.match(css, /--control-height:\s*28px/u);
  assert.match(css, /--control-paint-inset:\s*2px/u);
  assert.match(css, /--control-field-inset:\s*2px/u);
  assert.match(css, /--control-icon-size:\s*16px/u);
  assert.match(css, /--config-width:\s*860px/u);
  assert.equal(/--config-width:\s*860px;\s*--config-editor-width:\s*(\d+)px;/u.exec(css)?.[1], '480',
    'the editing-column budget is shared beside the configuration canvas width (#156)');
  assert.match(css, /@media \(any-pointer: coarse\)[\s\S]*?--control-height:\s*44px/u);
  assert.match(css, /@media \(any-pointer: coarse\)[\s\S]*?--control-paint-inset:\s*6px/u);
  assert.match(css, /@media \(any-pointer: coarse\)[\s\S]*?--control-field-inset:\s*8px/u);
  assert.match(css, /--ui-control-height:\s*24px/u, 'legacy consumers migrate explicitly, never via a global size change');
  assert.match(css, /@supports \(-webkit-touch-callout: none\)[\s\S]*?@media \(any-pointer: coarse\)[\s\S]*?\.config-input \{ font-size: var\(--fs-input-touch\); \}/u,
    'the shared native field keeps the iOS no-auto-zoom exception');
});

test('configuration navigation rows have the approved pointer/touch floor (#156)', () => {
  assert.match(css, /--config-nav-height:\s*40px/u);
  assert.match(css, /@media \(any-pointer: coarse\)[^]*?--config-nav-height:\s*44px/u);
  assert.match(css, /\.config-navigation \.side-row \{ min-height: var\(--config-nav-height\); \}/u);
});

test('compact control shapes have one explicit round policy, not flattened capsule caps (#161)', () => {
  assert.match(css, /--control-radius:\s*12px/u);
  assert.match(css, /--control-menu-radius:\s*16px/u);
  assert.match(css, /--control-dialog-radius:\s*22px/u);
  assert.match(css, /:where\(\s*\/\* shared vocabulary/u,
    'the legacy default cannot borrow a three-class selector weight and defeat a round variant');
  const round = css.match(/:is\(([^)]*\.control-field[^)]*)\) \{\s*corner-shape: round;/u)?.[1] ?? '';
  for (const selector of ['.command-button', '.config-input', '.segmented', '.slide-pill.control', '.sel-menu', '.dlg.confirm']) {
    assert.ok(round.includes(selector), `${selector}: same approved round treatment`);
  }
});

test('native fields own their full hit box while only the padding box is painted (#161)', () => {
  assert.match(css, /input\.config-input, \.control-field \{/u,
    'shortcut/address command variants keep their existing explicit state-border contract');
  const field = css.match(/\.control-field \{([^}]+)\}/u)?.[1] ?? '';
  assert.match(field, /height:\s*var\(--control-height\)/u);
  assert.match(field, /border:\s*solid transparent/u);
  assert.match(field, /border-width:\s*var\(--control-field-inset\) 0/u);
  assert.match(field, /background-clip:\s*padding-box/u);
  assert.match(field, /border-radius:\s*var\(--control-radius\) \/ calc\(var\(--control-radius\) \+ var\(--control-field-inset\)\)/u,
    'the inner paint keeps the full radius when the transparent vertical border grows');
  assert.doesNotMatch(field, /pointer-events|transform|clip-path|overflow/u);
});

test('configuration headers wrap whole action groups instead of squeezing or eliding titles (#156)', () => {
  const head = /\.config-head-inner \{([^}]+)\}/u.exec(css)?.[1] ?? '';
  const title = /\.config-head-inner h1 \{([^}]+)\}/u.exec(css)?.[1] ?? '';
  const actions = /\.config-actions \{([^}]+)\}/u.exec(css)?.[1] ?? '';
  assert.match(head, /flex-wrap:\s*wrap;/u);
  for (const declaration of ['flex: 1 1 160px;', 'white-space: normal;', 'overflow-wrap: anywhere;',
    'overflow: visible;', 'text-overflow: clip;']) {
    assert.ok(title.includes(declaration), `configuration title keeps ${declaration}`);
  }
  assert.match(actions, /margin-left:\s*auto;/u);
  assert.match(actions, /display:\s*flex;/u);
  assert.match(actions, /flex:\s*none;/u);
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

test('command and supporting-text tokens meet the accepted contrast floors in both themes (#155/#161)', () => {
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
    // This still serves checkbox/switch boundaries. It no longer establishes
    // contrast for compact fields, which deliberately have no drawn outline.
    assert.ok(contrast(color(body, '--control-border'), color(body, '--bg')) >= 3, `${theme} retained control boundary`);
    for (const surface of ['--control-surface', '--control-hover', '--control-selected', '--control-field-bg']) {
      assert.ok(contrast(color(body, '--text'), color(body, surface)) >= 4.5, `${theme} text on ${surface}`);
    }
    assert.ok(contrast(color(body, '--text2'), color(body, '--control-field-bg')) >= 4.5, `${theme} field placeholder`);
    assert.ok(contrast(color(body, '--accent-ink'), color(body, '--control-field-bg')) >= 3, `${theme} field focus ring`);
  }
});

test('tokenized command overlays preserve the original sRGB composites and contrast (#156)', () => {
  const root = /html \{([\s\S]*?)\n\}/u.exec(css)?.[1];
  assert.ok(root);
  const light = color(root, '--control-overlay-light');
  const dark = color(root, '--control-overlay-dark');
  assert.deepEqual(light, [255, 255, 255]);
  assert.deepEqual(dark, [0, 0, 0]);
  for (const theme of ['light', 'dark']) {
    const body = new RegExp(`html\\[data-theme="${theme}"\\] \\{([\\s\\S]*?)\\n\\}`, 'u').exec(css)?.[1];
    assert.ok(body);
    assert.doesNotMatch(body, /--control-overlay-(?:light|dark):/u, 'overlay endpoints do not vary by theme');
    const ink = color(body, '--accent-fill-ink');
    for (const name of ['--accent-fill', '--danger-fill']) {
      const fill = color(body, name);
      for (const [endpoint, alpha, original] of [[light, 0.04, 255], [dark, 0.06, 0]] as const) {
        const mixed = fill.map((c, i) => c * (1 - alpha) + endpoint[i]! * alpha);
        const before = fill.map(c => c * (1 - alpha) + original * alpha);
        assert.deepEqual(mixed, before, `${theme} ${name} alpha ${alpha}: unchanged composite`);
        assert.equal(contrast(mixed, ink), contrast(before, ink), 'unchanged contrast');
        assert.ok(contrast(mixed, ink) >= 4.5, `${theme} ${name} alpha ${alpha}: readable text`);
      }
    }
  }
});
