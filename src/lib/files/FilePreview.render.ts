import test from 'node:test';
import assert from 'node:assert/strict';
import type { PreviewFile } from './file-preview.ts';
import { renderHarness, RENDER_TIMEOUT_MS } from '../test/ssr.ts';

// One warm server for the whole render tier (board #178): the environment
// and the svelte runtime are built at import, outside this suite's timer.
const h = await renderHarness();
const noop = () => {};

test('the extracted preview renders every existing branch and preserves the code cap', { timeout: RENDER_TIMEOUT_MS }, async () => {
  const Preview = (await h.load('/src/lib/files/FilePreview.svelte')).default;
  const { render } = h;
  const renderFile = (file: Partial<PreviewFile>, showAllLines = false): string => render(Preview, { props: {
    currentFile: { path: '/test/file', name: 'file', ...file },
    fontSize: 14, wrapLines: false, hljs: null, showAllLines,
    previewLinkClick: noop, attachHtmlPreviewLinks: noop,
    previewBodyEl: null, previewEl: null, htmlPreviewEl: null, pdfContainer: null,
  } }).body;

  const markdown = renderFile({ stat: { mime_hint: 'text/markdown' }, content: '# Title\n\n[next](next.md)\n\n<script>inert</script>' });
  assert.match(markdown, /class="preview-body/u);
  assert.match(markdown, /<h1>Title<\/h1>/u);
  assert.match(markdown, /href="next\.md"/u);
  assert.ok(!markdown.includes('<script>'), 'raw Markdown HTML remains inert');
  assert.ok(!markdown.includes('preview-header'), 'no duplicate header in the extracted body');

  const csv = renderFile({ stat: { mime_hint: 'text/csv' }, content: 'a,b\n1,2' });
  assert.match(csv, /class="csv-render/u);
  assert.match(csv, /<td>1<\/td><td>2<\/td>/u);
  assert.ok(!csv.includes('<iframe'));

  const html = renderFile({ stat: { mime_hint: 'text/html' }, content: '<h1>HTML</h1>' });
  assert.ok(html.includes('<iframe '));
  assert.ok(html.includes('sandbox="allow-same-origin"'));
  assert.ok(!html.includes('allow-scripts'));
  const pdf = renderFile({ stat: { mime_hint: 'application/pdf' }, pdfData: 'bytes' });
  assert.match(pdf, /class="pdf-container/u);
  assert.ok(!pdf.includes('<iframe'));
  assert.match(renderFile({ stat: { mime_hint: 'image/png' }, dataUrl: 'data:image/png;base64,eA==' }), /<img[^>]*src="data:image\/png;base64,eA=="/u);
  assert.match(renderFile({ stat: { mime_hint: 'application/octet-stream' }, convertedHtml: '<h2>Slide</h2>' }), /<h2>Slide<\/h2>/u);

  const content = Array.from({ length: 3002 }, (_, i) => `line ${i}`).join('\n');
  const capped = renderFile({ stat: { mime_hint: 'text/plain' }, content });
  assert.equal([...capped.matchAll(/class="cl-row\b/gu)].length, 3000);
  assert.match(capped, /class="cl-more/u);
  const expanded = renderFile({ stat: { mime_hint: 'text/plain' }, content }, true);
  assert.equal([...expanded.matchAll(/class="cl-row\b/gu)].length, 3002);
  assert.ok(!expanded.includes('class="cl-more'));
});
