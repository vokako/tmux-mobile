import '@xterm/xterm/css/xterm.css';
import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

// Text fonts come from the system (see the font-strategy comment in
// index.html); only two symbol fonts are bundled in public/fonts/ and
// declared via @font-face there. The browser loads each woff2 lazily when
// the CSS first references its family, so there's nothing to do here.

const app = mount(App, { target: document.getElementById('app')! });

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
