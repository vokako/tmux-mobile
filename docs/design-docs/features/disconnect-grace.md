# Disconnect Grace Period for Window Resize Restore

## Context
`resize_pane` sets a tmux window to the client's viewport size. Since tmux
picks the smallest size across all attached clients for a shared window,
the server used to restore windows back to auto-size (`resize-window -A`)
the moment the client disconnected, so that a real terminal client attaching
the same session would not be stuck at 80×24 of a phone.

This fired on *every* disconnect, including:

- App backgrounded (iOS suspends the WebSocket in ~10-30 s)
- Cellular/Wi-Fi handover (network blip)
- App killed and reopened by the user

The observable symptom: reopening the app takes noticeably long before the
pane settles. The reason is a **double reflow**:

1. Disconnect → server runs `resize -A` → tmux reshapes the window to
   whatever the remaining clients (or defaults) want → every AI CLI in
   that pane redraws against the new dimensions.
2. Reconnect → client sends `resize_pane(cols, rows)` → tmux reshapes
   back to the phone size → second redraw.

On long scrollback (Kiro/Claude transcripts with tables, code blocks, long
responses) the two reflows can take multiple seconds of visible churn.

## Decision
Defer the restore with a configurable grace period. On disconnect, schedule
`resize -A` to run after `disconnect_grace_secs`. If the same (or any)
connection resizes the window again before the timer fires, cancel it —
the window is still being actively driven and should stay at its current
size.

Default: **600 s (10 min)**. Long enough to cover app-backgrounded + phone
locked + user returns after a coffee break. Short enough that a real desktop
tmux client attaching after a forgotten session still gets a reasonable
window size within minutes.

`disconnect_grace_secs = 0` preserves the legacy behavior (immediate
restore, no timer).

## Architecture

```
ResizeTrackerInner {
    per_conn  : HashMap<conn_id, HashSet<window>>
    per_window: HashMap<window,  WindowResizeState { active_conns, pending_restore }>
}
```

Two indexes, one purpose each:

- `per_conn` — so disconnect can iterate "which windows did this conn touch"
  without scanning every window.
- `per_window` — aggregate state shared across connections. `active_conns`
  is a refcount of still-connected clients that resized this window;
  `pending_restore` holds the `JoinHandle` of a running grace-timer task,
  or `None` if no restore is scheduled.

### Event flow

**`resize_pane(target, cols, rows)` (conn `C` on window `W`):**
1. `tmux resize-pane`
2. If `(C, W)` is a new pair in `per_conn`, increment `per_window[W].active_conns`.
3. If `per_window[W].pending_restore` is `Some`, `.abort()` it and clear.

**Connection `C` disconnects:**
1. Remove `C` from `per_conn`, get its window set.
2. For each window `W`:
   - Decrement `per_window[W].active_conns`.
   - If still > 0 → skip (another client is holding the size).
   - Else spawn a timer task:
     ```
     sleep(grace_secs)
     reconfirm active_conns == 0  // someone may have reconnected
     run resize -A in spawn_blocking
     remove per_window[W]
     ```
     Store its `JoinHandle` in `per_window[W].pending_restore`.

### Abort vs. re-check
`JoinHandle::abort()` cancels the task at its next `.await` point. If a
reconnect lands after the task has already passed the `sleep` but before
it executes the tmux call, the abort fires too late — so the task also
re-reads `active_conns` under the lock before calling `resize -A`. Belt
and suspenders.

### Lock discipline
`ResizeTracker` uses `std::sync::Mutex`. `MutexGuard` is `!Send`, so
holding it across an `await` breaks the spawned-future `Send` bound. The
disconnect cleanup works in two phases:

1. **Under the lock** — compute a `Decision` (Skip / RestoreNow / Scheduled),
   including synchronously spawning the grace-timer task and storing its
   handle in `pending_restore`. `tokio::spawn` is synchronous (returns
   immediately), so no await happens inside the lock.
2. **After the lock releases** — `match decision` handles the awaits
   (`RestoreNow` does a `spawn_blocking(resize -A).await`).

## Alternatives Considered

**Per-connection map only, with no per-window aggregation.** Original
design. Breaks when two clients share a window: whichever drops first
triggers `resize -A` even though the other is still attached and wants
the window at its current size. Per-window refcount fixes this as a
side benefit.

**Checking tmux `list-clients` on disconnect to decide whether to restore.**
Would give perfect semantics ("nobody's watching → restore"). But tmux
clients are per-session, not per-window, and the mapping to "would a
remaining tmux-mobile client see this pane wrong" is gnarly. The grace
timer sidesteps this: we just wait, and if someone reconnects and resizes,
we don't restore.

**Server-side capture cache keyed by target.** Would also help reconnect
speed (skip the 200 ms wait for first pane_output). Small win compared
to eliminating the reflow; can revisit if needed.

## Configuration
- `disconnect_grace_secs` in `~/.config/tmux-mobile/config.toml`
- Env var `DISCONNECT_GRACE_SECS` overrides config
- Default 600 (10 min); set to `0` for legacy immediate restore
