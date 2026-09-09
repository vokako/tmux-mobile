# File Handling & Security

## Context
File browser handles uploads, downloads, and previews across platforms with varying security constraints.

## Decision
Base64 encoding for small file transfer (previews), streaming HTTP for large-file downloads (user-initiated), chunked processing, sandboxed previews, platform-specific download/open paths.

## Key Decisions

### Two Download Paths
| Path | Used for | Size limit | Transport |
|------|----------|-----------|-----------|
| `fs_download` (WS RPC) | Inline PDF/image preview, Markdown-embedded images | 50 MB (`MAX_READ_SIZE`) | JSON-RPC over WebSocket, base64 |
| `/dl?path=…&ts=…&sig=…` | User-initiated file download | None — streams chunks | Plain HTTP on same port |

Frontend `fsDownloadHttp` always uses the streaming HTTP path now (both `ws://` and `wss://`). The server peeks the first bytes of every accepted connection (plain TCP via `TcpStream::peek`; TLS via `BufStream::fill_buf` after the TLS handshake) and branches HTTP vs WebSocket-upgrade. This is what keeps a 56 MB .pptx download working over `wss://` — before, `wss://` fell back to the WS RPC path and tripped `MAX_READ_SIZE`.

`fs_download` stays — it's still the right choice for inline preview (the browser wants the bytes as `data:` URL anyway, so the base64 it gets from the server is already the final shape).

### Resumable downloads (Range + retry)
Public-internet paths (reverse proxy in front of the server) routinely kill long-lived large responses: proxy idle/total timeouts, connection resets, silent stalls. Three pieces make `/dl` survive that:

