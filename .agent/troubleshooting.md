# Known Issues & Troubleshooting

## Terminal Touch Scrolling (Mobile)
**Status**: Bug — no inertia/momentum on mobile touch scroll
**Location**: `src/lib/Terminal.svelte` touchstart/touchmove handlers
**Details**: Current approach proxies touch events to xterm's `.xterm-viewport` scrollTop. Works but feels mechanical — no native inertia or elastic bounce. CSS `touch-action: pan-y` on xterm-screen doesn't work because canvas captures events.
**Potential fixes**:
- Investigate xterm.js addons for mobile support
- Try making xterm-screen `pointer-events: none` only on touch devices
- Implement momentum with `requestAnimationFrame` and velocity decay

## tmux History Limit
Recommend `set-option -g history-limit 50000` in tmux config. Default 2000 lines only holds ~3-5 conversation turns. App captures 200 lines by default.

## Android Build
Requires: Android SDK, NDK 28+, Java 17+, `aarch64-linux-android` Rust target.
Cleartext ws:// enabled via `network_security_config.xml` and `usesCleartextTraffic=true`.
After changing `tauri.conf.json` identifier, must delete `gen/android/` and run `tauri android init`.
