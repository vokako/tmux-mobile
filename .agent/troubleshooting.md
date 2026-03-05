# Known Issues & Troubleshooting

## Android File Opening
**Status**: Fixed
**Problem**: Two bugs with opening downloaded files on Android:
1. Download toast "Open" button: `Cannot construct instance of app.tauri.opener.OpenArgs` — caused by falling through to `tauriOpener.openPath()` which has a serialization bug on Android
2. Local file list open: `No file opener available` — `window.AndroidFileOpener` JS interface not registered yet

**Root cause**: `MainActivity.kt` used a single `rootView.post` to find and inject the JS interface into the WebView. If the WebView wasn't in the view hierarchy yet, `findWebView` returned null and the interface was never registered.

**Fix**:
- `MainActivity.kt`: Retry `attachFileOpener` with `postDelayed` (100ms intervals, max 50 attempts)
- `Files.svelte`: `openDownloaded()` checks `isAndroid` first, never falls through to `tauriOpener`. `openFileNative()` uses `waitForFileOpener()` (polls up to 2s for `window.AndroidFileOpener`)
- Rule: NEVER use `tauriOpener.openPath()` on Android — always use `AndroidFileOpener`

## tauri-plugin-opener on Android
**Status**: Known broken — do not use
The `@tauri-apps/plugin-opener` `openPath()` fails on Android with `OpenArgs` deserialization error. The Android plugin side expects a structured JSON object but receives a raw string. Use the custom `AndroidFileOpener` `@JavascriptInterface` in `MainActivity.kt` instead.

## Terminal Touch Scrolling (Mobile)
**Status**: Open — no inertia/momentum on mobile touch scroll
**Location**: `src/lib/Terminal.svelte`
**Details**: Uses ANSI→HTML rendering with native CSS overflow scrolling. Works but may lack perfect inertia on some devices.

## tmux History Limit
Recommend `set-option -g history-limit 50000` in tmux config. Default 2000 lines only holds ~3-5 conversation turns. Server captures 200 lines by default (`capture_pane` lines parameter).

## Android Build
Requires: Android SDK, NDK 28+, Java 17+, `aarch64-linux-android` Rust target.
Cleartext ws:// enabled via `network_security_config.xml` and `usesCleartextTraffic=true`.
After changing `tauri.conf.json` identifier, must delete `gen/android/` and run `tauri android init`.

## Path Traversal in Downloads
**Status**: Fixed
`save_to_downloads`, `delete_download`, `get_download_path` now use `sanitize_filename()` to strip directory components and reject `..` / empty names.

## Large File Upload Crash
**Status**: Fixed
`btoa(String.fromCharCode(...new Uint8Array(bytes)))` causes stack overflow on files >100KB due to JS argument limit. Fixed by chunking into 8192-byte segments.

## WebSocket Disconnect Hangs
**Status**: Fixed
Pending RPC promises were never rejected on disconnect. Now `rejectAllPending()` is called in `onclose` handler. Also, `connect()` cleans up any existing connection before creating a new one.

## Manual Disconnect During Reconnect
**Status**: Fixed
`doDisconnect()` now clears `reconnecting` state and `reconnectTimer` before calling `disconnect()`, preventing zombie reconnect attempts.

## HTML Preview Sandbox Escape
**Status**: Fixed
iframe sandbox had `allow-scripts allow-same-origin` together, which effectively negates the sandbox. Changed to `allow-same-origin` only (no script execution in previewed HTML).

## Markdown Image Preview
**Status**: Fixed
`resolveImages()` was using the markdown file's own mime type (`text/markdown`) for all resolved images. Fixed by inferring mime type from the image file's extension (`mimeFromName()`).
