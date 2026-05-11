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

### Base64 Chunking
`btoa(String.fromCharCode(...spread))` crashes on files >100KB (JS argument limit). Use 8192-byte chunks.

### iframe Sandbox
Never combine `allow-scripts` + `allow-same-origin` — negates sandbox entirely. Use `allow-same-origin` only for HTML preview.

### Path Traversal
All filenames from remote servers sanitized with `sanitize_filename()` (Rust `Path::file_name()`) before joining to download directory.

### Markdown Image MIME
Infer MIME from image file extension, not from parent markdown file's mime_hint.

## Lessons Learned
- Always reset loading/spinner states in catch blocks (e.g., `downloading = ''`)
- Android `gen/` files need backup before `tauri android init`
- When wrapping a stream in `BufStream` for peek-then-dispatch, flush before returning — `BufStream`'s write buffer is discarded on drop, so the tail of the response would otherwise be lost.
