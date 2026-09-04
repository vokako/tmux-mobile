# Terminal Rendering — frames, tail, and key encoding

How a pane's frames reach xterm.js and when they are allowed NOT to: hidden terminals record instead of render, local input snaps back to the live tail, and the two tmux key-encoding rules every send path must respect. Touch is in `terminal-touch.md` / `terminal-gestures.md`; sizing in `terminal-sizing.md`.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### A hidden terminal records frames, it does not render them

page layers stay mounted (`visibility: hidden`) so state survives tab switches, which meant every busy pane kept running the full frame pipeline (whole-snapshot ANSI color adaptation + full-screen xterm write + WebGL draw) at the server's 5 fps cadence, invisibly, per pane and per split cell — CPU as heat for nothing (owner, 2026-08-22: "发热挺厉害"). `Terminal` takes a `visible` prop (App passes `page === 'terminal'`, SplitView forwards it per cell, the Hub drawer passes the Hub's own `visible`); while false, `onPaneOutputCb` only updates `lastContent`, and a repaint-on-show effect replays the newest frame (tail follows, scrollback readers get the new-output pill). The replay is REQUIRED, not polish: the server skips frames identical to the last one it sent, so a dropped frame is never re-sent. `lastContent`/`lastCursor` are component-scoped plain lets (reset per target) so that effect can reach them without re-running on every frame.

### A font, theme or `active` change is a live option update, never a rebuild

`Terminal.svelte` has ONE lifecycle effect — dispose the old xterm, construct the new one, resubscribe, `capture_pane`, WebGL init, `kbLocked = true` — and its only dependency is `target`. The effect reads `target` and then runs its whole body under `untrack`, so the font size and family, the line height, the theme and `active` it reads synchronously while constructing cannot re-trigger it. Those changes belong to two small effects that write `term.options` on the running instance. Because `term` is a plain let (it is read in hundreds of places and must not make every effect that touches it reactive), those effects wait on `termGen` — a `$state` counter bumped once per build, after the instance is complete — and read their reactive inputs BEFORE the `!term` guard. Reason (review, 2026-09-03): both live effects used to start with `if (!term) return`, ran once at mount before the terminal existed, tracked nothing and never ran again, while the lifecycle effect tracked everything — so a system light/dark auto-switch mid-sentence on the phone rebuilt the terminal and dropped the keyboard, and in a desktop split a click that flipped `active` on two cells rebuilt both. `Terminal.source.test.ts` pins the shape.

### Local input returns to the live tail

every send path calls `resumeLiveTail()` — drop the selection, stop momentum, unpin `touchScrolling`, snap to bottom, repaint from `lastContent`. Without it a suppressed render never recovers: the server skips frames whose state equals the last one it *sent*, so it never re-sends the one the client dropped, and the typed characters only appear when an unrelated repaint (resize / visibility / pane switch) happens. `unlockKeyboard()` must never `clearTimeout(endTouchScrollTimer)` — that timer is the only pending reset of `touchScrolling`.

### Ctrl keys must be tmux named keys

with `extended-keys on`, tmux DROPS raw C0 bytes (`send-keys -l $'\x03'`) sent to panes in extended key mode (`#{pane_key_mode}`=`Ext` — every modern agent TUI). `tmux::send_keys` literal mode therefore splits C0 bytes into named keys (`C-c`, `M-C-x`); don't bypass it.

### xterm DA filtering

Filter device attribute responses before forwarding to tmux.

### Motion: the terminal's box never animates, only the chrome around it

[motion.md](../features/motion.md) principle 9. `.term-wrap`, `.xterm-wrap` and a split `.cell`'s body carry no transform, transition or animation — a resting transform makes them a containing block (every fixed popover breaks) and a transitioning size makes the fit run on a mid-flight measurement; the retired `.xterm-wrap { transition: margin-top }` had one writer and it set 0. What moves is the chrome: the toast, the selection handles and toolbar and the expanded chip strip fade in (`.appear`; their positions stay inline and instant), the to-tail button pops (app.css gives `.to-tail` its `pop-in`), the shortcut pills cross-fade colour on `--t-fast` with `transform` kept OUT of the list so the press `translateY(1px)` lands under the finger, and the window chips are keyed by window index with `animate:flip` on `moveMs()` so a renumbering moves the chip instead of repainting it. The collapse control is still a cut: the expanded bar and the collapsed chip are two different elements in two `{#if}` branches (pinned by `Terminal.source.test.ts`), so there is no one glyph to turn. A split cell's frame (`SplitView .cell`) cross-fades only its border colour and ring — colour is neither a containing block nor a size.

Hover / unfold / highlight (#86, 2026-09-04): resting on a window chip opens the ONE hover card (`use:hoverInfo` on the `.win-chip` wrapper — `index:name`, command, pane count, agent tag; the chip is passed `title={null}`, which AgentChip honours as "no native title", so the card is not doubled), and the to-tail button's card is a single note ("Back to the newest output"). Nothing unfolds and no highlight travels here: the chip strip scrolls and its chips are AgentChips, so the active chip is not a `.slide-pill` candidate, and the terminal's box stays untouched (principle 9 — the card is its own fixed layer and the action only adds listeners).
