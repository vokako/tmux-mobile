# Terminal Keyboard — open/close, overlay vs resize, and the input pipeline

The soft keyboard on the phone, the `.keep-rows` overlay for agent TUIs, and why printable keys bypass xterm's keydown path. Gesture state lives in `terminal-gestures.md`; the cols×rows math in `terminal-sizing.md`.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### Terminal keyboard

Double-tap to open (NOT single-tap). `kbLocked` flag + `inputmode` attribute. `endTouchScroll` must NEVER change `kbLocked` (race condition with delayed timers). `kbLocked` has exactly two writers: `unlockKeyboard()` opens and `lockKeyboard()` closes; the sanctioned lock sites — pane switch (the lifecycle effect), the blur timer, the keyboard-shift open→close transition, and the close half of the keyboard toggle — all call `lockKeyboard()`, each labelled at the call site, so `grep lockKeyboard()` is the complete list. `Terminal.source.test.ts` pins the two writers and the four labelled callers (the toggle's close half was an unlisted fifth direct write until 2026-09-03).

### Keyboard is an OVERLAY for agent TUIs, a resize for everything else

opening the keyboard used to shrink the box → change cols×rows → `resize_pane` → tmux resize → the agent redraws its whole conversation (seconds), twice per toggle. Now `.keep-rows` (`isMobile && detectAgent(command)`, i.e. the shared `AGENTS` table) pins the xterm host to `--kb-locked-h` and bottom-anchors it under `html.keyboard-open`, so the observed box never changes and the ResizeObserver — still the ONE re-fit trigger — has nothing to report. `--kb-locked-h` is captured in `doResize` only while the keyboard is DOWN. `vim` and friends keep resizing: they repaint cheaply and need the real visible size. Verified: a kiro pane held `151x27` across open+close, a zsh pane went `27 → 8 → 27`. See `docs/design-docs/pages/terminal-sizing.md`.

### Printable keys bypass xterm's keydown

unmodified printable keydowns return `false` from `attachCustomKeyEventHandler` and flow through the textarea input pipeline; a CAPTURE-phase `input` listener on termEl claims non-composition `insertText` events (`stopImmediatePropagation`) and forwards them. It must claim, not just forward: xterm v6 handles `insertText` itself when no keydown preceded it (`!e.composed || !_keyDownSeen`) — exactly the no-keydown commits WKWebView IMEs produce for CJK punctuation — so two live handlers sent those characters TWICE. Reason for the bypass: CJK IMEs convert punctuation (`,`→`，`) at the input stage with NO composition events — xterm's keydown fast path would emit raw ASCII and preventDefault, killing the conversion. Applies to ALL platforms; composition stays with xterm. Paste is detected via a CAPTURE-phase `paste` listener (xterm's own paste handler fires onData synchronously before same-phase listeners) and routed to the `paste_text` RPC — tmux `paste-buffer -p` adds bracketed-paste markers iff the pane app enabled `?2004`, so multi-line pastes don't execute line by line.

### Auto-pair textarea force-clear

(all platforms, was mobile-only): Force-clear xterm's hidden textarea after keyboard input (NOT paste, NOT mid-IME-composition). Use `paste` event flag to distinguish — NEVER use `data.length` (auto-paired `""` `()` have length 2, gets misclassified as paste). Composition needs TWO signals: `compositionstart/end` listeners AND per-event `insertCompositionText` inputType — some Android IMEs (Samsung/pad suggestion-bar keyboards) compose without ever firing compositionstart. `compositionend` must reset BOTH flags: Chromium commits as input(insertCompositionText) → compositionend with no trailing input event, so a sticky per-event flag would permanently suppress the clear for standard IMEs (GBoard).

### Escape is claimed in capture; there is NO focus guard around it

A bare Escape is claimed by the hardware-capture handler like the Ctrl/Alt combos and encoded by hand (`\x1b`), so the send never depends on whose keydown runs first; an Escape mid-IME-composition stays with the composition. Board #20's "Esc 让当前框失去焦点" on the desktop (2026-08-26 … 09-03) went through three rounds of DOM blur guards and one native (`objc2` `cancelOperation:`) patch before the owner traced it to a browser EXTENSION on their machine blurring the focused input on Esc, before the page ever sees the key. Everything but the claim was removed (owner: "避免我们过度修复了"), and `Terminal.source.test.ts` pins the absence. If Esc drops focus again: check the browser's extensions first, then the debug panel's `kb(hardware): passthrough Escape` line — no line means the key never reached the page.
