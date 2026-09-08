import test from 'node:test';
import assert from 'node:assert/strict';

(globalThis as any).$state = (value: unknown) => value;
(globalThis as any).localStorage = { getItem: () => null, setItem: () => {} };
(globalThis as any).document = { documentElement: { lang: 'en' } };
// Node >= 21 ships a read-only global navigator with a real language.

const { i18n, t, setLocale } = await import('./i18n.svelte.ts');

test('the document speaks the UI\u2019s language — SC glyphs need zh-CN (board #97)', () => {
  // index.html ships lang="en"; Han-unified codepoints rendered by a fallback
  // font pick regional variants by that tag, so Chinese UI text drew
  // Japanese-variant shapes until the locale writes the real language.
  setLocale('zh');
  assert.equal((globalThis as any).document.documentElement.lang, 'zh-CN');
  setLocale('en');
  assert.equal((globalThis as any).document.documentElement.lang, 'en');
});

test('t() falls back lang -> en -> key', () => {
  // Assert the mechanism, not specific copy (translators own the strings).
  setLocale('zh');
  assert.notEqual(t('sessions'), 'sessions'); // resolved from the zh table
  assert.equal(t('definitely-not-a-key'), 'definitely-not-a-key'); // unknown → verbatim key
  setLocale('en');
  assert.equal(t('sessions'), 'Sessions');
  assert.equal(i18n.lang, 'en');
});

test('locale tables stay key-complete in both directions', async () => {
  // Drift guard: a key added to one locale but not the other silently
  // falls back to English (or the raw key) at runtime — catch it here.
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('./i18n.svelte.ts', import.meta.url), 'utf8');
  const tables = [...source.matchAll(/^  (en|zh): \{([\s\S]*?)^  \},/gm)];
  assert.equal(tables.length, 2, 'expected exactly en + zh tables');
  const keysOf = (body: string) => new Set([...body.matchAll(/^    ([A-Za-z0-9_]+):/gm)].map(m => m[1]!));
  const en = keysOf(tables[0]![2]!);
  const zh = keysOf(tables[1]![2]!);
  const missingInZh = [...en].filter(k => !zh.has(k));
  const missingInEn = [...zh].filter(k => !en.has(k));
  assert.deepEqual(missingInZh, [], `keys missing in zh: ${missingInZh}`);
  assert.deepEqual(missingInEn, [], `keys missing in en: ${missingInEn}`);
});
