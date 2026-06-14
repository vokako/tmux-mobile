import '@xterm/xterm/css/xterm.css';
import { mount } from 'svelte';
import App from './App.svelte';

// Fonts are bundled in public/fonts/ and declared via @font-face in
// index.html (loaded from the local origin, not a CDN — see the comment
// there). The browser loads each woff2 lazily when the app's CSS first
// references its family, so there's nothing to do here at runtime.

const app = mount(App, { target: document.getElementById('app') });

export default app;
