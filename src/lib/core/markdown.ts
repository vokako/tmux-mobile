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

/** In the chat, rendered/raw is a view switch. Agents often wrap an entire
 * answer (or a quoted .md file inside an answer) in ```markdown even though the
 * user asked to SEE the formatted result. In rendered view that language fence
 * is therefore transparent: remove only its wrapper and let its contents go
 * through the normal Markdown parser. Raw view still reads `m.body` untouched.
 *
 * Fence length matters: a four-backtick markdown wrapper may legitimately
 * contain triple-backtick code blocks, so only an equally long (or longer)
 * closing run ends it. Other language fences remain ordinary code blocks. */
function unwrapMarkdownFences(src: string): string {
  const lines = src.split('\n');
  const out: string[] = [];
  let fence: { char: '`' | '~'; len: number } | null = null;
  for (const line of lines) {
    if (!fence) {
      const open = /^\s*(`{3,}|~{3,})\s*(?:markdown|md)\s*$/i.exec(line);
      if (!open) { out.push(line); continue; }
      fence = { char: open[1]![0] as '`' | '~', len: open[1]!.length };
      continue;
    }
    const close = new RegExp(`^\\s*\\${fence.char}{${fence.len},}\\s*$`);
    if (close.test(line)) { fence = null; continue; }
    out.push(line);
  }
  // An unclosed language fence is malformed source. Do not silently reinterpret
  // the rest of the message; marked's normal code-fence behavior is safer.
  return fence ? src : out.join('\n');
}

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
  const rendered = unwrapMarkdownFences(src);
  const escaped = rendered.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  let html: string;
  try { html = marked.parse(escaped, { gfm: true, breaks: true }) as string; }
  catch { html = escaped; }
  if (cache.size > 500) cache.clear(); // bound the cache
  cache.set(src, html);
  return html;
}
