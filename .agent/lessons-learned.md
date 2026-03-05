# Lessons Learned

## 1. Terminal Output → Chat UI Parsing

### ANSI Colors Are the Best Semantic Markers
Text-only parsing (regex on stripped text) is fragile. A `>` character appears in code, markdown quotes, and actual prompts.

Use ANSI color codes as semantic delimiters BEFORE stripping them:
- Kiro CLI colors `>` differently: color 93 (purple) for user prompt, color 141 (light purple) for agent response
- Insert marker tokens using color-specific regex, then strip ANSI for text parsing

**Gotcha**: The color reset `\e[39m` after `>` is NOT always present. Make the reset optional in regex.

### Soft-Wrapped Lines
Use `capture-pane -J` flag to join soft-wrapped lines. Without it, messages get split at screen width and lines get misclassified.

### System Hints vs User Input
Kiro CLI shows placeholder hints at the prompt. Real user-typed text has NO ANSI codes. System hints are always colored — check if raw text starts with `\x1b[`.

### Thinking Spinner
Filter braille spinner lines (`/^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]\s*Thinking/i`), show CSS animation instead.

## 2. Tauri + Android Platform

### tauri-plugin-opener Is Broken on Android
`openPath()` fails with `OpenArgs` deserialization error. Use a custom `@JavascriptInterface` in `MainActivity.kt` with `FileProvider.getUriForFile()` + `Intent.ACTION_VIEW`.

### WebView JS Interface Timing
`addJavascriptInterface` via `rootView.post` can fail if the WebView isn't in the hierarchy yet. Use a retry loop with `postDelayed` (with a max retry limit to avoid infinite loops).

### Frontend Must Wait for JS Interface
Add a `waitForFileOpener()` helper that polls for `window.AndroidFileOpener` (up to 2s) before attempting to open files. Never fall through to broken Tauri plugin APIs.

### Tauri Plugin Readiness
Always `await tauriReady` (the Promise.all of plugin imports) before using any Tauri plugin. Dynamic imports are async.

### Platform Detection
```js
const isTauri = !!(window.__TAURI__ || window.__TAURI_INTERNALS__);
const isAndroid = /android/i.test(navigator.userAgent);
```
Always check `isAndroid` before falling back to generic Tauri APIs — some don't work on Android.

## 3. WebSocket Client Robustness

### Clean Up on Disconnect
- Reject all pending promises in `onclose` handler (otherwise callers hang forever)
- Close existing WebSocket before creating new one in `connect()` (prevents socket leaks on rapid reconnect)
- Wrap `JSON.parse` in try-catch in `onmessage` (malformed messages crash the handler)

### Manual Disconnect Must Cancel Reconnect
`doDisconnect()` must clear `reconnecting` flag and `clearTimeout(reconnectTimer)`, otherwise a pending reconnect timer fires after manual disconnect.

### Optional Chaining on Server Push Params
Use `data.params?.target` not `data.params.target` — malformed server messages can have missing params.

## 4. File Handling

### Base64 Encoding Stack Overflow
`btoa(String.fromCharCode(...new Uint8Array(bytes)))` crashes on files >100KB (JS argument limit). Chunk into 8192-byte segments:
```js
let binary = '';
for (let i = 0; i < bytes.length; i += 8192) {
  binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
}
const b64 = btoa(binary);
```

### Path Traversal in Local Downloads
Filenames from remote servers can contain `../`. Always sanitize with `Path::file_name()` in Rust before joining to the download directory.

### Markdown Image MIME Type
When resolving relative image paths in markdown preview, infer MIME from the image's filename extension — NOT from the parent markdown file's mime_hint.

### iframe Sandbox
Never combine `allow-scripts` + `allow-same-origin` — it negates the sandbox entirely. For HTML preview, `allow-same-origin` alone is sufficient (CSS/fonts work, no JS execution).

## 5. Common Pitfalls
- **Don't strip ANSI too early** — you lose semantic information needed for chat parsing
- **Test with real tmux output** — `tmux capture-pane -p -e` to see actual ANSI codes
- **Account for tmux screen width** — use `-J` flag or messages get split
- **Error state cleanup** — always reset loading/spinner states in catch blocks (e.g., `downloading = ''`)
- **Android gen/ files** — `MainActivity.kt`, `AndroidManifest.xml` etc. are in `src-tauri/gen/android/` and survive `tauri android init` only if backed up
