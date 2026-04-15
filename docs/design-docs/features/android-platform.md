# Android Platform Integration

## Context
Tauri 2 on Android has several plugin bugs and WebView limitations that require custom workarounds.

## Decision
Use custom `@JavascriptInterface` in `MainActivity.kt` for file opening instead of `tauri-plugin-opener`. Implement retry-based WebView detection for JS interface injection.

## How It Works
- `MainActivity.kt` → `FileOpener` inner class → `@JavascriptInterface` → `FileProvider.getUriForFile` → `Intent.ACTION_VIEW`
- `attachFileOpener` retries up to 50 times (5s) waiting for WebView in view hierarchy
- Frontend: `waitForFileOpener()` polls for `window.AndroidFileOpener` (up to 2s) before file open
- Downloads go to `/storage/emulated/0/Download/TmuxMobile/`
- Keyboard height via `OnGlobalLayoutListener`, safe area insets via `WindowInsetsCompat`
- Cleartext ws:// enabled via `network_security_config.xml`

## Alternatives Considered
- **tauri-plugin-opener**: Rejected — `openPath()` fails with `OpenArgs` deserialization error on Android
- **Single rootView.post for JS interface**: Rejected — WebView may not be in hierarchy yet, causing silent failure

## Trade-offs
- Custom native code to maintain in `MainActivity.kt`
- Retry loops add startup latency (up to 5s worst case)
- `gen/android/` files survive `tauri android init` only if backed up

## Lessons Learned
- NEVER use `tauriOpener.openPath()` on Android — always use `AndroidFileOpener`
- Always check `isAndroid` before falling back to generic Tauri APIs
- `addJavascriptInterface` via single `rootView.post` can fail silently — use retry loop with max attempts
- After changing `tauri.conf.json` identifier, must delete `gen/android/` and run `tauri android init`
