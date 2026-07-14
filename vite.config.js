import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

function allowCompactXtermLines() {
  const original = 'if(i<1)throw new Error(`${e} cannot be less than 1, value: ${i}`)';
  const patched = 'if(i<(e==="lineHeight"?.8:1))throw new Error(`${e} cannot be less than ${e==="lineHeight"?.8:1}, value: ${i}`)';

  return {
    name: 'allow-compact-xterm-lines',
    enforce: 'pre',
    transform(code, id) {
      if (!id.endsWith('/@xterm/xterm/lib/xterm.mjs')) return;
      if (!code.includes(original)) throw new Error('Unsupported @xterm/xterm lineHeight validation');
      return code.replace(original, patched);
    },
  };
}

export default defineConfig({
  plugins: [allowCompactXtermLines(), svelte()],
  clearScreen: false,
  server: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
    allowedHosts: ['.ts.net', 'localhost'],
  },
});
