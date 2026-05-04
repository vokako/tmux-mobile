# File Browser Page

## Purpose
Browse, preview, edit, and manage files on the remote server's filesystem. Starting directory is the active session's working directory.

## Components
- Unified toolbar: all actions in one compact icon row
- Breadcrumb path row (separate from toolbar)
- File/directory list with icons, size, modified date
- Bookmark panel (star current dir, scrollable saved paths)
- Recent files panel (last 20 opened files, scrollable, capped to 40vh)
- File preview: Markdown (rendered + mermaid + KaTeX), CSV (table), code (syntax highlighted), HTML (sandboxed iframe), PDF (pdf.js), images
- Text editor with syntax highlighting, undo stack, save button
- File operations: create file/folder, rename, delete, upload, download
- File info panel: path (tap to copy), type, size, modified, permissions
- Git integration: status view, per-file stage/unstage, diff viewer, commit log, add all/commit/push

## Interactions
- Tap directory → navigate into it
- Tap file → preview (or info page if file size > 5 MB or not previewable)
- From info page → tap preview (eye) button to load preview on demand
- Long file names in the list scroll horizontally on touch drag
- Tap edit → open text editor
- Long-press / info button → file info panel
- Star button → bookmark current directory
- Refresh button in the toolbar → re-list current directory
- Swipe right from left edge → go back
- Upload button → file picker (Tauri on desktop/Android, `<input>` in browser)
- Download button → save file locally

## API Calls
- `fs_cwd(session)` — get session working directory
- `fs_list(path, show_hidden)` — list directory
- `fs_stat(path)` — file metadata
- `fs_read(path)` — read text file (≤512KB)
- `fs_write(path, content)` — save text file
- `fs_mkdir(path)` — create directory
- `fs_delete(path)` — delete file/directory
- `fs_rename(from, to)` — rename/move
- `fs_download(path)` — download as base64 (≤50MB)
- `fs_upload(path, data)` — upload as base64
- `fs_convert(path, format?)` — convert file to HTML for preview (currently pptx only, requires python3 + python-pptx)
- `git(subcmd, args, cwd)` — git operations
- `get_bookmarks()` / `save_bookmarks(bookmarks)` — bookmark persistence

## State Management
- Current path, directory listing
- Preview content and mode (markdown/csv/code/html/pdf/image)
- Editor content, unsaved changes flag, undo stack
- Bookmarks array (server-side persistence)
- Show hidden files toggle
- Git status, diff, log data

## Edge Cases
- Base64 upload uses chunked encoding (8192 bytes/chunk) to avoid stack overflow
- Files > 5 MB (or not previewable by mime/name) open the info page instead of auto-loading preview; user confirms via preview button to avoid heavy transfers on mobile
- Markdown preview resolves relative image paths, infers MIME from image extension (not parent file)
- HTML preview iframe: `allow-same-origin` only, NO `allow-scripts` (sandbox escape prevention)
- Android downloads go to `/storage/emulated/0/Download/TmuxMobile/`, opened via FileProvider + Intent
- Android file opening uses `AndroidFileOpener` JS interface, NOT `tauri-plugin-opener`
- Filenames sanitized server-side (`sanitize_filename()`) to prevent path traversal
- Error states (downloading, uploading) always reset in catch blocks
