import test from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown } from './markdown.ts';

test('a single tilde is text, not strikethrough', () => {
  // The bug: a weather reply full of ranges (`26~32℃`) came back with several
  // lines struck through, because GFM opens <del> at one tilde and closes it at
  // the next — across a `breaks: true` newline.
  const body = '• 8月17日：阴，26~32℃，东风微风\n• 8月18日：小雨，26~32℃，东南风微风';
  const html = renderMarkdown(body);
  assert.ok(!html.includes('<del>'), `no strikethrough, got: ${html}`);
  assert.ok(html.includes('26~32℃'), 'the range survives as written');
});

test('double tildes still strike text out, on one line only', () => {
  assert.match(renderMarkdown('~~gone~~ kept'), /<del>gone<\/del>/);
  // Inline markdown inside still renders.
  assert.match(renderMarkdown('~~**bold gone**~~'), /<del><strong>bold gone<\/strong><\/del>/);
  // Spanning a newline would be how the damage spread; it must not.
  assert.ok(!renderMarkdown('~~open\nclose~~').includes('<del>'));
});

test('blockquotes render — escaping `>` up front used to kill them', () => {
  const html = renderMarkdown('> 引用一句\n> 第二行');
  assert.match(html, /<blockquote>/, `got: ${html}`);
  assert.ok(!html.includes('&gt; 引用'), 'the marker was consumed, not printed');
  // Nested markdown inside a quote still works.
  assert.match(renderMarkdown('> **重点**'), /<blockquote>[\s\S]*<strong>重点<\/strong>/);
});

test('html in agent output stays inert, markdown still renders', () => {
  const html = renderMarkdown('<img src=x onerror=alert(1)> **bold**');
  assert.ok(!html.includes('<img'), 'raw html is escaped');
  assert.match(html, /<strong>bold<\/strong>/);
  // `<` is the only character a tag can start with, so leaving `>` alone is safe.
  const script = renderMarkdown('<script>alert(1)</script>');
  assert.ok(!script.includes('<script'), `got: ${script}`);
  assert.match(script, /&lt;script&gt;|&lt;script>/, 'shown as text');
  // A closing angle bracket in prose is just text.
  assert.match(renderMarkdown('a > b'), /a &gt; b|a > b/);
});
