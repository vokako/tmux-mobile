import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
const context = await readFile(new URL('./ContextMenu.svelte', import.meta.url), 'utf8');
const select = await readFile(new URL('./Select.svelte', import.meta.url), 'utf8');
const style = (source: string) => source.match(/<style>([\s\S]*?)<\/style>/u)?.[1] ?? '';

test('menu chrome and row metrics have one shared owner, with compact pointer and coarse targets (#165)', () => {
  assert.match(css, /--menu-row-height: 28px/u);
  assert.match(css, /--menu-row-height: 44px/u);
  assert.match(css, /\.menu-surface \{[^}]*border-radius: var\(--control-menu-radius\)/u);
  assert.match(css, /\.menu-item \{[^}]*min-height: var\(--menu-row-height\)/u);
  assert.match(css, /\.menu-item \{[^}]*border-radius: var\(--control-radius\)/u);
  assert.match(css, /\.menu-item \{[^}]*font: 400 var\(--menu-font-size\)\/var\(--control-line-height\) var\(--font-ui\)/u);
  for (const [source, root] of [[context, 'ctx'], [select, 'sel-menu']]) {
    assert.match(source!, new RegExp(`class="${root} menu-surface menu-list pop-layer"`, 'u'));
    assert.match(source!, /class="(?:sel-opt )?menu-item"/u);
    const panel = new RegExp(`\\.${root} \\{([^}]+)\\}`, 'u').exec(style(source!))?.[1] ?? '';
    assert.doesNotMatch(panel, /background:|border:|border-radius:|box-shadow:|padding:|gap:/u);
  }
  assert.doesNotMatch(style(context), /\.ctx button\s*\{/u);
  assert.doesNotMatch(style(select), /\.sel-opt\s*\{/u);
  assert.match(style(context), /width: max-content/u,
    'intrinsic width must not depend on the left position computed from that same measurement');
});

test('picker and tooltip share only frame paint, not menu layout or interaction roles (#165)', async () => {
  for (const [path, root, role] of [
    ['../sessions/PanePicker.svelte', 'picker', 'dialog'],
    ['./HoverCard.svelte', 'hover-card', 'tooltip'],
  ]) {
    const source = await readFile(new URL(path!, import.meta.url), 'utf8');
    assert.match(source, new RegExp(`class="${root} menu-surface pop-layer"\\s+class:ready=\\{[^}]+\\}\\s+role="${role}"`, 'u'));
    assert.doesNotMatch(source, /menu-item|menu-list/u);
    const panel = new RegExp(`\\.${root} \\{([^}]+)\\}`, 'u').exec(style(source))?.[1] ?? '';
    assert.doesNotMatch(panel, /background:|border:|border-radius:|box-shadow:/u);
    if (role === 'tooltip') assert.match(source, /\.hover-card\.ready \{ pointer-events: none;/u);
  }
});

test('menu copy wraps and icon columns align without creating new renderers or selection semantics (#165)', () => {
  assert.match(css, /\.menu-label \{[^}]*overflow-wrap: anywhere/u);
  assert.match(context, /const hasIcons = \$derived\(items\.some\(\(item\) => !!item\.icon\)\)/u);
  // An agent option's .ava tile takes the same column (board #293).
  assert.match(select, /const hasIcons = \$derived\(shown\.some\(\(option\) => !!option\.icon \|\| !!option\.ink\)\)/u);
  for (const source of [context, select]) {
    assert.match(source, /\{#if hasIcons\}<span class="menu-icon" aria-hidden="true">/u);
    assert.match(source, /class="[^"]*menu-check[^"]*"/u);
  }
  assert.match(context, /role=\{it\.checked === undefined \? 'menuitem' : 'menuitemcheckbox'\}/u);
  assert.match(select, /role="option" aria-selected=\{o\.value === value\}/u);
});

test('warning and destructive menu TEXT keeps 4.5:1 on normal and hover surfaces (#165)', () => {
  assert.match(css, /--menu-warn-ink: color-mix\(in srgb, var\(--status-warn\) 60%, var\(--text\)\)/u);
  assert.match(css, /\.menu-item\.warn \{ color: var\(--menu-warn-ink\)/u);
  assert.match(css, /\.menu-item\.danger \{ color: var\(--danger-ink\)/u);
  const color = (body: string, name: string): number[] => {
    const value = new RegExp(`${name}:\\s*([^;]+)`, 'u').exec(body)?.[1] ?? '';
    if (value.startsWith('#')) return [1, 3, 5].map(i => parseInt(value.slice(i, i + 2), 16));
    assert.match(value, /^rgba\(/u);
    return value.slice(5, -1).split(',').map(Number);
  };
  const mix = (a: number[], b: number[], alpha: number) => a.slice(0, 3).map((n, i) => n * alpha + b[i]! * (1 - alpha));
  const lum = (rgb: number[]) => rgb.map(n => {
    n /= 255;
    return n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4;
  }).reduce((sum, n, i) => sum + n * [0.2126, 0.7152, 0.0722][i]!, 0);
  const ratio = (a: number[], b: number[]) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
  for (const theme of ['light', 'dark']) {
    const body = new RegExp(`html\\[data-theme="${theme}"\\] \\{([\\s\\S]*?)\\n\\}`, 'u').exec(css)?.[1];
    assert.ok(body);
    const bg = color(body, '--bg'), warn = color(body, '--status-warn');
    const warnInk = mix(warn, color(body, '--text'), 0.6);
    const dangerWash = color(body, '--danger-bg');
    for (const surface of [bg, mix(warn, bg, 0.14)]) {
      assert.ok(ratio(warnInk, surface) >= 4.5, `${theme} warning text`);
    }
    for (const surface of [bg, mix(dangerWash, bg, dangerWash[3]!)]) {
      assert.ok(ratio(color(body, '--danger-ink'), surface) >= 4.5, `${theme} danger text`);
    }
    if (theme === 'light') assert.ok(ratio(warn, bg) < 4.5, 'negative: dot amber is not readable menu text');
  }
});
