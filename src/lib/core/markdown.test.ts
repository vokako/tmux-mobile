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

test('markdown/md fences are rendered in rendered view, not shown as source', () => {
  // This is the exact shape used by proj:test seq=52: prose, a quoted tasks.md
  // file, then prose. Raw view bypasses this function and still shows the fence.
  const body = '@human tasks.md：\n\n```markdown\n# Plan\n\n- [x] done\n- [ ] next\n```\n\n完成。';
  const html = renderMarkdown(body);
  assert.match(html, /<h1>Plan<\/h1>/, `heading should render, got: ${html}`);
  assert.match(html, /<input[^>]*checked/, 'task item should render as checked');
  assert.ok(!html.includes('language-markdown'), 'markdown wrapper is transparent in rendered view');
  assert.ok(!html.includes('<pre>'), 'the quoted md is not presented as source');
});

test('only complete markdown fences unwrap; code and malformed fences stay code', () => {
  assert.match(renderMarkdown('```rust\n# not a heading\n```'), /<pre><code class="language-rust">/);
  assert.match(renderMarkdown('```markdown\n# unclosed'), /<pre><code class="language-markdown">/);
  // Four ticks can wrap a document that itself contains a normal code fence.
  const nested = renderMarkdown('````md\n# Doc\n```js\nconst x = 1\n```\n````');
  assert.match(nested, /<h1>Doc<\/h1>/);
  assert.match(nested, /<code class="language-js">/);
});

test('LaTeX renders in every dialect agents emit; money and code stay prose', () => {
  // The four delimiter families (owner, 2026-08-26: "latex公式要正确渲染").
  assert.ok(renderMarkdown('inline $v = 2t$ here').includes('class="katex"'), '$…$');
  assert.ok(renderMarkdown('$$\\int_0^1 x\\,dx$$').includes('katex-display'), '$$…$$');
  assert.ok(renderMarkdown('so \\(E = mc^2\\) holds').includes('class="katex"'), '\\(…\\)');
  assert.ok(renderMarkdown('\\[\\sum_{i=0}^n i\\]').includes('katex-display'), '\\[…\\]');
  // TeX is read RAW — `<` survives into the formula, not as `&lt;`.
  assert.ok(renderMarkdown('$a < b$').includes('class="katex"'), 'relational < inside math');
  // Pandoc guards: everyday dollars are not formulas.
  assert.ok(!renderMarkdown('costs $5, earns $10 total').includes('katex'), 'money');
  assert.ok(!renderMarkdown('US$5 vs A$7 deal$').includes('katex'), 'currency prefixes');
  // Code is holed out before math extraction.
  assert.ok(!renderMarkdown('```sh\necho $PATH $HOME\n```').includes('katex'), 'fence');
  assert.ok(!renderMarkdown('run `$x$` now').includes('katex'), 'inline code');
  // Placeholders never leak into the output.
  assert.ok(!renderMarkdown('mix `code` and $v=1$ and $$w=2$$').includes('MATH'), 'holes restored');
});
