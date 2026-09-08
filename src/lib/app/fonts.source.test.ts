// Source-contract test for the UI/display font stacks (board #97, round 3).
//
// The rule that must not rot: the SC families sit BEFORE the system-font
// cascade. `-apple-system` does not "fall through" for Han — the system font
// slot resolves CJK through the OS's own language-preference cascade (page
// lang notwithstanding). With the named SC families ahead of it, Han
// deterministically hits PingFang SC / YaHei / Noto SC by NAME; latin still
// resolves at the bundled Inter/Space Grotesk in front.
//
// Round 5 found the bug the owner actually saw (#97, 2026-09-08): with both
// surfaces already drawing Han in PingFang SC (DevTools "Rendered Fonts"),
// bubbles still showed centred punctuation and Traditional 骨/感 while the
// composer was right. The one difference left was body's raw
// `font-feature-settings: 'cv05' 1, 'cv08' 1` (Inter's l/I alternates): a
// cvNN tag means whatever each font says, PingFang SC answers 5 and 8 with
// its Traditional forms, and a <textarea>'s UA `font:` shorthand resets the
// property — which is why the composer was immune. Font-specific features
// are therefore declared in @font-feature-values, scoped to the family, and
// a raw font-feature-settings is forbidden in the codebase.
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
  assert.equal(lit('SYSTEM_STACK'), cssVar('--font-mono'), 'SYSTEM_STACK === --font-mono');
});

// Round 4: the mono stack. No monospace face carries Han, so a Chinese
// character in the terminal, a code span, a path or a chip reached the
// generic `monospace` and was drawn by the OS cascade — the same Japanese
// variants, one surface over. The SC families close the mono stack (after
// the bundled symbol fillers, whose position the line-box trap fixes; before
// the generic keyword), and every data surface goes through var(--font-mono)
// so it inherits both the user's terminal font and the SC tail.
test('the mono stack ends with the SC families, after the symbol fillers AND the generic', () => {
  const stack = cssVar('--font-mono');
  const sym = stack.indexOf("'Symbols Nerd Font Mono'");
  const gen = stack.indexOf(', monospace,');
  const sc = stack.indexOf("'PingFang SC'");
  assert.ok(sym >= 0 && gen > sym, 'the generic comes after the symbol fillers');
  // The generic BEFORE the SC families is what keeps latin monospace on a
  // platform that has none of the named monos: an SC face carries latin too
  // and, placed before the generic, would take the whole terminal line.
  assert.ok(sc > gen, 'SC families come after the generic monospace, never before it');
  assert.ok(stack.trimEnd().endsWith("'WenQuanYi Micro Hei'"), 'the SC tail closes the stack');
  assert.ok(stack.indexOf('ui-monospace') === 0, 'the platform mono is still the first (line-box) font');
});

test('no component spells its own mono stack — data surfaces wear var(--font-mono)', async () => {
  const { readdir } = await import('node:fs/promises');
  const root = new URL('../', import.meta.url);
  const walk = async (dir: URL): Promise<string[]> => {
    const out: string[] = [];
    for (const e of await readdir(dir, { withFileTypes: true })) {
      const u = new URL(e.name + (e.isDirectory() ? '/' : ''), dir);
      if (e.isDirectory()) out.push(...(await walk(u)));
      else if (e.name.endsWith('.svelte')) out.push(u.pathname);
    }
    return out;
  };
  for (const f of await walk(root)) {
    const src = await readFile(f, 'utf8');
    assert.doesNotMatch(src, /ui-monospace|Menlo/u, `${f.slice(f.indexOf('/src/'))} spells a raw mono stack`);
  }
  const rules = appCss.replace(/\/\*[\s\S]*?\*\//gu, '').replace(cssVar('--font-mono'), '');
  assert.doesNotMatch(rules, /ui-monospace|Menlo/u, 'app.css rules use the var');
});

test("Inter's alternates are family-scoped; no raw font-feature-settings anywhere (#97 round 5)", async () => {
  assert.match(appCss, /@font-feature-values 'Inter Variable' \{\s*@character-variant \{ disambiguated-l: 5; disambiguated-i: 8; \}/u);
  assert.match(appCss, /font-variant-alternates: character-variant\(disambiguated-l, disambiguated-i\)/u);
  // The UA `font:` shorthand on form controls resets font-variant-alternates;
  // inputs opt back in — safe now that the feature can only reach Inter.
  assert.match(appCss, /input, textarea, select \{ font-family: inherit; font-variant-alternates: inherit; \}/u);
  const { readdir } = await import('node:fs/promises');
  const root = new URL('../../', import.meta.url);
  const walk = async (dir: URL): Promise<string[]> => {
    const out: string[] = [];
    for (const e of await readdir(dir, { withFileTypes: true })) {
      const u = new URL(e.name + (e.isDirectory() ? '/' : ''), dir);
      if (e.isDirectory()) out.push(...(await walk(u)));
      else if (/\.(svelte|css|ts)$/u.test(e.name) && !e.name.endsWith('.test.ts')) out.push(u.pathname);
    }
    return out;
  };
  for (const f of await walk(root)) {
    // Comments may name the property (this rule's own rationale does); rules may not.
    const src = (await readFile(f, 'utf8')).replace(/\/\*[\s\S]*?\*\//gu, '').replace(/^\s*\/\/.*$/gmu, '');
    assert.doesNotMatch(src, /font-feature-settings/u, `${f.slice(f.indexOf('/src/'))}: a cvNN/ssNN tag is font-specific — declare it in @font-feature-values for ONE family`);
  }
});
