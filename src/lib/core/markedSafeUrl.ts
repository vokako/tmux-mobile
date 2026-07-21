// A bare-URL autolinker for `marked` that stops at CJK / fullwidth characters.
//
// GFM's built-in autolink extends a bare URL until whitespace, so a URL pasted
// immediately before non-ASCII text — e.g. `https://host/admin（密码：xyz）` —
// swallows the trailing `（密码：xyz）` into the href. This inline extension
// matches the URL up to the first CJK or fullwidth char (and drops trailing
// sentence punctuation), leaving the rest as plain text.
//
// `marked` is a singleton, so importing this module once registers it for every
// renderer (Files preview, Team chat, Chat view).
import { marked } from 'marked';

// Fullwidth forms, CJK punctuation, CJK/Kana/Hangul ideographs.
const STOP = /[\u2E80-\u9FFF\u3000-\u303F\u3040-\u30FF\uAC00-\uD7AF\uFF00-\uFFEF]/;

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
      // href/text are substrings of the (already HTML-escaped) source, so we
      // pass them through unchanged — matching marked's own autolink output.
      renderer(token: any) {
        return `<a href="${token.href}">${token.text}</a>`;
      },
    },
  ],
});
