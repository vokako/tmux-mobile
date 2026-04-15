# File Handling & Security

## Context
File browser handles uploads, downloads, and previews across platforms with varying security constraints.

## Decision
Base64 encoding for file transfer, chunked processing for large files, sandboxed previews, platform-specific download/open paths.

## Key Decisions

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
