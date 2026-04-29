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
- Heavy RPCs are wrapped in `heavyRpc(...)` which bumps a global counter
  `heavyRpcInFlight`; while > 0 the heartbeat task skips its next ping (and
  resets `pingFails` to 0). Rationale: a large response (e.g. 50 MB base64)
  can saturate the downlink for tens of seconds on mobile, queueing the
  ping response behind the transfer and tripping the 6 s RPC timeout even
  on a healthy link. The transfer itself has its own 60 s timeout so a
  genuinely dead link is still caught.
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
  saturates the downlink. JSON-RPC ping shares the same transport and can't
  pre-empt a multi-megabyte send, so the `heavyRpcInFlight` counter pauses
  the heartbeat for the duration of a known long transfer (downloads,
  uploads). Any new large-response RPC must wrap its `call(...)` in
  `heavyRpc(() => ...)` or it will trip the heartbeat on slow networks.
