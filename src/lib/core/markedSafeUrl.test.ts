import test from 'node:test';
import assert from 'node:assert/strict';
import { marked } from 'marked';
import './markedSafeUrl.ts'; // side-effect: registers the extension

test('bare URLs stop at CJK / fullwidth characters', () => {
  const html = marked.parse('见 https://host/admin（密码：xyz）') as string;
  assert.match(html, /<a href="https:\/\/host\/admin">https:\/\/host\/admin<\/a>/);
  assert.doesNotMatch(html, /href="[^"]*密码/);
});

test('trailing sentence punctuation is dropped from the link', () => {
  const html = marked.parse('see https://example.com/path.') as string;
  assert.match(html, /<a href="https:\/\/example\.com\/path">/);
});

test('ordinary ASCII URLs are linked whole', () => {
  const html = marked.parse('https://example.com/a?b=1&c=2#frag') as string;
  assert.match(html, /<a href="https:\/\/example\.com\/a\?b=1&(amp;)?c=2#frag">/);
});
