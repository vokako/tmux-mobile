import '@xterm/xterm/css/xterm.css';
import { mount } from 'svelte';
import App from './App.svelte';

// Fonts are bundled in public/fonts/ and declared via @font-face in
// index.html (loaded from the local origin, not a CDN — see the comment
// there). The browser loads each woff2 lazily when the app's CSS first
// references its family, so there's nothing to do here at runtime.

const app = mount(App, { target: document.getElementById('app') });

// PWA: register the service worker so the browser treats the web UI as
// installable (enables the `beforeinstallprompt` offer surfaced by
// InstallPrompt.svelte). Skipped inside the Tauri shell — it's already a
// native app — and only on secure origins (https / localhost), which the
// Service Worker API requires.
const isTauri = !!(window.__TAURI__ || window.__TAURI_INTERNALS__);
if (!isTauri && 'serviceWorker' in navigator && window.isSecureContext) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch((err) => {
      console.warn('Service worker registration failed:', err);
    });
  });
}

export default app;
