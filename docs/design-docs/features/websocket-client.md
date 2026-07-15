# WebSocket Client Robustness

## Context
Mobile connections are unreliable. WebSocket client must handle disconnects, reconnects, and edge cases gracefully.

## Decision
Custom WebSocket client (`ws.js`) with auto-reconnect, pending promise cleanup, and multi-address failover.

## How It Works
- **Pane-output routing is a per-target listener registry**, not a single
  callback. `addPaneOutputListener(target, cb)` / `removePaneOutputListener(target)`
  (and the `…ClosedListener` pair) keep a `Map<target, cb>`; the `pane_output`
  / `pane_closed` dispatch in `onmessage` routes each push to the listener for
  that exact `target`. This is what lets desktop split-screen mount several
  `Terminal` instances on one connection — each registers its own target and
  they no longer overwrite a shared slot. The single-pane path is the
  degenerate case (one listener for one target). See
  `docs/design-docs/features/split-screen.md`.
- `connect()` cleans up existing connection before creating new one
- `onclose` rejects all pending RPC promises (prevents caller hangs); each
  rejection carries a `reason` (e.g. `'connection lost'`,
  `'heartbeat: …'`, `'superseded by new connect'`) and an
  `err.code = 'DISCONNECTED'` so callers can distinguish cleanly.
- **Liveness is entirely at the WebSocket protocol layer now.** The server
  sends `Message::Ping` every 15 s; browsers auto-reply with `Message::Pong`
  at the WS layer without executing any application code. The server
  tracks `last_pong_at`; if it hasn't seen a pong in 45 s it tears down
  the connection. On the client that shows up as `ws.onclose` → reject
  pending RPCs → trigger reconnect UI. This removes all application-layer
  heartbeat logic from ws.js (previously ~40 lines of `lastRxAt`,
  `HEARTBEAT_QUIET_MS`, `pingInFlight`, etc. all gone). The key property
  it gets us: a 50 MB `fs_download` frame can be in flight for tens of
  seconds and the keepalive is completely unaffected — PING/PONG goes
  through the WS framing layer, not through our JSON-RPC mutex.
- RPC timeout is 6 s by default; long-running methods (`fs_download`,
  `fs_upload`) pass `60_000` at the call site.
- **3-consecutive-RPC-timeout disconnect is gated on `pending.size === 0`
  AND inbound silence ≥ 10 s.** Two separate "link is actually alive"
  signals suppress the breaker:
  1. If a long RPC is still in flight, its single huge response frame is
     almost certainly what's delaying the short polling RPCs behind it on
     the shared WS send mutex — let the pollers fail individually.
  2. If any inbound message (pane_output push, RPC reply, handshake)
     arrived within the last 10 s (`TIMEOUT_DISCONNECT_INBOUND_SILENCE_MS`),
     the link is alive but slow — common on high-RTT cellular where 5–7 s
     round trips make 6 s RPC timeouts fire while server pushes keep
     arriving. Tearing down + re-handshaking on such a link makes things
     strictly worse. In this case the timeout counter resets to 0.
  When nothing is pending and inbound has been silent past the threshold,
  the disconnect fires as before.
- Auto-reconnect with exponential backoff
- **Transport loss is reported even when no close event arrives.** Once a socket
  has authenticated, the first RPC that finds no current OPEN socket requests
  recovery through the same one-shot disconnect callback as `onclose`.
  Repeated polling and keypresses cannot start duplicate reconnect loops; a
  deliberate `disconnect()` disables recovery before closing.
- **Socket identity owns every callback, cipher, and asynchronous send.** A
  replaced socket's late close/message handler is ignored, AES-GCM counters
  live on that socket instead of in module-global state, and all encrypted RPC
  and subscription sends share one per-socket promise queue so nonce order and
  wire-frame order cannot diverge. Encryption that finishes after replacement
  cannot send through the new socket. This prevents an old connection from
  clearing or corrupting a successful in-app reconnect — the failure mode that
  previously required restarting the whole app process.
- **Foreground recovery also handles an apparently-open socket.** Mobile
  WebViews can suspend with `WebSocket.readyState === OPEN`, then resume after
  pane pushes or xterm's live-tail state has gone stale. On every transition
  back to `visible`, the app idempotently re-sends all active subscriptions;
  each mounted Terminal also resets transient touch state, re-fits/repaints,
  and pulls one `capture_pane` snapshot. If it was following the tail before
  suspension it resumes following the tail; deliberate scrollback remains
  pinned and only receives the new-output indicator. This recovery applies to
  Android, browser/PWA, and desktop WebViews through `visibilitychange`.
- Multi-address failover: server `machine_id` tracks alternate addresses
- Optional E2E encryption layer
- `JSON.parse` wrapped in try-catch in `onmessage`
- Optional chaining on server push params (`data.params?.target`)

## Alternatives Considered
- **Socket.IO**: Rejected — adds dependency, WebSocket is sufficient for JSON-RPC
- **No auto-reconnect**: Rejected — mobile connections drop frequently
- **Application-layer ping (JSON-RPC `ping` method)**: used until the
  sent-by-server WS PING migration. Problem was that the ping *response*
  shared the send mutex with every other outbound frame, so a 50 MB
  fs_download could block the pong response behind tens of seconds of
  data even on a healthy link. We iterated twice on patches
  (`heavyRpcInFlight` counter, then `lastRxAt`) before realizing the
  real fix was to move liveness out of the application layer entirely.

## Trade-offs
- Custom reconnect logic to maintain
- Pending promise rejection can cause UI flicker if not handled in components
- Server-initiated keepalive means the server is slightly more complex;
  the ping_task must coexist with the send_task + receiver loop and
  share the same `out_tx` to avoid fighting for `ws_sender` (see
  `concurrent-ws-rpc.md` for the full task layout).

## Lessons Learned
- Always reject pending promises on disconnect — otherwise callers hang forever
- Manual disconnect MUST cancel reconnect timers — otherwise zombie reconnects fire
- Wrap `JSON.parse` in try-catch — malformed messages crash the handler
- Use optional chaining on server push params — malformed messages can have missing params
- Application-layer heartbeats look clean until they start fighting with
  real application traffic on a shared transport. The first iteration
  wrapped specific heavy RPCs; the second monitored last-received-at;
  both worked *most* of the time but both were working against the
  WS layer instead of with it. WS PING/PONG is the first-principles
  answer: it's exactly what the protocol designers gave us for this,
  and it runs at a layer that is unaffected by our JSON-RPC queueing.
- Client-side polling RPCs such as Terminal's `list_panes` are still plain
  `call()`s and can
  independently trigger the "3 consecutive timeouts" disconnect rule.
  On WAN a 50 MB download response monopolizes the send mutex long
  enough that 3 pollers time out before the download frame finishes —
  users saw the transfer abort "disconnected" halfway through. Fix:
  gate that disconnect on `pending.size === 0` so pollers failing
  behind a long RPC don't kill the connection. When nothing is pending
  and the link still silently fails, the disconnect still fires.
