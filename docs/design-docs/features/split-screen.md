# Desktop Split-Screen

## Context
On a phone there's only room for one tmux pane, but on the desktop app and wide
browser windows the screen can hold several. Split-screen tiles 2/3/4/6
independent terminal cells, each bound to any `session:window.pane`, so the user
can watch multiple agents / shells at once like a tiling terminal. Mobile and
narrow windows keep the single-pane behavior unchanged.

## Decision
Compose multiple existing `Terminal` instances in a CSS grid (`SplitView.svelte`)
rather than building a new multi-pane renderer. The server already multiplexes
subscriptions; the only client change required was making pane-output delivery
multi-listener.

## How It Works
- **ws.js per-target registry** (the enabling change): `pane_output` / `pane_closed`
  are routed to a `Map<target, cb>` keyed listener instead of one shared
  callback. Each cell's `Terminal` registers its own target. See
  `websocket-client.md`.
- **Server**: no change. `Subscriptions` is a `HashMap<String,String>` and
  `subscription_loop` already pushes per subscribed target; N cells = N
  `subscribe` calls on one connection.
- **`SplitView.svelte`**: CSS grid (`.layout-{2,3,4,6}`), one `.cell` per entry,
  keyed by a stable `cell.id` so re-assigning a cell's target rebuilds only that
  Terminal. Each cell has a header (an `AgentChip` label + a pane picker popover
  populated by `listSessionsWithPanes()` + a close button) over the Terminal
  body. `min-width:0; min-height:0` on cell/body is required so grid children
  shrink and each xterm's `ResizeObserver` sees its real box.
- **Focus / active cell**: clicking a cell sets `activeCellId` (visual border).
  Input routing is automatic — each Terminal has its own hidden xterm textarea;
  only the focused one receives keys. `activeCellId` is purely the highlight.
- **`App.svelte` state**: `splitLayout` (1 = single, else 2/3/4/6),
  `splitCells [{id,target,session,command}]`, `activeCellId`. The pre-existing
  single `terminalTarget` stays the source of truth for the Files page, nav
  pills, narrow-screen fallback, and persistence; the active cell mirrors into
  it via `assignCell`.
- **Gating**: `splitEligible = !isTouchDevice && innerWidth >= 900` (a
  `window.resize` effect keeps it live). Narrowing below 900 px collapses to the
  single Terminal on cell 0's pane; widening restores. Mobile never mounts
  SplitView.
- **Layout toolbar**: a thin row at the top of the terminal page (`1·2·3·4·6`),
  rendered only when `splitEligible`. Reuses the surface/pill visual language.
- **Persistence**: `splitLayout` + `splitCells` ride in `tmux_state`. Restore is
  re-gated on `splitEligible`, so a desktop-saved layout degrades to single-pane
  on a phone.
- **Reconnect**: `resubscribeAll()` subscribes every populated cell (or the
  single target). Each mounted Terminal also re-subscribes itself via the
  `ws-reconnected` event.

## Interactions
- **fontSize**: one global prop → every cell re-fits; the cmd/ctrl +/- handler is
  unchanged.
- **Chat**: split cells are always `viewMode="terminal"`; chat stays single-pane.
- **Files**: unchanged; overlays the whole terminal page and keys off the single
  `terminalSession`.

## Trade-offs
- N terminals = N subscriptions + N xterm instances; each runs its own 200 ms
  snapshot stream. Acceptable on desktop; gated off mobile.
- Cell 0 mirrors `terminalTarget` so the narrow-screen fallback shows the
  expected pane; reassigning the active cell updates the mirror.

## Verification
See the plan's verification section: build (`npm run build`), run
`npm run tauri:dev`, then check single-pane regression, two panes streaming at
once, per-cell input/resize, close, pane death, reconnect, the narrow gate, and
persistence. Backend multi-subscribe + single-pane routing were verified against
the live server over one WS connection.
