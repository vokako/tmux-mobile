# Frontend Conventions

How the Svelte/TypeScript side is written: the TypeScript migration rules, platform checks, Tauri plugin loading, and the handful of data-handling rules that bit us once. The design language has its own normative doc (`../design-docs/features/design-language.md`).

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### TypeScript migration (in progress)

new modules are written in TS; existing `.js` is converted file-by-file (rename + types, no logic changes in the same commit). Rules: relative imports use **explicit `.ts` extensions** (the same files are executed by both Vite and `node --test`, which does native type stripping — needs Node ≥ 23.6); **erasable syntax only** (no enums/namespaces — `erasableSyntaxOnly` enforces this); type-checking is `npm run check` only (Vite/node never type-check). svelte-check requires TypeScript 5.x (7.x is the incompatible Go rewrite — both are pinned exact).

### Platform checks

`isTauri` (Tauri vs browser), `isAndroid` (Android vs macOS). Always check `isAndroid` first.

### Tauri plugins

Always `await tauriReady` before use. Dynamic imports gated behind platform checks.

### Android file opening

Use `AndroidFileOpener` JS interface, NEVER `tauri-plugin-opener`.

### Base64 large data

Chunked 8192 bytes per chunk. Never spread all bytes.

### HTML preview

iframe `allow-same-origin` only, NO `allow-scripts`.

### Chat markdown escapes `&` and `<`, never `>`

those two are all raw HTML needs to be inert (a tag cannot start without `<`), and escaping `>` as well silently killed every markdown construct that uses it — `> quote` reached the parser as `&gt; quote`, so blockquotes NEVER rendered. Strikethrough is also stricter than GFM here: `~~double~~` only, one line only, and an unpaired tilde run is CONSUMED as text (an overridden marked tokenizer that returns false falls back to marked's own, so refusing the match is not enough). Reason: `26~32℃` is ordinary Chinese punctuation and single-tilde GFM struck out half a weather report. In chat rendered view, a complete `markdown`/`md` fence is a transparent wrapper: agents wrap requested `.md` documents in one, and rendering that as `<pre>` made rendered mode look like raw (`proj:test` seq=52). Raw still shows exact `m.body`; other-language and unclosed fences remain code.
