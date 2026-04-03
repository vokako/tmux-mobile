import '@xterm/xterm/css/xterm.css';
import { mount } from 'svelte';
import App from './App.svelte';

const fontWeights = [300, 600];
fontWeights.forEach(w => {
  const face = new FontFace(
    'Maple Mono',
    `url(https://cdn.jsdelivr.net/fontsource/fonts/maple-mono@latest/latin-${w}-normal.woff2) format('woff2')`,
    { weight: String(w), style: 'normal', display: 'swap' }
  );
  face.load().then(f => document.fonts.add(f)).catch(() => {});
});

const app = mount(App, { target: document.getElementById('app') });

export default app;
