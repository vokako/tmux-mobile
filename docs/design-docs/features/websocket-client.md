# WebSocket Client Robustness

## Context
Mobile connections are unreliable. WebSocket client must handle disconnects, reconnects, and edge cases gracefully.

## Decision
Custom WebSocket client (`ws.js`) with auto-reconnect, pending promise cleanup, and multi-address failover.

## How It Works
- `connect()` cleans up existing connection before creating new one
- `onclose` rejects all pending RPC promises (prevents caller hangs)
- Heartbeat ping every **5 s** with a **6 s** default RPC timeout (both tuned
  for flaky mobile links: 2 consecutive heartbeat fails *or* 3 consecutive
  RPC timeouts force a disconnect and trigger the reconnect UI; the previous
  10 s / 10 s settings pushed detection latency to 20-30 s which users read
  as "the app is frozen")
- Long-running RPCs (`fs_download`, `fs_upload`) pass explicit `timeoutMs` so
  the tightened default does not abort legitimate large transfers
- Activity-aware heartbeat: every inbound WS message stamps `lastRxAt`.
  The heartbeat tick skips pinging while `now - lastRxAt < HEARTBEAT_QUIET_MS`
  (8 s) — recent traffic *is* proof that the connection is healthy, no need
  to add an extra roundtrip. Only when the wire goes quiet do we send an
  explicit ping, and that ping gets a relaxed `PING_TIMEOUT_MS` (20 s)
  because the response can legitimately queue behind a large in-flight
  transfer. A `pingInFlight` guard prevents ticks from piling pings on top
  of each other.
- Auto-reconnect with exponential backoff
- Per-attempt connect timeout scales with address class: LAN 2 s, Tailscale
  3 s, WAN 5 s (public internet can legitimately need several seconds for
  TCP + TLS handshake on cellular)
- Multi-address failover: server `machine_id` tracks alternate addresses
- Optional E2E encryption layer
- `JSON.parse` wrapped in try-catch in `onmessage`
- Optional chaining on server push params (`data.params?.target`)

## Alternatives Considered
- **Socket.IO**: Rejected — adds dependency, WebSocket is sufficient for JSON-RPC
- **No auto-reconnect**: Rejected — mobile connections drop frequently

## Trade-offs
- Custom reconnect logic to maintain
- Pending promise rejection can cause UI flicker if not handled in components

## Lessons Learned
- Always reject pending promises on disconnect — otherwise callers hang forever
- Manual disconnect MUST cancel reconnect timers — otherwise zombie reconnects fire
- Wrap `JSON.parse` in try-catch — malformed messages crash the handler
- Use optional chaining on server push params — malformed messages can have missing params
- A healthy WS link can still miss heartbeats if a large in-flight response
  saturates the downlink. Our first fix wrapped specific heavy RPCs
  (`fs_download`, `fs_upload`) in an in-flight counter so the heartbeat would
  skip during them. That worked, but it was a *per-RPC* opt-in — every new
  large-response RPC had to remember to wrap. The current approach inverts
  it: the heartbeat reads *any* recent inbound traffic as liveness proof
  (`lastRxAt`) and only pings when the wire is genuinely quiet. No RPC
  needs to declare itself "heavy".
