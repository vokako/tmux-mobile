import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import {
  DEV_DOWNLOAD_PATH,
  DEV_WEB_PORT,
  DEV_WS_PATH,
  devServerTargets,
} from './scripts/dev-ports.mjs';

export function devProxy(env = process.env) {
  const target = devServerTargets(env);
  return {
    [DEV_WS_PATH]: { target: target.ws, ws: true },
    [DEV_DOWNLOAD_PATH]: { target: target.http },
  };
}

function allowCompactXtermLines() {
  const original = 'if(i<1)throw new Error(`${e} cannot be less than 1, value: ${i}`)';
  // Lower bound must stay in sync with LINE_HEIGHT_MIN in src/lib/terminal-prefs.svelte.js.
  const patched = 'if(i<(e==="lineHeight"?.4:1))throw new Error(`${e} cannot be less than ${e==="lineHeight"?.4:1}, value: ${i}`)';
  const glyphAssignment = 'f.textContent=R';
  const centeredGlyphAssignment = 'f.replaceChildren(Object.assign(this._document.createElement("span"),{className:"xterm-glyph",textContent:R}))';
  const compositionAssignment = 'this._compositionView.textContent=t.data';
  const centeredCompositionAssignment = 'this._compositionView.replaceChildren(Object.assign(this._compositionView.ownerDocument.createElement("span"),{className:"xterm-glyph",textContent:t.data}))';

  return {
    name: 'allow-compact-xterm-lines',
    enforce: 'pre',
    transform(code, id) {
      // Dev serves excluded deps with a `?v=<hash>` query on the id; strip it.
      if (!id.split('?')[0].endsWith('/@xterm/xterm/lib/xterm.mjs')) return;
      if (!code.includes(original)) throw new Error('Unsupported @xterm/xterm lineHeight validation');
      const glyphAssignments = code.split(glyphAssignment).length - 1;
      if (glyphAssignments !== 3) throw new Error('Unsupported @xterm/xterm DOM glyph renderer');
      const compositionAssignments = code.split(compositionAssignment).length - 1;
      if (compositionAssignments !== 1) throw new Error('Unsupported @xterm/xterm IME renderer');
      return code
        .replace(original, patched)
        .replaceAll(glyphAssignment, centeredGlyphAssignment)
        .replace(compositionAssignment, centeredCompositionAssignment);
    },
  };
}

export function createViteConfig(command, env = process.env) {
  return {
    plugins: [allowCompactXtermLines(), svelte()],
    // esbuild dep pre-bundling bypasses plugin transforms, so the compact-lines
    // patch above would never apply in dev. Serve @xterm/xterm from source.
    optimizeDeps: {
      exclude: ['@xterm/xterm'],
    },
    clearScreen: false,
    server: {
      host: '0.0.0.0',
      port: DEV_WEB_PORT,
      strictPort: true,
      allowedHosts: ['.ts.net', 'localhost'],
      ...(command === 'serve' ? { proxy: devProxy(env) } : {}),
    },
  };
}

export default defineConfig(({ command }) => createViteConfig(command));
