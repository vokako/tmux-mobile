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
- **One entry per address and per machine** (board 318, owner 2026-10-08: "为什么这里有两个一样的地址，但是显示成两个server了"). `recordServer` used to match the machine id first and never look further, so a machine-less entry with the same address (migrated from the address history, or recorded before the id was known) stayed as a ghost twin forever. Now every record absorbs twins (same address, or same machine id) into the resolved entry, and `migrateServers` runs `repairServers` on every boot to heal old lists once (idempotent; a healthy list is not rewritten). The survivor keeps its id, machine, address and token, takes a twin's name only if its own is a default and the twin's was typed, inherits CURRENT if the twin held it, and the twin's parked keys are removed.
- **Identity is the MACHINE, not the address** (lead review). `tmux_machines`
  (machineId → addresses) is the existing failover authority; an entry
  persists `machineId` and upsert merges on it first, so a reconnect over
  Tailscale updates the LAN entry instead of minting a second "server".
  Migration attributes history addresses through the same map: the current
  machine's alternates never re-materialize as entries, an unseen machine
  yields one entry, unattributed addresses dedupe by address.
- **Switching is in place, no reload** (board 315, owner 2026-10-08: "切换的过程要丝滑一些，不要整个画面又像到登录页面重新登录加载一样"). It was storage writes + `location.reload()` until then, which flashed the disconnected top bar and the connect card. `lib/app/server-switch.ts` is THE switch; every entry (rail/phone switcher, Settings › Connection, Add server, the failed panel's Retry/Back) calls `switchTo`. The shell (rail, tab bar) stays drawn; the server-bound content tree is not mounted while `switching` is set and is remounted under `{#key serverEpoch}` on arrival. The ORDER is the contract, each step finished before the next:
  1. **Leave guards** (`lib/app/leave-guards.ts`): every instance that will be destroyed registers its OWN guard (the Files page, the Hub drawer's Files, AgentsPage on the rail or inside Settings). Guards on the page the user is on ask first, without navigating; any other dirty one is revealed on its page and asks in its own dialog; a save/delete in flight finishes first. While the walk runs App HOLDS Settings mounted (`{#if page === 'prefs' || holdPages}`), so revealing Files can never unmount an Agents editor that has not answered (review P1: before the hold, that editor's draft was lost and its guard then skipped as "gone"). The walk always ends on the page it started from. Any "stay" and nothing happened.
  2. **One intent**: the reconnect machine stops, `switching` unmounts the tree (so no keystroke, send or save reaches it), and every App async begun earlier (boot restore's `listSessions`, `optimizeConnection`) checks `serverSwitch.owns(intent)` before writing.
  3. **Downloads suspended**: every running attempt aborts with `SUSPEND` and is awaited, so no retry, re-sign or WS fallback of the old server's chain can go out on the next connection (`download.ts` re-checks the signal after its retry sleep, before `freshUrl`). Its part is KEPT (see file-handling.md). Then `tick()` lets the unmounted tree's persistence run (the composer's last keystroke) — under the old server.
  4. **Park the source, once**: `parkFrom` files every key in `PARKED_KEYS` under the leaving id. Only a switch that began CONNECTED parks; Retry and Back reuse the frozen `from` and never park again, so a half-switched world can never overwrite the source's slot.
  5. Socket closed (`disconnect()` rejects every pending RPC), `resetServerMemory()`.
  6. **Connect without touching the mirror or CURRENT**: a reload mid-switch boots the server you left.
  7. **Only after auth**, `activateSwitched`: record by the machine id the server REPORTED (a new address of a known machine folds into its entry, a new machine gets a new one; an address never guesses a machine), add the address to that machine's failover set, `pointTo` the canonical entry, write the mirror. Then `comeUp`: `hubPrefs.reloadServerState()`, `serverEpoch++`, `bringUp` (the boot path's own half after connect).

  **One socket owner.** Moving the socket to another address of the SAME server (the Connection page's address rows, the periodic optimizer) goes through `connectAddress` in the same module, under the same intent counter: a switch that starts while it dials makes it stale, and a stale attempt neither publishes its result nor restarts the reconnect loop (review P1: it used to call `reconnectMachine.start()` in its catch, and that loop reads the mirror, i.e. the server just left, over the new server's socket). `probeHub` and `loadBackends` drop an answer that arrives after a switch began. The reconnect machine's own starts (`setOnDisconnect`, the visibility resume) are skipped while `switching` is set.

  A failure keeps the panel (Retry · Back to the source · Edit · the server switcher); in FAILED only the server control and the panel work, the page buttons stay disabled. A second address of the SAME machine is not a switch: failover keeps its path and nothing resets. The disconnected connect page reaching a different server comes up the same way (`onConnected(true)`).

  | state | where it lives | on a switch |
  |---|---|---|
  | theme, fonts, zoom, locale, layout, shortcuts, rail order, widths, notifications, `tmux_servers`, `tmux_machines` (keyed by machine id) | localStorage | global, untouched |
  | `tmux_state` (tab, terminal target, split), `tmux_machine_id`, `tmux_hub_project` / `_drafts` / `_seen` / `_lead` / `_drawer` / `_roster_expanded` (per-project maps keyed by tmux session NAME: `app` on A is not `app` on B) | localStorage, `PARKED_KEYS` | parked as `<key>::<id>`, the target's surfaced |
  | every mounted component, its ws.ts listeners and subscriptions | memory | unmounted with the tree |
  | Files' parked cwds (`browsed`), download rows, served backends, the hover card, App's hub probe / serverInfo / terminal and Files targets | module/App memory | `resetServerMemory()` |
  | download parts on disk | the shell | kept; `partId` hashes the machine id, Resumable lists only this server's |

  What the tests cover, and what they do not:
  - `server-switch.test.ts` runs the real switch module, the real storage half (`servers.ts`), the real leave walk and the real `download()` against a fake transport that answers as machine A or B (one socket, `disconnect()` rejects what is pending). Around it is a hand-written app double (connected/mounted flags, unmount writes as a function list), not App: it proves the ORDER and the storage rules, plus `connectAddress` ownership (an A address still dialing when B is picked neither reconnects nor touches B). Scenarios: A→B→A with a same-named project; B fails twice → Back to A with every parked key unchanged; a pending read rejected by the switch (the fake rejects it on disconnect, so no late success continuation runs); a suspended download re-signs nothing; a dirty-guard cancel; a second address of A; a superseded intent. Each has a negative control.
  - `leave-host.mount.test.ts` mounts the REAL Files and AgentsPage in App's page structure (`LeaveHost.test.svelte`: Files in a hidden layer, Agents under the held `prefs` condition) and drives the real walk: a cancel at either editor loses nothing, Settings stays mounted while Files asks, a save in flight is waited for. The fixture mirrors App's condition; `App.source.test.ts` pins App's own expression.
  - `Files.mount.test.ts` drives Files' real discard dialog through the walk.
  - Not covered by a test: App itself end to end (no App mount harness); the browser check in `temp/check/c315.mjs` (fake ws as two machines) is the evidence for the shell, the panel and no reload.
  - `server-memory.source.test.ts` keeps the `resetServerMemory` list honest against every module-level store under `src/lib`; it is maintenance, not a proof.
- **Single click switches; a pencil renames** (2026-09-12, #165, lead decision
  01:32). The old 260ms click hold guessed whether another click would arrive
  and delayed the requested switch, repeating the roster defect retired in
  #168. The chooser now closes and invokes the same switch primitive at once;
  choosing the current entry closes without reconnecting. Each row has an
  explicitly named pencil command, disabled while that row is already being
  edited. Rename still uses the existing name input and registry operation.
  The timer, delayed-click functions and double-click handler are removed
  whole. No alternate switch path, storage format or reconnect behavior is
  introduced.
- **A row's name defaults to the hostname the server reports** (board #310,
  owner 2026-10-05: "显示的名称应该默认是主机的名称，不是连接的url"). Every auth
  answers `{authenticated, machine_id, hostname}`, and one App effect hands that
  hostname to `servers.ts adoptHostname` for this machine's entry. A URL host
  (`hostLabel(address)`, e.g. `d1x.cloudfront.net` or `192.168.11.221`) is only
  the pre-auth placeholder. A name the user typed is never overwritten:
  `renameServer` sets `named`. The adopted value is kept as `hostname` on the
  entry, so it survives an address change (failover to the LAN address) and
  follows a renamed host, while an entry renamed by hand before #310 (no flag,
  a name that is neither its URL host nor an adopted hostname) keeps its name.
  No one-time migration: an old entry whose name is still its URL host adopts
  the hostname at its next connect. The address stays on the row's second line
  (`.sm-addr`), so two machines named alike remain distinguishable. Pinned by
  `servers.test.ts`.
