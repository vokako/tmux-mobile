# File Browser Page

## Purpose
Browse, preview, edit, and manage files on the remote server's filesystem. The
File Browser is always available (it does not require an open terminal pane).
The starting directory follows the active terminal/team session's working
directory, or the server's home directory when no session is open yet.

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

### Bookmarks / recent-files write discipline
Both lists persist whole-array last-writer-wins (`save_bookmarks`,
`set_pref('recentFiles')`) and RPCs are concurrent, so the client guards
against clobbering (`src/lib/Files.svelte`):
1. Never persist before the first successful load (a write of the default
   `[]` would erase the server list); if the lazy load fails, skip
   persisting rather than wipe.
2. Every local mutation bumps a generation counter; fetch continuations
   only assign if their generation is still current — an in-flight read
   must not overwrite a newer local mutation.
3. The lazy first load is single-flighted (one shared promise).
These are client-side guards only; two *different* clients can still race
(server-side merge semantics would be the deeper fix — see unresolved.md).

## Edge Cases
- Base64 upload uses chunked encoding (8192 bytes/chunk) to avoid stack overflow
- Files > 5 MB (or not previewable by mime/name) open the info page instead of auto-loading preview; user confirms via preview button to avoid heavy transfers on mobile
- Markdown preview resolves relative image paths, infers MIME from image extension (not parent file)
- HTML preview iframe: `allow-same-origin` only, NO `allow-scripts` (sandbox escape prevention)
- Android downloads go to `/storage/emulated/0/Download/TmuxMobile/`, opened via FileProvider + Intent
- Android's downloaded-files list is sorted by filesystem modification time descending (newest first)
- Download progress ring and label use the same clamped integer percentage; the ring has exact, non-rounded endpoints
- Android file opening uses the `AndroidFileOpener` JS interface injected before initial page load by `onWebViewCreate`, NOT `tauri-plugin-opener`
- Android reattaches and health-checks the file opener after app resume; a failed download-complete Open remains retryable
- Filenames sanitized server-side (`sanitize_filename()`) to prevent path traversal
- Error states (downloading, uploading) always reset in catch blocks
- Git arguments are passed directly as argv, not through a shell; log format separators such as `|` are valid argument data
