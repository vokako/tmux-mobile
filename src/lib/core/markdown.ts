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

// Strikethrough requires DOUBLE tildes here, unlike GFM, which also accepts
// `~x~`. A single tilde is a range separator in everyday Chinese text — the
// weather reply `26~32℃，东风微风 … 26~32℃` opened a <del> at the first tilde
// and closed it at the next one, striking out several lines of a real answer
// (owner report, 2026-08-16). Being unable to strike text out is a small loss;
// striking out text nobody asked to strike is a wrong rendering of the content.
// Kept to one line as well: a <del> that spans a `breaks: true` newline is how
// the damage spread.
//
// A tilde run that is not a valid `~~…~~` is CONSUMED as plain text, because an
// overridden tokenizer that returns false falls back to marked's own — so
// refusing the match is not enough to stop it, the characters have to be taken
// out of play.
marked.use({
  tokenizer: {
    // The signature is marked's own; the return is a Del token, a text token
    // that swallows the tildes, or false when this is not our business.
    del(this: any, src: string): any {
      if (!src.startsWith('~')) return false;
      const m = /^~~(?=\S)([^\n]*?\S)~~/.exec(src);
      if (m) {
        return { type: 'del', raw: m[0], text: m[1]!, tokens: this.lexer.inlineTokens(m[1]!) };
      }
      const run = /^~+/.exec(src)![0];
      return { type: 'text', raw: run, text: run };
    },
  },
});

const cache = new Map<string, string>();

export function renderMarkdown(body: string | null | undefined): string {
  const src = body || '';
  const hit = cache.get(src);
  if (hit !== undefined) return hit;
  // Escape `&` and `<` — and deliberately NOT `>`. Those two are all that raw
  // HTML needs to be inert: a tag cannot start without `<`, so `&lt;script>`
  // renders as text either way. Escaping `>` as well (which this did) also ate
  // every markdown construct that uses it: `> quote` arrived at the parser as
  // `&gt; quote`, so blockquotes NEVER rendered — reported as "the content is not
  // rendered as markdown", and it was right for that whole class of message.
  const escaped = src.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  let html: string;
  try { html = marked.parse(escaped, { gfm: true, breaks: true }) as string; }
  catch { html = escaped; }
  if (cache.size > 500) cache.clear(); // bound the cache
  cache.set(src, html);
  return html;
}