- **Server: `Range: bytes=N-` support.** `/dl` answers `206 Partial Content` + `Content-Range` for the open-ended single-range form, and advertises `Accept-Ranges: bytes`. Only `bytes=N-` is honored (the only form our client sends); other forms fall back to a full 200, which is legal per RFC 7233.
- **Server: robust request parsing.** The request is read until `\r\n\r\n` (a proxy may split the request line across TCP segments — a single `read()` used to truncate the query mid-signature and 403 valid requests). `/dl?` is located anywhere in the request line so an unstripped proxy path prefix (`GET /tmux/dl?...`) still routes. Same prefix tolerance in the HTTP-vs-WS dispatch (`looks_like_dl_request`, request line only — header echoes don't match).
- **Client: `fetchWithResume` (Files.svelte).** Streams the body with a 20 s stall watchdog (AbortController); on any mid-transfer failure it retries with `Range: bytes=<received>-`, so finished bytes are never re-fetched. Each retry re-signs the URL via `fs_download_url` (signatures expire after 60 s — a retry minutes into a transfer would otherwise 403). The retry budget (4) refills whenever an attempt makes progress, so a flaky-but-moving link survives many small interruptions; only consecutive zero-progress failures abort. If a resume gets 200 instead of 206 (proxy stripped the Range), the client restarts from byte 0 rather than corrupting the buffer.

### Base64 Chunking
`btoa(String.fromCharCode(...spread))` crashes on files >100KB (JS argument limit). Use 8192-byte chunks.

### iframe Sandbox
Never combine `allow-scripts` + `allow-same-origin` — negates sandbox entirely. Use `allow-same-origin` only for HTML preview.

HTTP(S) links from previews must not navigate the embedded WebView. App-level
links, Markdown, and converted HTML use the shared delegated handler in
`external-links.js`; a raw HTML preview installs the same handler directly on
the sandbox iframe's document because DOM events do not cross iframe boundaries.
The handler classifies the literal `href`: explicit HTTP(S) URLs and
protocol-relative network URLs are external; schemeless file paths and
`#fragment` links are not. Only protocol-relative URLs use the browser's
resolved `href`.
It listens to both primary `click` and middle-button `auxclick`; other auxiliary
buttons remain available for their normal context-menu behavior.
Tauri opens links through `plugin-opener`; browser mode uses a separate
`noopener` tab. A Tauri opener error is reported and never falls back to
`window.open`, which could create another in-app WebView.

### File references stay in Files (#106, 2026-09-09)

Root cause: the document's **capture** external-link handler used resolved
`anchor.href`, turning `/local/.../notes.md` into `http://localhost:5173/local/...`.
It called `window.open` before the Hub/Files bubble handler routed the same
click into a preview. The late #99 `preventDefault` net could not undo a
programmatic open. Reproduced on Chromium 152.0.7977.64 against the current
`:5173` module, not inferred from an old client.

`core/path-links.ts` owns path classification, line-suffix stripping, URI
decoding and filesystem-relative resolution. Hub uses the project cwd; all
Files previews use the source document's directory. Plain, Cmd/Ctrl and
middle clicks route through the same handler. The preview body covers
Markdown, converted HTML and other rendered content; the sandbox HTML
iframe installs that same handler on its own document because events cannot
bubble into the parent. It keeps `allow-same-origin` only. PDF currently
renders canvases without an annotation/link layer; CSV renders escaped table
cells, not an iframe or active links. No new renderer is introduced.

Linked previews retain their origin (document/list, parent listing and reading
position). Back buttons and mobile browser/gesture Back consume this history
before the directory floor; Hub delegates its Back handler into the Files
drawer before closing it. Desktop browser toolbar Back retains the app-shell
contract (no global history trap). An initial reference opens with its parent
listing below it. Listing the
target's parent must not race and clear the preview: it is awaited as a
non-navigating listing refresh. Newer file/directory intents invalidate older
file loads. Missing files leave the existing preview and show the localized
path error; they never fall back to browser navigation.

On first mount (also the mobile chat-to-Files jump), a cwd response records
the source directory but does not override an explicit path handoff. The
same `cwdFollowStep` decision owns this precedence; a later real cwd change
still follows normally. Previously the concurrent follow could send the
newly opened preview back to the session directory.

True HTTP(S) URLs retain the external opener in both browser and Tauri;
mailto and document fragments retain their native behavior. The app-wide
path net remains only for otherwise unrouted surfaces, not as the fix for
an incorrect external classification. Unit tests reproduce the capture/router
ordering and source contracts pin the primary, auxiliary and iframe wiring.

### Content Security Policy and opener scope

`tauri.conf.json` sets a CSP with `script-src 'self'` and no `'unsafe-inline'`
for scripts (2026-09-03; it was `null`). The webview renders untrusted text
through `{@html}` — agent chat markdown, repository READMEs, mermaid SVG, KaTeX —
and `withGlobalTauri` exposes IPC on `window`. Escaping is the first line of
defence (`core/markdown.ts`); the CSP is the second: an escaping bug becomes a
broken image, not a script with IPC access. Inline STYLES stay allowed because
xterm, KaTeX and mermaid all emit them, and stylesheets and fonts may come from
any http(s) host: the HTML preview is a `srcdoc` iframe, which INHERITS this
policy, and a previewed report that links a CDN stylesheet or web font must
still look like itself. Scripts in that iframe were never run (the sandbox has
no `allow-scripts`), so the policy that matters — `script-src 'self'` — costs
the preview nothing. `connect-src` allows any `ws:`/`wss:`/
`http:`/`https:` host because the server address is user-entered, plus
`ipc: http://ipc.localhost` for Tauri's IPC. Bundled index.html has no inline
script (one `<script type="module" src>`), so nothing legitimate is blocked.
The policy governs only the Tauri webview; the browser/PWA build has no CSP
header today.

The opener capability (`capabilities/default.json`) was `path: "**"`. It exists
for exactly one call — opening the file the user just saved through the save
dialog — so it is now scoped to `$HOME`, `$DOWNLOAD`, `$DOCUMENT`, `$DESKTOP`,
`$APPCACHE` and `$TEMP`. Android never uses it (`AndroidFileOpener`). A save
outside those trees still succeeds; only the "open it" button reports an
error, which is the right failure.

### Path Traversal
All filenames from remote servers sanitized with `sanitize_filename()` (Rust `Path::file_name()`) before joining to download directory.

### .pptx preview is extracted in-process
`fs_convert` used to shell out to `python3 -c "import pptx …"`. That made the
feature depend on a `python-pptx` install on the server machine; where it was
missing, the preview surfaced a raw `ModuleNotFoundError` traceback in the UI.
`src-tauri/src/pptx.rs` now reads the deck directly: a .pptx is a zip of XML, so
the module has a small central-directory zip reader (store + deflate via the
`flate2` dependency we already carry, CRC-verified) and a single-pass scan of
`ppt/slides/slideN.xml` for `<a:p>` paragraphs and `<a:tbl>` tables. Same HTML
card markup as before, so the frontend is untouched.

Details worth keeping:
- **Slide order comes from `<p:sldIdLst>`**, not file names — PowerPoint keeps
  `slideN.xml` fixed when slides are reordered or deleted. Numeric sort is only
  the fallback when `presentation.xml`/its rels are unreadable.
- **Zip64 is rejected with a clear message** rather than mis-parsed; no deck
  generator we've seen emits it under 4 GB.
- Cross-checked against `python-pptx` on 17 real decks: identical slide counts,
  no missing text. The native scan finds *more* text — it walks grouped shapes,
  which `python-pptx`'s flat `slide.shapes` skips.

### Git command argumentsThe Git RPC keeps an explicit subcommand allowlist, then passes every argument
directly through Rust `Command::args` without a shell. Shell metacharacters are
therefore ordinary argument data (the log view uses `|` in `--format`); only
NUL is rejected because operating-system argv cannot represent it.

### Git verbs report through one banner
`GitPanel.svelte` has ONE outcome surface for git verbs: `flash(msg)` sets `pushResult` for 3 s under the header (`✗ ` prefix = failure, red). Push and commit always used it; stage, unstage and add-all wrapped their call in `catch {}` and said nothing, so a failing `git add` (index.lock left by a crash, permissions, a path outside the work tree) looked like "the plus button did nothing" (review, 2026-09-03). Now every verb's catch goes through `failed(e)` → the same banner — no second error mechanism. One timer, restarted per message, so a slow earlier 3 s timeout cannot blank a newer message. `git()` also throws on ANY non-zero exit, naming the exit code when stderr is empty, so the banner never has nothing to show. `gitError` remains the LOAD error (status/log listing failed) and is a separate, persistent line.

### Markdown preview uses the one safe renderer
The `FilePreview` body calls `renderMarkdown` from `src/lib/core/markdown.ts` — the same escape-first pipeline the chat uses (rule 13: `&` and `<` are escaped BEFORE marked parses, `>` is not, so blockquotes work and raw HTML is inert text). Files used to carry a second renderer that called `marked.parse` on the raw file, and marked v17 does not sanitize: a `README.md` in any cloned repo (or one an agent wrote) containing `<img src=x onerror=…>` ran in the app origin, where `localStorage` holds the token and `__TAURI_INTERNALS__` can invoke commands (review, 2026-09-03). The trade-off is deliberate: a README's inline HTML (badge tables, centered logos) renders as text; markdown-syntax images and links still work. Mermaid fences still render — the shared output keeps `code.language-mermaid`, and `renderMermaidBlocks` swaps them for SVG after paint. `FilePreview.source.test.ts` pins the shared renderer; Files only delegates the body and never calls `marked.parse`.

### Heavy preview libraries load on first use
pdf.js, mermaid and highlight.js (+15 grammars) are `import()`ed by memoized loaders (`loadPdfjs`, `loadMermaid`, `loadHljs`) in `file-preview.ts` the first time a PDF, a mermaid fence or a code/text file is opened. Files is statically imported by App and the Hub drawer, so the static imports it had put 1.5 MB into the entry chunk of the primary (Android) target: 2.30 MB (652 KB gzip) before, 1.14 MB (344 KB gzip) after (review, 2026-09-03). The highlighter is a `$state`: the lined preview and the editor overlay render escaped-and-plain until it lands, then re-render highlighted; a markdown file with no diagram never loads mermaid. KaTeX stays static because `core/markdown.ts` (chat) needs it on the first message.

### Navigation ownership (#110, 2026-09-09)

`file-nav.ts` owns the non-reactive directory and linked-preview histories,
file request generation and preview/info Back decisions. One factory instance
belongs to each Files component; the full Files page and Hub drawer never
share these stacks. `file-view-state.ts` still owns editor-exit and cwd-follow
decisions. Files applies the results through its existing RPC, confirmation,
motion, browser-history and DOM-scroll code.

This is a behavior-preserving extraction of the #106 implementation, not a
new navigation model: linked origins precede Git/parent fallback, loaded text
info first returns to its preview, directory Back retraces visits, and only
tab visits climb below the directory stack. Clearing either history does not
implicitly invalidate a file request; the existing call sites still advance
the generation explicitly. Unit tests characterize each decision and preserve
the captured listing/file/scroll references.

### Preview ownership (#110, 2026-09-09)

`FilePreview.svelte` contains the original preview-body branches and their
CSS, with no new wrapper or header. `file-preview.ts` holds MIME/highlighting/
CSV helpers and the lazy PDF, Mermaid, image and iframe-link renderers.
Files creates one renderer factory per component lifetime and keeps the
existing 50 ms scheduling, highlighter mirror, DOM bindings and code-cap
state. Live context getters deliberately retain the pre-extraction reads;
this move does not change parser behavior or asynchronous cancellation/error
policy. Closing a preview or visiting the editor does not recreate loader
caches or reset an expanded code preview.

Pure helper tests characterize existing behavior (including the simple CSV
splitter), renderer boundary tests cover image/link handling, and the real
Svelte render test covers every branch plus the 3000-line cap. Files and
FilePreview each pin their own wiring; browser comparison covers the actual
rendered output and Back chain on both layouts.

### Markdown Image MIME
Infer MIME from image file extension, not from parent markdown file's mime_hint.

### The house icon means `~`, and only `~`
`DirPicker`'s house opens the user's home (`~`, resolved by the server). Files' toolbar wore the same house for `fsCwd(session)` — the active pane's working directory — so one glyph pointed at two places depending on the screen (review, 2026-09-03). The destination was the useful one for a file browser that follows the terminal, so the CONTROL stayed and the glyph changed: it is now the `terminal` icon with a `filesSessionDir` title/aria-label ("Session directory"). Rule: an icon is a promise about where you land; reusing `home` for anything but `~` breaks it, and a new destination gets its own glyph rather than borrowing one.

### Every way out of the editor asks once — through one helper
`Files.svelte` has ONE exit for the editor, `leaveEditor(run)`: with no unsaved edits the move runs at once; with edits it is parked as `pendingAct = { kind: 'leave', run }` behind the shared ConfirmDialog and runs on confirm. Cancel drops the move — nothing is queued. Until 2026-09-03 only the back button asked; the session/pane switch, the follow-the-real-cwd rule and the drawer's "look here" (`navRequest`) each set `view = 'list'` outright and the text was silently gone (review finding, highest priority). The decision is pure and unit-tested in `file-view-state.ts`: `leaveDecision({ view, edited })` and `cwdFollowStep(reported, lastSourceDir, guard)`. The follow step commits `lastSourceDir` BEFORE the dialog, so a cancelled follow is skipped for that event and the same cwd does not ask again on the next effect run (it would otherwise re-prompt every time the tab regains visibility). `leaveEditor` reads `view`/`isEdited` under `untrack` because its callers are `$effect`s that must not re-run on every keystroke. When a session switch and a cwd follow both want to move in the same event, the later (follow) replaces the earlier pending move — following the real cwd already outranks the parked position.

## Motion

The file browser follows [motion.md](motion.md). A directory load KEEPS the
rows on screen and dims them (`.file-list.busy`, opacity 0.55) only after a
150ms threshold — DirPicker's rule, so a fast listing never blinks and the
"Loading…" placeholder appears only for the very first answer; GitPanel's
lists do the same. Things that enter animate: the bookmarks/recent panel, the
new-item/rename rows and the commit row `.appear-rise`; the drop hint, the
error banners and the push-result banner `.appear`; the download toasts rise
in, and the "Copied" flash is one local in+out keyframe (`toast-fade`, fade in
over 10%, hold, fade out) because it is a one-shot, not an intro atom. The
toasts centre with auto margins rather than `translateX(-50%)` so the intro
owns `transform`. The bookmark star swaps glyphs (`{#key}` + `.appear-pop`)
because no rotation reads star → star-filled. Breadcrumbs are keyed by path
and only the tip fades in. Git status rows are keyed by file and flip on
`moveMs()`; the git diff drills in from the right and the list back from the
left under 760px with the app.css `drill-in-*` keyframe pair (one copy: a
component references a global keyframe by name; only a LOCAL `@keyframes`
gets scoped). The progress arc is still not transitioned (it
tracks the integer). Exits everywhere are cuts.

Hover / unfold / highlight (#86, 2026-09-04): a file row's hover card
(`use:hoverInfo` on `.file-main`, principle 16) shows kind (folder / file /
symlink → target / broken link), size and modified through the SAME
`formatSize`/`formatDate` the info view uses; a breadcrumb's card is the full
path, a bookmark's or recent file's its path, and a git status row's the
porcelain code in words ("modified, staged · modified, unstaged",
`files/git-status.ts`, tested). The native `title` on the file name is gone.
A directory's ENTRANCE is one beat, at answer time (board #93, owner: "旧的
页面滑出去，新的页面进来。同时新的页面应该从上到下按行显示过渡加载。新页面加
载和滑入是同时进行的，有可能加载慢就可能慢半拍"): the navigation records its
direction (`pendingSlide` — deeper is `fwd`; up, crumbs and the history pop
are `back`), and when the answer lands the drill slide and the top-to-bottom
row unfold start TOGETHER. The tap-time slide was the flash: it finished
over the OLD rows and the swap then read as a detached blink. A slow answer
starts its entrance late — the honest reading — with the busy dim (150ms
threshold) covering the wait. The unfold (`revealDir`, dropped after
`revealMs()` per the atom's contract) also plays for a first fill and for
slide-less navigations (the desktop split, an external jump), where it IS
the whole entrance; a same-directory refresh keeps its nodes and animates
nothing; view switches (preview, editor) still slide at tap time — they
swap instantly. DirPicker keeps its one-shot first-answer `reveal`.
GitPanel's Status/Log tabs are
the ONE travelling highlight (`use:slideIndicator` + `.slide-pill`): the pill
glides, the buttons only change colour.

## Lessons Learned
- Always reset loading/spinner states in catch blocks (e.g., `downloading = ''`)
- Android `gen/` files need backup before `tauri android init`
- When wrapping a stream in `BufStream` for peek-then-dispatch, flush before returning — `BufStream`'s write buffer is discarded on drop, so the tail of the response would otherwise be lost.
