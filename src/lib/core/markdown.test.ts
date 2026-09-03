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

test('a quote in a bare URL cannot break out of the href attribute', () => {
  // Reproduced 2026-09-03: `&`/`<` escaping makes TEXT inert, but an href is an
  // ATTRIBUTE, and a `"` there ends it — the rest of the URL became onclick=.
  const html = renderMarkdown('see https://a.b/x"onclick="alert(1) now');
  assert.ok(!/<a[^>]*\bonclick=/u.test(html), `attribute injection: ${html}`);
  // Still a link, with the quote percent-encoded the way marked's own autolink does.
  assert.match(html, /<a href="https:\/\/a\.b\/x%22onclick=%22alert\(1\)">/u);
});

test('links and images only carry http(s), mailto or relative targets', () => {
  // marked v17 does not sanitize schemes: a javascript: link is a click away
  // from script in the app origin (the token lives in localStorage).
  const bads = [
    'javascript:alert(1)', 'JavaScript:alert(1)', 'vbscript:msgbox',
    'data:text/html,<script>alert(1)</script>', ' javascript:alert(1)',
    'java\tscript:alert(1)',
  ];
  for (const bad of bads) {
    const html = renderMarkdown(`[click](${bad})`);
    assert.ok(!/<a\b/u.test(html), `no anchor for ${JSON.stringify(bad)}: ${html}`);
    assert.ok(html.includes('click'), `the text survives for ${JSON.stringify(bad)}: ${html}`);
  }
  // Entity-encoded colons: the chat path's `&` escape already defuses them
  // (the browser decodes `&amp;#58;` ONCE, to the literal `&#58;`), and the
  // scheme guard reads them the way a browser would for any renderer that does
  // not pre-escape. Either way, what the browser sees is never `javascript:`.
  for (const bad of ['javascript&#58;alert(1)', 'javascript&#x3a;alert(1)', 'javascript&colon;alert(1)']) {
    const html = renderMarkdown(`[click](${bad})`);
    const href = /href="([^"]*)"/u.exec(html)?.[1] ?? '';
    const seen = href.replace(/&amp;/g, '&');
    assert.ok(!/^[\s\u0000-\u0020]*javascript:/iu.test(seen), `browser-visible href ${JSON.stringify(seen)} for ${JSON.stringify(bad)}`);
  }
  // An image is a reference (rule 13): bytes and scripts are not references.
  const img = renderMarkdown('![shot](data:image/png;base64,AAAA) and ![x](javascript:alert(1))');
  assert.ok(!img.includes('<img'), `no image for a non-reference: ${img}`);
  assert.ok(img.includes('shot'), 'the alt text survives');
  // Ordinary targets are untouched, including a query string with `&`.
  assert.match(renderMarkdown('[a](https://x.y/z?p=1&q=2)'), /<a href="https:\/\/x\.y\/z\?p=1&amp;q=2">a<\/a>/u);
  assert.match(renderMarkdown('[m](mailto:a@b.c)'), /<a href="mailto:a@b\.c">/u);
  assert.match(renderMarkdown('[rel](docs/x.md)'), /<a href="docs\/x\.md">/u);
  assert.match(renderMarkdown('![pic](https://x.y/p.png)'), /<img src="https:\/\/x\.y\/p\.png" alt="pic"/u);
});

test('inline code inside a dollar span is never swallowed as math', () => {
  // Reproduced 2026-09-03: code is holed out to \x00CODE0\x00 BEFORE math runs,
  // and the `$…$` body class matched across the placeholder — KaTeX got the
  // placeholder, the code never came back.
  const html = renderMarkdown('cost $5 and `code` is 3$ ok');
  assert.ok(!html.includes('CODE0'), `placeholder leaked: ${html}`);
  assert.match(html, /<code>code<\/code>/u, 'the code renders');
  assert.ok(!html.includes('katex'), 'money around code is prose');
  // The display and \( \) families hold the same line.
  assert.ok(!renderMarkdown('$$ a `b` c $$').includes('CODE0'));
  assert.ok(!renderMarkdown('\\( a `b` c \\)').includes('CODE0'));
  assert.ok(!renderMarkdown('\\[ a `b` c \\]').includes('CODE0'));
});
