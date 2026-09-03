# WebSocket Client Robustness

## Context
Mobile connections are unreliable. WebSocket client must handle disconnects, reconnects, and edge cases gracefully.

## Decision
Custom WebSocket client (`ws.ts`) with auto-reconnect, pending promise cleanup, and multi-address failover.

## How It Works
- **Browser development has one public origin.** `npm run dev:all` exposes
  Vite on `:5173`; Vite proxies WebSocket upgrades at `/ws` and streaming
  downloads at `/dl` to the watched Rust service bound to loopback. A fresh
  dev browser defaults its connection field to `ws(s)://location.host/ws`.
  `httpOriginForWs()` deliberately discards the trailing `/ws` segment when it
  builds signed download URLs, so downloads use the sibling `/dl` route while
  a production parent prefix such as `/tmux` remains intact. `dev:all`
  explicitly disables TLS on the loopback hop even when the persisted server
  config has certificates; HTTPS/WSS terminates at the public Vite/Tailscale
  edge. The internal Rust port remains available for the separate-start
  compatibility workflow.
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
  heartbeat logic from ws.ts (previously ~40 lines of `lastRxAt`,
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
- **One reconnect loop at a time, identified by generation.** `start()` is a
  no-op while a loop runs (it returns `false`); `cancel()` ends the loop, and
  every asynchronous continuation — probe, connect, retry timer, watchdog —
  carries the generation it started under and is discarded when it is not the
  current one. A boolean `reconnecting` could not tell "cancelled" from
  "cancelled and restarted", so a superseded chain's late `connection timeout`
  used to continue the new loop's counter and `noteAddressUnreachable` a
  reachable address for two minutes (review, 2026-09-03). The disconnect
  callback, the foreground check and a failed address switch may all call
  `start()` during one outage; a typed address (`onAddress`) calls `cancel()`
  first because it is a NEW intent. `reconnect.test.ts` pins both.
- **Socket identity owns every callback, cipher, and asynchronous send.** A
  replaced socket's late close/message handler is ignored, AES-GCM counters
  live on that socket instead of in module-global state, and all encrypted RPC
  and subscription sends share one per-socket promise queue so nonce order and
  wire-frame order cannot diverge. Encryption that finishes after replacement
  cannot send through the new socket. This prevents an old connection from
  clearing or corrupting a successful in-app reconnect — the failure mode that
  previously required restarting the whole app process.
- **Inbound frames are dispatched in wire order.** Decrypt + decode is async
  and its latency depends on the frame (a ≥256-byte payload runs through
  `DecompressionStream`, a small one through a synchronous `TextDecoder`), so
  two handlers started back to back could finish in the other order — for
  `pane_output` that painted snapshot N after N+1 and, since the server only
  pushes on change, left the stale screen up. `_recvQueue`, the mirror of
  `_sendQueue`, chains every frame's handler behind the previous one's.
- **A decrypt failure is a disconnect, now.** The receive counter advances
  before the frame is checked, so after one bad frame every later frame fails
  too; the session is dead. Waiting for the idle probe's three timeouts meant
  ~20 s of a silently frozen app before reconnect started. `forceDisconnect`
  runs from the catch instead.
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
- Optional E2E encryption layer. **One key per direction (v2):** the server
  advertises `e2e: 2` in its nonce frame, the client asks for it and derives a
  proof key, a send key and a receive key from the token and both nonces with
  three HKDF labels. Under v1 one key did all three jobs and both directions
  counted nonces from 0, so client frame #n and server frame #n were sealed under
  the same (key, nonce) — the AES-GCM nonce-reuse failure. A server that does not
  advertise `e2e` still gets v1, so nothing older is locked out. Derivations on
  both sides are pinned to shared vectors in `ws.test.ts` and `wire.rs`.
- `JSON.parse` wrapped in try-catch in `onmessage`
- Optional chaining on server push params (`data.params?.target`)

## Multi-Server (board #55)

`src/lib/app/servers.ts` is the named-server registry over the same keys this
doc already describes. Design decisions, in the order they bit:

- **The old keys stay the ACTIVE MIRROR.** `tmux_address`/`tmux_token`/
  `tmux_socket` keep meaning "the server we are on": ws.ts, the reconnect
  machine, deep links and the boot auto-connect read them unchanged, and a
  downgraded client sees the single-server world it expects. The registry
  (`tmux_servers` + `tmux_server_current`) only remembers what else exists.
- **Identity is the MACHINE, not the address** (lead review). `tmux_machines`
  (machineId → addresses) is the existing failover authority; an entry
  persists `machineId` and upsert merges on it first, so a reconnect over
  Tailscale updates the LAN entry instead of minting a second "server".
  Migration attributes history addresses through the same map: the current
  machine's alternates never re-materialize as entries, an unseen machine
  yields one entry, unattributed addresses dedupe by address.
- **Switching is storage writes + reload** (`applySwitch`): park the leaving
  server's `tmux_state`/`tmux_machine_id` under `tmux_state::<id>` /
  `tmux_machine_id::<id>`, restore the target's pair (or clear — a first
  visit must not inherit A's terminal targets), point the mirror keys, clear
  `tmux_disconnected`, reload. The boot path is the ONE way up against a
  server, so the Hub room cache, mounted terminals, Files parked cwds and
  Team state reset by construction instead of by per-component sweeps. The
  caller cancels the reconnect machine and closes the socket FIRST — a live
  retry loop re-reads `tmux_address` and would race the writes.
- **An address switch shows on the row that was tapped** (review, 2026-09-03):
  App keeps `pendingAddress` from the moment `onAddress` dials until the
  connect settles either way (`.finally`, guarded so a second tap's pending
  is not cleared by the first's outcome), and Preferences' address list gives
  every row a dot in the ONE status language — `--status-sleep` at rest, accent
  on the current address, accent + app.css `.live-dot` on the one still dialing
  (`aria-busy`, title "Connecting…"). Before, the old row stayed lit while the
  socket was rebuilt and nothing said the tap had landed. A failed direct
  connect hands over to the reconnect machine, whose bar carries on from there.
- **The rail entry is a control, not a page**: it rides the RAIL_GAP branch
  (above the configure group — "右下角agent上边"), carries no rail slot, and
  its popover follows the app's one menu recipe (fixed layer, measured then
  shown, outside-pointerdown/Escape/resize dismissal). The Settings connect
  form IS the add flow; the `+` row only navigates there.

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
- EVERY path that produces a fresh server connection must call
  `resubscribeActive()` (and dispatch `ws-reconnected`): the server's
  subscription table is per-connection state, but the client's
  `subRefcount` survives the disconnect because Terminals stay mounted
  in hidden page-layers. A refcount that is already > 0 means later
  `subscribe()` calls never re-send the wire message — the terminal
  freezes on its last snapshot while `send_keys` (a plain RPC) keeps
  working, which users report as "typing is invisible but sending
  works". The reconnect machine, address switch, and optimizer paths
  all had it; the manual connect from Settings (`onConnected`) was the
  one that didn't — found only because each path wires its own
  post-connect sequence by hand. Symptom → suspect list: frozen display
  + working input = wire-subscription/refcount divergence first.
- Client-side polling RPCs such as Terminal's `list_panes` are still plain
  `call()`s and can
  independently trigger the "3 consecutive timeouts" disconnect rule.
  On WAN a 50 MB download response monopolizes the send mutex long
  enough that 3 pollers time out before the download frame finishes —
  users saw the transfer abort "disconnected" halfway through. Fix:
  gate that disconnect on `pending.size === 0` so pollers failing
  behind a long RPC don't kill the connection. When nothing is pending
  and the link still silently fails, the disconnect still fires.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### WebSocket lifecycle

`connect()` cleans up existing. `onclose` rejects pending. `doDisconnect()` clears timers. Heartbeat ping every 15s; 2 consecutive RPC timeouts → auto-close → reconnect.

### Servers are a named registry; the machine is the identity

(board #55): `src/lib/app/servers.ts` keeps `tmux_servers` (+`tmux_server_current`) while the old `tmux_address`/`tmux_token`/`tmux_socket` stay the ACTIVE MIRROR every existing reader keeps reading. One machine = one entry however many LAN/Tailscale/WAN addresses it answers on — `recordServer` merges by `machineId` (learned at connect) without moving CURRENT, migration attributes `tmux_address_history` through `tmux_machines`, and the same-machine failover semantics are untouched. A different-machine successful connect goes through `activateConnected`: park the old live state/machine id before surfacing the target and reload; a same-machine alternate records in place. Switching (`applySwitch` + reload) parks/restores per-server `tmux_state`/`tmux_machine_id` under `::<id>` keys so restore targets never cross servers, and reuses the boot path so no in-memory cache (Hub rooms, terminals, Files cwds) can leak across; the caller cancels the reconnect machine and drops the socket first. The desktop rail's switcher rides the RAIL_GAP branch above the configure group — a control (no drag slot), popover in the one menu recipe; the Settings form is the add flow and activates only after authentication. Forgetting a non-current server captures its row identity and asks through the shared `ConfirmDialog` before removing config + parked state.
