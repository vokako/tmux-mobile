# PWA Install Offer (web only)

## Context
The web UI (Vite + Svelte, served over a browser — Tailscale HTTPS, a reverse
proxy, or `npm run preview`) is a long-lived companion screen. Browsers can
"install" such a page as a standalone app (own window, home-screen icon, no
URL bar). We surface that offer in-app instead of leaving it to the browser's
easily-missed mini-infobar. The Tauri desktop/Android shells are already
native apps, so this is strictly a **browser-context** feature.

## Decision
Minimal hand-rolled PWA (no `vite-plugin-pwa` dependency): a static manifest +
a no-op service worker satisfy installability, and a small Svelte banner
(`InstallPrompt.svelte`) drives the offer. No offline caching.

## Pieces
| File | Role |
|------|------|
| `public/manifest.webmanifest` | App metadata + icons (192/512 `any`, 512 `maskable`). `display: standalone`, theme/background `#0b0b0d` (dark bg). |
| `public/sw.js` | Service worker registered **only** to make the page installable. Network-passthrough `fetch` handler (no `respondWith`) — present purely because some Chromium versions gate installability on a fetch handler. `skipWaiting` + `clients.claim` so updates take over immediately. |
| `public/pwa-192.png`, `pwa-512.png`, `pwa-maskable-512.png` | Generated from `src-tauri/icons/icon.png` (1024²) via `sips`. The maskable one scales the glyph to ~72% over an opaque black field so it survives Android's circular mask (the source icon has transparent rounded corners — unusable as maskable directly). |
| `index.html` | `<link rel="manifest">` + `theme-color` + `apple-*` tags (iOS "Add to Home Screen" reads these; there's no JS install API on iOS). |
| `src/main.ts` | Registers `/sw.js` — gated on `!isTauri && 'serviceWorker' in navigator && window.isSecureContext`. |
| `src/lib/ui/InstallPrompt.svelte` | The banner. Mounted once in `App.svelte`. |

## Why no offline caching
tmux-mobile is useless without a live WebSocket server — there's nothing
meaningful to serve offline. A caching SW would only add risk: serving a stale
build after deploy. So the SW deliberately caches nothing; it exists solely for
the installability signal.

## InstallPrompt behavior
Two paths, because install APIs differ by engine:
- **Chromium (Chrome/Edge/Android/Samsung):** the browser fires
  `beforeinstallprompt`; we `preventDefault()` it, stash the event, show our
  banner, and call the stashed event's `prompt()` when the user taps Install.
- **iOS Safari:** no programmatic API exists. We detect iOS Safari (UA, with
  the iPadOS-masquerades-as-Mac touch check) and show "Share → Add to Home
  Screen" instructions after a short delay. In-app browsers / CriOS / FxiOS are
  excluded (only Safari can add to home screen on iOS).

Suppression rules (never nag):
- Hidden inside Tauri (`__TAURI__`/`__TAURI_INTERNALS__`).
- Hidden when already running standalone (`display-mode: standalone` /
  `navigator.standalone`).
- Dismissal is remembered in `localStorage` (`tmux_pwa_dismissed`) for a 14-day
  cooldown. `appinstalled` clears it.

## Gotchas / constraints
- **HTTPS required.** Service workers + `beforeinstallprompt` only work on
  secure origins (https or `localhost`). Plain `http://<LAN-ip>:5173` (the
  default dev/preview path over LAN) is **not** a secure context, so the offer
  won't appear there — this is expected, not a bug. The intended install path
  is the Tailscale HTTPS serve (see README) or any TLS reverse proxy.
- **iOS timing.** The iOS banner is delayed ~2.5 s so it doesn't collide with
  first paint / the connect screen.
- Icons are committed PNGs (regenerate with `sips` from `src-tauri/icons/icon.png`
  if the brand icon changes).
