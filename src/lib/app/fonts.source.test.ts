// Source-contract test for the UI/display font stacks (board #97, round 3).
//
// The rule that must not rot: the SC families sit BEFORE the system-font
// cascade. `-apple-system` does not "fall through" for Han — the system font
// slot resolves CJK through the OS's own language-preference cascade (page
// lang notwithstanding), so with Japanese anywhere in the system preferences
// a Chinese bubble drew 骨/感 in their Japanese variants while the composer
// (IME/form path) drew them right. With the named SC families ahead of it,
// Han deterministically hits PingFang SC / YaHei / Noto SC by NAME; latin
// still resolves at the bundled Inter/Space Grotesk in front.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const appCss = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
const fontsTs = await readFile(new URL('./fonts.svelte.ts', import.meta.url), 'utf8');

const cssVar = (name: string) =>
  new RegExp(`\\n  ${name}: ([^;]+);`, 'u').exec(appCss)?.[1] ?? '';

test('Han hits a named SC family before the system cascade can pick a variant', () => {
  for (const v of ['--font-ui', '--font-display']) {
    const stack = cssVar(v);
    assert.ok(stack, `${v} exists`);
    const sc = stack.indexOf("'PingFang SC'");
    const sys = stack.indexOf('-apple-system');
    assert.ok(sc >= 0 && sys > sc, `${v}: PingFang SC (and friends) come before -apple-system`);
    assert.ok(stack.trimEnd().endsWith('sans-serif'), `${v}: the generic keyword stays last`);
  }
});

test('the fonts.svelte.ts literals mirror app.css exactly (the sync the comments demand)', () => {
  const lit = (name: string) => {
    const m = new RegExp(`const ${name} =\\s*((?:"[^"]*"\\s*\\+?\\s*)+);`, 'u').exec(fontsTs)?.[1] ?? '';
    return [...m.matchAll(/"([^"]*)"/gu)].map((x) => x[1]).join('');
  };
  assert.equal(lit('UI_STACK'), cssVar('--font-ui'), 'UI_STACK === --font-ui');
  assert.equal(lit('DISPLAY_STACK'), cssVar('--font-display'), 'DISPLAY_STACK === --font-display');
});
