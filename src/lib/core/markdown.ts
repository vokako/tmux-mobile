// Shared safe markdown rendering for chat bodies and skill previews.
//
// Agent output is UNTRUSTED, and marked (v17) no longer sanitizes, so HTML is
// escaped FIRST — markdown syntax (**, #, ```fences```, tables, links) still
// renders, but raw <script>/<img onerror> becomes inert literal text.
// Memoized because chat re-renders on every poll/push. Extracted from
// Team.svelte when the Hub (and the skill preview) needed the same contract.
// Importing this module also registers the CJK-aware autolinker.
import { marked } from 'marked';
import './markedSafeUrl.ts';

const cache = new Map<string, string>();

export function renderMarkdown(body: string | null | undefined): string {
  const src = body || '';
  const hit = cache.get(src);
  if (hit !== undefined) return hit;
  const escaped = src.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  let html: string;
  try { html = marked.parse(escaped, { gfm: true, breaks: true }) as string; }
  catch { html = escaped; }
  if (cache.size > 500) cache.clear(); // bound the cache
  cache.set(src, html);
  return html;
}