- **The server chooser is a non-modal picker dialog** (2026-09-12, #165).
  It contains selection, rename and removal controls, so it follows the
  PanePicker family and native Tab order, not action-menu arrow navigation.
  Rail and Settings openers name the same dialog through haspopup/controls.
  It shares popup chrome but keeps two-line identity rows and sibling
  commands; the row budget remains stable while a name becomes an input.
  Measurements include borders, width is intrinsic before clamping, and
  the panel scrolls inside the zoom-corrected viewport inset.
  Outside pointer and opener-ancestor scroll close it; its own scroll and
  background terminal output stay open. Resize closes
  an ordinary picker, but an active rename remeasures the existing anchor
  instead: a soft keyboard must not destroy the input it just opened.
  Escape ownership is modal, then rename, then picker. Cancelling rename
  clears the draft state before focus restoration can fire blur; Enter and
  blur never commit during composition. An outside dismissal during composition
  records the current editor/menu identity and waits for the final native
  input before committing and closing, never deleting the unfinished draft.
  The same composition token gates blur and repeated close through the ending
  update; only afterward is the final native input value read and the captured
  commit/close intent completed. A replaced input or cancelled edit is ignored.
  Normal blur/outside dismissal keeps the existing rename commit behavior.
  Focus returns to the pencil or opener
  only if that origin is still connected and another control/modal has not
  taken ownership; a removed row falls back to the surviving picker after
  the existing confirmation. Application navigation shortcuts leave keys
  to focused menus/dialogs. No global Back/popstate mechanism is introduced.
  Chromium 152.0.7977.64 verifies 42 real-App picker checks on desktop and
  coarse input, including native CDP composition and adversarial ending/blur
  ordering, both openers, confirmation focus, resizing and ancestor scrolling.
  This is a controlled connection fixture, not OS-IME or APK acceptance;
  the Tauri-only navigation shortcut path is source-guarded in this run.
- **An address switch shows on the row that was tapped** (review, 2026-09-03):
  App keeps `pendingAddress` from the moment `onAddress` dials until the
  connect settles either way (`.finally`, guarded so a second tap's pending
  is not cleared by the first's outcome), and Preferences' address list gives
  every row a dot in the ONE status language — `--status-sleep` at rest, accent
  on the current address, accent + app.css `.live-dot` on the one still dialing
  (`aria-busy`, title "Connecting…"). Before, the old row stayed lit while the
  socket was rebuilt and nothing said the tap had landed. A failed direct
  connect hands over to the reconnect machine, whose bar carries on from there.
- **The failover set is editable: removable rows, drag priority** (board #222,
  owner 2026-09-20: "可以手动删除某个链接，并且可以设定连接的优先级上下拖动
  排序"). The list's stored order IS the priority — the reconnect round-robin
  walks it and `findBestAddress`'s stable class sort keeps it within a class —
  so a drag needs no new concept, only a write. A row's × removes one
  alternate; the ACTIVE address is never removable (it is the live connection,
  and deleting it would only hide the primary while it stays primary). The
  drag speaks the rail's reorder idiom through the generic `moveBefore` /
  `listDropAt` in nav-order.ts (one mechanism, not a second reorder dialect),
  but the GRIP, not the row, is the handle: the row is already the switch
  command, and on touch a whole-row vertical drag would fight the page's
  scroll — the grip opts out with `touch-action: none`, and Arrow keys on the
  focused grip are the keyboard form of the same move. Both edits hand the new
  list to App's `onAddressesChange`, which persists through
  `servers.ts` `saveMachineAddresses` — the map's ONE writer, sanitized at the
  door — and bumps a version so the derived list re-reads (localStorage is not
  reactive).
- **The rail entry is a control, not a page**: it rides the RAIL_GAP branch
  (above the configure group — "右下角agent上边"), carries no rail slot, and
  its popover follows the app's one menu recipe (fixed layer, measured then
  shown, outside-pointerdown/Escape/resize dismissal). The Settings connect
  form IS the add flow; the `+` row only navigates there.
- **The phone's entry is a row at the top of Settings** (review, 2026-09-03):
  the rail does not exist on the touch layout, so the registry had no door
  there at all — named servers were invisible on the device the app is for.
  App passes `onServers` (= `toggleServerMenu`) and the connected machine's
  authenticated hostname (`serverInfo.hostname`, falling back to
  `hostLabel(activeAddress)` before auth) to Preferences only when
  `connected && layout.isTouchDevice`; Preferences renders a `.side-row`
  (swap icon + HOSTNAME, never the registry alias or raw address) above the
  category list that opens the SAME popover, anchored to the row.
  The popover's outside-dismissal spares whichever control opened it
  (`serverMenuTrigger`), not a class name, so both doors toggle cleanly. The
  registry is read at boot and on open/rename, so the name follows a rename.

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

### Connection-link copy feedback belongs to its attempt (#167, 2026-09-12)

Both share commands previously ignored `copyText(false)` and displayed success;
their independent timers could also clear a later copy. App and the connect
form now use `createFeedbackLifetime` and its one completion scheduler. Only a
successful copy shows the brief copied state. A connect-form failure remains
visible as an inline `config-error` alert beside the share command until retry
or input change, without nesting another feedback frame inside the connect
card. App rejects the current failed attempt into Preferences' existing
command-error channel. Neither path logs the link or creates a global notice.

The link and context are captured before awaiting the clipboard. A newer
attempt, changed inputs/page/server or unmount invalidates the old outcome;
disposal also cancels expiry. App's descriptor is a serialized primitive of
page, connection state, active address, server ID and machine ID: replacing
`serverInfo` during same-machine polling must not clear feedback or invalidate
a pending copy. App additionally rechecks its non-reactive stored address,
token and socket. URLSearchParams construction and token-bearing
deep-link semantics are unchanged. Settings mount tests use fake credentials
and controlled clipboard results/timers; App wiring is source-pinned and
Preferences' existing error/retry channel is exercised by its mount test,
not claimed as a full-App or native-clipboard test.

### Servers are a named registry; the machine is the identity

(board #55): `src/lib/app/servers.ts` keeps `tmux_servers` (+`tmux_server_current`) while the old `tmux_address`/`tmux_token`/`tmux_socket` stay the ACTIVE MIRROR every existing reader keeps reading. One machine = one entry however many LAN/Tailscale/WAN addresses it answers on — `recordServer` merges by `machineId` (learned at connect) without moving CURRENT, migration attributes `tmux_address_history` through `tmux_machines`, and the same-machine failover semantics are untouched. A different-machine successful connect goes through `activateConnected` (or, for the in-place switch, `activateSwitched` after auth — board 315): park the old live state before surfacing the target; a same-machine alternate records in place. Switching is in place since board 315 (`server-switch.ts`, the order and the state-ownership table are in the Multi-Server section above): the per-server keys (`PARKED_KEYS`) park/restore under `::<id>` so restore targets and Hub drafts never cross servers, the content tree remounts and `resetServerMemory` clears the module caches, so nothing in memory leaks across. The desktop rail's switcher rides the RAIL_GAP branch above the configure group — a control (no drag slot), popover in the one menu recipe; Add server is a dialog over the connected shell (cancel changes nothing) and activates only after authentication. Forgetting a non-current server captures its row identity and asks through the shared `ConfirmDialog` before removing config + parked state.

### The failover set's order is the priority, edited through one writer

(board #222): the Connection page's address list is the machine's `tmux_machines` failover set, and its stored order is what the reconnect round-robin walks — so reordering IS setting priority, no new concept. A row's × removes an alternate (never the ACTIVE address — the live connection), and the grip drags a row to its new place (Arrow keys on the focused grip are the keyboard form); the drag reuses the rail's idiom via the generic `moveBefore`/`listDropAt` in nav-order.ts, with the grip — not the row — as the handle because the row is already the switch command and a whole-row touch drag would fight the page's scroll. Both edits go to App's `onAddressesChange`, which persists through `saveMachineAddresses` (the map's ONE writer: sanitized, deduped, an empty set drops the key) and bumps a version counter so the derived list re-reads. The hooks-management row that used to sit under the list retired in the same task — managed agents carry hooks in their isolated homes, so the global install/remove surface and its `agent_hooks_*` RPCs were deleted whole (agent-notifications.md § Hook Management).
