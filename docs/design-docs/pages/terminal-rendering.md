# Terminal Rendering — frames, tail, and key encoding

How a pane's frames reach xterm.js and when they are allowed NOT to: hidden terminals record instead of render, local input snaps back to the live tail, and the two tmux key-encoding rules every send path must respect. Touch is in `terminal-touch.md` / `terminal-gestures.md`; sizing in `terminal-sizing.md`.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### A hidden terminal records frames, it does not render them

page layers stay mounted (`visibility: hidden`) so state survives tab switches, which meant every busy pane kept running the full frame pipeline (whole-snapshot ANSI color adaptation + full-screen xterm write + WebGL draw) at the server's 5 fps cadence, invisibly, per pane and per split cell — CPU as heat for nothing (owner, 2026-08-22: "发热挺厉害"). `Terminal` takes a `visible` prop (App passes `page === 'terminal'`, SplitView forwards it per cell, the Hub drawer passes the Hub's own `visible`); while false, `onPaneOutputCb` only updates `lastContent`, and a repaint-on-show effect replays the newest frame (tail follows, scrollback readers get the new-output pill). The replay is REQUIRED, not polish: the server skips frames identical to the last one it sent, so a dropped frame is never re-sent. `lastContent`/`lastCursor` are component-scoped plain lets (reset per target) so that effect can reach them without re-running on every frame.

### Local input returns to the live tail

every send path calls `resumeLiveTail()` — drop the selection, stop momentum, unpin `touchScrolling`, snap to bottom, repaint from `lastContent`. Without it a suppressed render never recovers: the server skips frames whose state equals the last one it *sent*, so it never re-sends the one the client dropped, and the typed characters only appear when an unrelated repaint (resize / visibility / pane switch) happens. `unlockKeyboard()` must never `clearTimeout(endTouchScrollTimer)` — that timer is the only pending reset of `touchScrolling`.

### Ctrl keys must be tmux named keys

with `extended-keys on`, tmux DROPS raw C0 bytes (`send-keys -l $'\x03'`) sent to panes in extended key mode (`#{pane_key_mode}`=`Ext` — every modern agent TUI). `tmux::send_keys` literal mode therefore splits C0 bytes into named keys (`C-c`, `M-C-x`); don't bypass it.

### xterm DA filtering

Filter device attribute responses before forwarding to tmux.
