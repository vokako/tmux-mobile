# Frontend Conventions

How the Svelte/TypeScript side is written: the TypeScript migration rules, platform checks, Tauri plugin loading, and the handful of data-handling rules that bit us once. The design language has its own normative doc (`../design-docs/features/design-language.md`).

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### TypeScript migration (in progress)

new modules are written in TS; existing `.js` is converted file-by-file (rename + types, no logic changes in the same commit). Rules: relative imports use **explicit `.ts` extensions** (the same files are executed by both Vite and `node --test`, which does native type stripping — needs Node ≥ 23.6); **erasable syntax only** (no enums/namespaces — `erasableSyntaxOnly` enforces this); type-checking is `npm run check` only (Vite/node never type-check). svelte-check requires TypeScript 5.x (7.x is the incompatible Go rewrite — both are pinned exact).

### Platform checks

`src/lib/core/platform.ts` is the ONE place the platform is decided; import its flags, never re-derive them from `window.__TAURI_INTERNALS__` or a `/android/i` test (until 2026-09-03 six files each had their own copy and the rule named symbols that existed nowhere, so it could not be tested). `isTauri` (inside any Tauri shell vs browser/PWA), `isAndroid` (the Android shell — implies `isTauri`), `isTauriDesktop` (the macOS shell). Always check `isAndroid` first: Android IS a Tauri shell, so a bare `isTauri` branch that reaches for a desktop plugin runs on the phone too. `detectPlatform(win, nav)` is the pure form for tests (`platform.test.ts`). `external-links.ts` keeps a window-parameterised `isTauriWindow` for its own tests, implemented on the same helper.

### Tauri plugins

Always `await tauriReady` (from `platform.ts`) before importing or calling a `@tauri-apps/*` plugin, inside an `isTauri` branch. Dynamic imports gated behind platform checks; a component that needs the plugin MODULES keeps its own promise chained on `tauriReady` (`tauriPlugins` in `Files.svelte`).

### Android file opening

Use `AndroidFileOpener` JS interface, NEVER `tauri-plugin-opener`.

### Base64 large data

Chunked 8192 bytes per chunk. Never spread all bytes.

### HTML preview

iframe `allow-same-origin` only, NO `allow-scripts`.

### Chat markdown escapes `&` and `<` in text, never `>`; attributes have their own guards

those two are all raw HTML needs to be inert as TEXT (a tag cannot start without `<`), and escaping `>` as well silently killed every markdown construct that uses it. Text is not the only context, though (review, 2026-09-03): an `href` is an ATTRIBUTE, and a `"` inside a URL ends it — `https://a.b/x"onclick="alert(1)` autolinked into an anchor with a live `onclick`. And marked v17 does not sanitize schemes, so `[x](javascript:…)` reached the DOM as written. Both guards live in `core/markedSafeUrl.ts`, registered once on the `marked` singleton so EVERY renderer inherits them: the autolinker percent-encodes its href exactly as marked's own `cleanUrl` does, and a `link`/`image` renderer override renders any target whose scheme is not `http`, `https` or `mailto` (images: `http`/`https` only — an image is a reference to bytes elsewhere, never bytes) as its text/alt. `safeLinkTarget()` reads the scheme the way a browser reads an attribute — numeric and `&colon;` entities decoded, ASCII controls and spaces stripped — because `java\tscript:` is `javascript:` to WebKit. Tests: `markdown.test.ts` (the two reproduced payloads through `renderMarkdown`) and `markedSafeUrl.test.ts` (the guard's own table) — `> quote` reached the parser as `&gt; quote`, so blockquotes NEVER rendered. Strikethrough is also stricter than GFM here: `~~double~~` only, one line only, and an unpaired tilde run is CONSUMED as text (an overridden marked tokenizer that returns false falls back to marked's own, so refusing the match is not enough). Reason: `26~32℃` is ordinary Chinese punctuation and single-tilde GFM struck out half a weather report. In chat rendered view, a complete `markdown`/`md` fence is a transparent wrapper: agents wrap requested `.md` documents in one, and rendering that as `<pre>` made rendered mode look like raw (`proj:test` seq=52). Raw still shows exact `m.body`; other-language and unclosed fences remain code.
