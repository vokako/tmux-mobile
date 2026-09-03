// A bare-URL autolinker for `marked` that stops at CJK / fullwidth characters.
//
// GFM's built-in autolink extends a bare URL until whitespace, so a URL pasted
// immediately before non-ASCII text — e.g. `https://host/admin（密码：xyz）` —
// swallows the trailing `（密码：xyz）` into the href. This inline extension
// matches the URL up to the first CJK or fullwidth char (and drops trailing
// sentence punctuation), leaving the rest as plain text.
//
// It also carries the two attribute-context guards every renderer needs
// (2026-09-03 review): a `"` inside a URL is percent-encoded before it lands in
// `href="\u2026"`, and a link or image whose target is not http(s), mailto or
// relative is rendered as its TEXT. The `&`/`<` escape the chat path applies
// makes text inert; an attribute has its own delimiter, and marked v17 does not
// sanitize schemes \u2014 `[x](javascript:\u2026)` used to reach the DOM as written.
//
// `marked` is a singleton, so importing this module once registers it for every
// renderer (Files preview, Team chat, Chat view).
import { marked } from 'marked';

// Fullwidth forms, CJK punctuation, CJK/Kana/Hangul ideographs.
const STOP = /[\u2E80-\u9FFF\u3000-\u303F\u3040-\u30FF\uAC00-\uD7AF\uFF00-\uFFEF]/;

/** marked's own `cleanUrl`: percent-encode what encodeURI encodes (`"`, `'`,
 * `<`, `>`, space\u2026) and undo double-encoding of `%`. `null` when the URL is not
 * even encodable (a lone surrogate). */
function encodeHref(href: string): string | null {
  try { return encodeURI(href).replace(/%25/g, '%'); } catch { return null; }
}

/** Is this target something a reader can follow without running code?
 * Allowed: no scheme (relative, `#anchor`), `http:`, `https:`, `mailto:`.
 * The scheme is read the way a BROWSER reads an attribute \u2014 numeric and
 * `&colon;` entities decoded, ASCII control characters and spaces stripped
 * (`java\tscript:` is `javascript:` to WebKit) \u2014 so an encoding trick cannot
 * smuggle `javascript:` past a naive prefix test. */
export function safeLinkTarget(href: string | null | undefined, kind: 'link' | 'image' = 'link'): boolean {
  if (href == null) return false;
  const decoded = href
    .replace(/&#(\d+);?/g, (_m, d) => String.fromCodePoint(Number(d) & 0x1fffff))
    .replace(/&#[xX]([0-9a-fA-F]+);?/g, (_m, h) => String.fromCodePoint(parseInt(h, 16) & 0x1fffff))
    .replace(/&colon;/gi, ':')
    .replace(/[\u0000-\u0020\u007f]/g, '');
  const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(decoded);
  if (!m) return true; // relative
  const scheme = m[1]!.toLowerCase();
  if (scheme === 'http' || scheme === 'https') return true;
  return kind === 'link' && scheme === 'mailto';
}

marked.use({
  renderer: {
    // Returning `false` hands the token back to marked's default renderer, so
    // an allowed target renders exactly as before (title, escaping, encoding).
    link(this: any, token: any) {
      if (safeLinkTarget(token.href, 'link')) return false;
      return this.parser.parseInline(token.tokens);
    },
    image(this: any, token: any) {
      if (safeLinkTarget(token.href, 'image')) return false;
      // The alt text is plain text; escape it as marked would have.
      return String(token.text ?? '').replace(/&(?!(?:#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    },
  },
});

marked.use({
  extensions: [
    {
      name: 'safeUrl',
      level: 'inline',
      start(src: string) {
        const i = src.search(/https?:\/\//);
        return i < 0 ? undefined : i;
      },
      tokenizer(src: string) {
        const m = /^https?:\/\/[^\s<]+/.exec(src);
        if (!m) return;
        let url = m[0];
        const cut = url.search(STOP);
        if (cut >= 0) url = url.slice(0, cut);
        // Trailing sentence punctuation is almost never part of the URL.
        url = url.replace(/[.,;:!?'"]+$/, '');
        // Nothing meaningful past the scheme → let the default tokenizers try.
        if (!/^https?:\/\/.+/.test(url)) return;
        return { type: 'safeUrl', raw: url, href: url, text: url };
      },
      // The text is a substring of the (already HTML-escaped) source and goes
      // through unchanged. The href is an ATTRIBUTE: a `"` in the URL would end
      // it and the rest of the URL would become markup, so it is percent-encoded
      // exactly as marked's own autolink does. An unencodable URL is text.
      renderer(token: any) {
        const href = encodeHref(token.href);
        return href === null ? token.text : `<a href="${href}">${token.text}</a>`;
      },
    },
  ],
});
