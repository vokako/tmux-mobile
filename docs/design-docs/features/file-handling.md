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
The handler evaluates the anchor's resolved `href`, not only its literal
attribute, so relative and protocol-relative Markdown links cannot bypass it.
It listens to both primary `click` and middle-button `auxclick`; other auxiliary
buttons remain available for their normal context-menu behavior.
Tauri opens links through `plugin-opener`; browser mode uses a separate
`noopener` tab. A Tauri opener error is reported and never falls back to
`window.open`, which could create another in-app WebView.

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

### Markdown Image MIME
Infer MIME from image file extension, not from parent markdown file's mime_hint.

## Lessons Learned
- Always reset loading/spinner states in catch blocks (e.g., `downloading = ''`)
- Android `gen/` files need backup before `tauri android init`
- When wrapping a stream in `BufStream` for peek-then-dispatch, flush before returning — `BufStream`'s write buffer is discarded on drop, so the tail of the response would otherwise be lost.
