import test from 'node:test';
import assert from 'node:assert/strict';
import { adaptAnsiColors, contrastRatio, type AnsiTheme } from './ansi-colors.ts';

const darkTheme: AnsiTheme = {
  background: '#0a0a0f', foreground: '#c9d1d9', black: '#0a0a0f', red: '#ff5050', green: '#4ade80', yellow: '#fbbf24', blue: '#00d4ff', magenta: '#c084fc', cyan: '#22d3ee', white: '#c9d1d9', brightBlack: '#484848', brightRed: '#ff6b6b', brightGreen: '#6ee7a0', brightYellow: '#fcd34d', brightBlue: '#38bdf8', brightMagenta: '#d8b4fe', brightCyan: '#67e8f9', brightWhite: '#f1f5f9',
};
const lightTheme: AnsiTheme = {
  background: '#f5f5f7', foreground: '#1a1a2e', black: '#f5f5f7', red: '#dc2626', green: '#16a34a', yellow: '#ca8a04', blue: '#0088cc', magenta: '#9333ea', cyan: '#0891b2', white: '#1a1a2e', brightBlack: '#9ca3af', brightRed: '#ef4444', brightGreen: '#22c55e', brightYellow: '#eab308', brightBlue: '#2563eb', brightMagenta: '#a855f7', brightCyan: '#06b6d4', brightWhite: '#0f0f1a',
};

type Rgb = [number, number, number];
function lastPair(output: string): [Rgb, Rgb] {
  const matches = [...output.matchAll(/\x1b\[(38|48);2;(\d+);(\d+);(\d+)m/g)];
  const colors = new Map(matches.map(match => [match[1], match.slice(2).map(Number)]));
  return [colors.get('38') as Rgb, colors.get('48') as Rgb];
}

for (const [name, theme] of [['dark', darkTheme], ['light', lightTheme]] as const) {
  test(`${name}: truecolor foreground and background retain readable contrast`, () => {
    const output = adaptAnsiColors('\x1b[38;2;255;255;0;48;2;128;0;128mtext', theme);
    const [foreground, background] = lastPair(output);
    assert.ok(contrastRatio(foreground, background) >= 4.5);
  });

  test(`${name}: basic and indexed ANSI colors are adapted as a pair`, () => {
    const output = adaptAnsiColors('\x1b[33;48;5;238mtext', theme);
    const [foreground, background] = lastPair(output);
    assert.ok(contrastRatio(foreground, background) >= 4.5);
  });

  test(`${name}: reverse video remains readable`, () => {
    const output = adaptAnsiColors('\x1b[38;2;220;220;220;48;2;235;235;235;7mtext', theme);
    const [foreground, background] = lastPair(output);
    assert.ok(contrastRatio(foreground, background) >= 4.5);
  });
}

test('leaving default reverse video clears injected color overrides', () => {
  const output = adaptAnsiColors('\x1b[7mreverse\x1b[27mnormal', darkTheme);
  assert.match(output, /\x1b\[27m\x1b\[39;49mnormal$/);
});

test('reset stops emitting explicit overrides', () => {
  const output = adaptAnsiColors('\x1b[31mred\x1b[0mplain', darkTheme);
  assert.match(output, /\x1b\[0mplain$/);
});

test('separate foreground and background sequences are evaluated as one pair', () => {
  const output = adaptAnsiColors('\x1b[38;2;255;255;0mfg\x1b[48;2;128;0;128mbg', lightTheme);
  const [foreground, background] = lastPair(output);
  assert.ok(contrastRatio(foreground, background) >= 4.5);
});

test('SGR color state continues across newlines', () => {
  const output = adaptAnsiColors('\x1b[31mfirst\n\x1b[48;5;238msecond', darkTheme);
  const [foreground, background] = lastPair(output);
  assert.ok(contrastRatio(foreground, background) >= 4.5);
});

test('39 and 49 restore theme defaults independently', () => {
  const output = adaptAnsiColors('\x1b[31;44mcolored\x1b[39mdefault fg\x1b[49mdefaults', darkTheme);
  assert.match(output, /\x1b\[49m\x1b\[39;49mdefaults$/);
  const [foreground, background] = lastPair(output.slice(0, output.indexOf('defaults')));
  assert.ok(contrastRatio(foreground, background) >= 4.5);
});

test('light theme keeps the default background unchanged for foreground-only text', () => {
  const output = adaptAnsiColors('\x1b[38;2;255;255;0mtext', lightTheme);
  const [, background] = lastPair(output);
  assert.deepEqual(background, [245, 245, 247]);
});

for (const [name, theme] of [['dark', darkTheme], ['light', lightTheme]] as const) {
  test(`${name}: default reverse video remains a visible block`, () => {
    const [foreground, background] = lastPair(adaptAnsiColors('\x1b[7mtext', theme));
    const terminalBackground = theme.background.match(/[a-f\d]{2}/gi)!.map((value: string) => Number.parseInt(value, 16));
    assert.notDeepEqual(background, terminalBackground);
    assert.ok(contrastRatio(foreground, background) >= 4.5);
  });
}

for (const [name, theme] of [['dark', darkTheme], ['light', lightTheme]] as const) {
  test(`${name}: representative color matrix always meets text contrast`, () => {
    const samples = [
      [0, 0, 0], [255, 255, 255], [238, 238, 238], [26, 26, 46],
      [255, 0, 0], [0, 180, 0], [255, 255, 0], [0, 128, 255],
      [128, 0, 128], [0, 180, 180], [90, 90, 90], [180, 120, 40],
    ];
    for (const foreground of samples) {
      for (const background of samples) {
        const input = `\x1b[38;2;${foreground.join(';')};48;2;${background.join(';')}mtext`;
        const [mappedForeground, mappedBackground] = lastPair(adaptAnsiColors(input, theme));
        assert.ok(
          contrastRatio(mappedForeground, mappedBackground) >= 4.5,
          `${foreground} on ${background} mapped to ${mappedForeground} on ${mappedBackground}`,
        );
      }
    }
  });
}
