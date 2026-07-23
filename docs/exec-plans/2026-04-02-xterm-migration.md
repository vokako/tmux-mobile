# Refactor: Replace ansi-to-html with xterm.js

## Why
The custom ANSI-to-HTML cursor overlay has persistent positioning bugs (cursor_x adjustment for joined lines, visual row calculation for browser wrapping, trailing_empty off-by-one). xterm.js handles cursor, ANSI parsing, and wrapping natively.

## Previous xterm.js issues (commit c60fa59)
The old implementation had two problems:
1. **Manual touch scrolling** — proxied touchstart/touchmove to `.xterm-viewport.scrollTop` with `e.preventDefault()`, killing momentum/elastic scroll
2. **Flicker** — used `term.reset()` + `term.write(content)` on every update

## Plan

### 1. Install packages / remove ansi-to-html
```bash
npm install @xterm/xterm @xterm/addon-fit
npm uninstall ansi-to-html
```

### 2. Server-side: simplify cursor, add raw y/h (server.rs)
Add `cursor.y` (raw tmux cursor_y) and `cursor.h` (pane_height) to the pane_output message. The complex cursor_line/cursor_x_adj calculation can be removed — xterm.js handles cursor positioning via ANSI escape sequences.

New cursor JSON: `{ "x": cursor.0, "y": cursor.1, "w": cursor.3, "h": cursor.2 }`

Keep the `-J` content as-is (xterm.js re-wraps at the same column width).

### 3. Frontend: rewrite Terminal.svelte rendering

**Remove:**
- `import Convert from 'ansi-to-html'` and both converter instances
- `ansiVisibleWidth()` function
- `cursorStyle` derived computation
- `termHtml` state and the ANSI→HTML `$effect`
- `measureEl` (char-measure span) and its HTML
- `<span class="term-cursor">` overlay
- `.ansi-output`, `.term-cursor`, `.char-measure` CSS

**Add:**
- `import { Terminal } from '@xterm/xterm'` and `import { FitAddon } from '@xterm/addon-fit'`
- `import '@xterm/xterm/css/xterm.css'` in `src/main.js`
- xterm.js Terminal instance with dark/light theme palettes
- FitAddon for auto-sizing
- xterm container `<div class="xterm-wrap" bind:this={termEl}>` replacing the `.ansi-output` div

**Content write strategy (avoids flicker):**
```javascript
// Single write: hide cursor → clear → content → position cursor → show cursor
const pad = '\n'.repeat(cursor.h - 1 - cursor.y);  // pad trailing empty rows
term.write('\x1b[?25l');  // hide cursor during rewrite
term.reset();
term.write(content + pad + `\x1b[${cursor.y + 1};${cursor.x + 1}H\x1b[?25h`, () => {
  if (!atBottom) term.scrollToLine(Math.min(prevViewport, term.buffer.active.baseY));
});
```

**Cursor-only updates** (no content change): just send `\x1b[${y+1};${x+1}H`

**Resize flow:**
1. FitAddon.fit() calculates optimal cols/rows for container
2. Call `resizePane(target, cols, rows)` (already in ws.js)
3. Server resizes tmux pane → next capture matches xterm dimensions
4. Resize on: initial mount, window resize, visualViewport resize, tab switch

**Touch scrolling fix (KEY DIFFERENCE from old impl):**
- Do NOT manually handle touchstart/touchmove
- Let xterm.js handle scroll natively via `.xterm-viewport`
- Add CSS `overscroll-behavior: contain` on the xterm container
- Set `disableStdin: true` so terminal doesn't consume touch events

**Theme sync:**
- Dark/light theme objects (same as old impl)
- MutationObserver on `data-theme` → update `term.options.theme`

**Scroll behavior:**
- `term.onScroll()` → update `termAtBottom`
- Scroll-to-bottom button calls `term.scrollToBottom()`
- Auto-scroll on content update when user was at bottom

### 4. Keep unchanged
- `paneContent` state (still set from pane_output for ChatView/parsers)
- ChatView component and its props
- All input handling (handleSubmit, handleKeydown, direct mode, shortcuts)
- Window switcher
- viewMode switching (terminal/chat)

### 5. xterm.js Terminal config
```javascript
{
  cursorBlink: true,
  cursorStyle: 'bar',
  disableStdin: true,
  fontSize: 14,
  fontFamily: "'SF Mono', Menlo, 'Courier New', monospace",
  theme: getTermTheme(),
  scrollback: 500,
  convertEol: true,
  allowTransparency: false,
}
```

### 6. CSS changes
```css
.xterm-wrap {
  height: 100%;
  overscroll-behavior: contain;
}
/* Ensure xterm viewport allows native touch scrolling */
.xterm-wrap :global(.xterm-viewport) {
  overscroll-behavior: contain;
  -webkit-overflow-scrolling: touch;
}
```

## Files changed
- `package.json` — add @xterm/xterm, @xterm/addon-fit; remove ansi-to-html
- `src/main.js` — add xterm.css import
- `src-tauri/src/server.rs` — simplify cursor to raw x/y/w/h
- `src/lib/Terminal.svelte` — major rewrite of rendering, keep all input/chat logic
