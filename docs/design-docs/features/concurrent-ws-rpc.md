# Concurrent RPC on a Single WebSocket

## Context
Before this change, `handle_connection_ws` processed every request serially:

```
while let Some(msg) = receiver.next().await {
    let response = spawn_blocking(handle_request).await;  // BLOCKS the loop
    sender.lock().await.send(response).await;              // BLOCKS the loop
}
```

Two real-world symptoms fell out of that:

1. **`fs_download` starves everything else.** Reading a 50 MB file and
   base64-encoding it inside `spawn_blocking` takes several seconds; then the
   resulting single WS frame saturates the outbound TCP for tens of seconds
   on mobile. During that whole time the receiver `.await` is not polled, so
   other RPCs sitting in the TCP buffer never even get parsed. The client's
   3-polling-timeout disconnect rule trips (see `websocket-client.md`
   "pending.size === 0" gate).
2. **A slow tmux call on any one RPC pauses `pane_output` pushes** (which
   share the same connection).

## Decision
Make each connection process RPCs concurrently, while preserving strict
ordering of the encryption / decryption counters — which would be easy to
corrupt if multiple tasks raced to encrypt into a shared AES-GCM context.

### Architecture
```
                ┌── spawn_blocking(handle_request)
receiver loop  ─┼── spawn_blocking(handle_request)      ─┐
(decrypt FIFO)  ├── spawn_blocking(handle_request)       │─► out_tx ──► send task
                └── ...                                  │            (encrypt FIFO,
                                                         │             ws.send)
subscription_loop  (tmux polling, independent task)  ────┘
```

Key pieces:

- **`recv_cipher`** (`HalfCipher`): lives inside the receiver loop task.
  Every inbound frame is decrypted here, in order, with a monotonically
  incrementing counter. Decryption MUST stay serial — AES-GCM fails loudly
  if counters are reordered.
- **`send_cipher`** (`HalfCipher`): lives inside the send task. Every
  outbound frame is encrypted here, in order, with its own counter.
- **`out_tx` / `out_rx`** (`mpsc::UnboundedChannel<Outbound>`): the single
  funnel that serializes send-side work. All tasks (business, subscription,
  errors) enqueue; the send task dequeues in FIFO, encrypts, and writes to
  the WebSocket sink.
- **`Outbound` enum**: `Plain` (pre-auth handshake and plain-fallback
  auth response), `Encrypted` (everything after auth), `Snapshot`
  (pane_output pushes — same wire treatment as `Encrypted` but tracked
  by the in-flight counter, see below), `Ping` (WS protocol keepalive),
  and `InitCipher` (hands the send cipher to the send task right after
  successful auth).
- **`snapshots_inflight`** (`Arc<AtomicUsize>`): backpressure for pane
  snapshots. The subscription loop increments before enqueueing a
  `Snapshot` and *skips the entire capture tick* while the counter is
  non-zero; the send task decrements after the frame is written to the
  socket. Latest-frame-wins: on a link slower than the 200 ms capture
  cadence, the next tick after the socket drains captures *current*
  pane content instead of replaying a backlog of stale frames. Without
  this, slow links accumulated seconds of dead snapshots in the channel
  + kernel buffer, and small RPC replies queued behind them until the
  client's timeout breaker tripped a false disconnect.

### Why `HalfCipher` not `SessionCipher`
The old `SessionCipher` bundled both counters, implying one-object ownership.
Once we split encrypt and decrypt across two tasks, both halves can't share
one object without a Mutex that would re-serialize the very work we want
to parallelize. Two independent halves each with their own counter, both
derived from the same AES-256 key, is the natural shape.

### Auth is still serial
Authentication must complete before the cipher is known. The auth branch
of the receiver loop runs inline (does not `spawn` a task), sends its
response via `out_tx`, and on success emits `InitCipher` + the auth
response in that order.

Plain-token auth (the fallback for `http://` clients without
`crypto.subtle` — e.g. a browser pointing at a LAN IP) is different:
`authenticated` flips to `true`, but **no `InitCipher` is ever
emitted**. Subsequent RPC responses still go out as
`Outbound::Encrypted(...)` from the dispatch code (they don't know
which auth style the client used). The send task therefore has a
fallback: when it sees `Outbound::Encrypted` and has no cipher, it
sends the JSON as plain text instead of dropping it. That matches the
client's expectations in plain-token mode. If we ever want to force
the encrypted path and reject plain-fallback entirely, tighten the
auth branch — do not rely on the send task's silent fallback.

## Alternatives Considered
- **Second WebSocket just for downloads**: clean separation but requires
  a second auth + second key-exchange handshake on connect. More moving
  parts for a narrow win.
- **HTTP-only download endpoint** (a preexisting work-in-progress in
  `ws.js`): still the right long-term direction for huge transfers
  because it bypasses the WS framing entirely. Complementary to this
  change — concurrent WS RPC is the floor, HTTP download is the ceiling.
- **Chunked frame downloads**: server splits `fs_download` into many
  small WS frames, each an independent `Message`. Would let the send
  task interleave poller responses in between chunks. Rejected for now:
  meaningfully changes the client's decode path and duplicates what
  `fs_download_url` already does via HTTP.
- **Per-message async `Mutex`-guarded cipher**: keep one `SessionCipher`
  behind `Arc<tokio::sync::Mutex<_>>`. Works but the mutex is held for the
  entire send, so you end up serializing anyway — and cross-task encrypt
  ordering becomes easy to get wrong when multiple tasks each do
  "lock -> encrypt -> drop lock -> enqueue to ws". Not worth the risk.

## Trade-offs
- **Response ordering is no longer request-ordering.** Two `tmux
  list_panes` requests submitted in A,B order may come back B,A if B
  finishes first. Clients always key responses by `id`, so this is OK,
  but if anything ever relied on FIFO it will now break.
- **Send task is a single consumer.** If the send side ever got stuck
  (OS socket buffer full, slow peer), the channel would grow unbounded
  in memory for RPC responses. Pane snapshots are exempt: the
  `snapshots_inflight` counter caps them at one in flight, so the
  dominant traffic source self-limits. For the large download frame,
  we accept the transient memory hit.
- **Error on the send task** (ws.send failure) just returns; the
  receiver loop notices on its next ws read (it will get an error)
  and tears down. Could be tighter but adequate.

## Lessons Learned
- AES-GCM nonce reuse is immediate data loss. Every variant of "share a
  cipher across tasks" needs a mutex; once you have the mutex you've
  given up the parallelism you wanted. Pivot is to *own* the cipher
  (each half by exactly one task) and funnel work through a channel.
- `tokio::task::spawn` around a `spawn_blocking(...).await` is the
  pattern — it lets the async receiver return immediately while the
  blocking work happens on the blocking threadpool.
- The send task must receive an explicit `InitCipher` message rather
  than reading a shared cipher via `Arc<Mutex<...>>`: the latter
  re-introduces the ordering problem. The former guarantees that by the
  time the task processes an `Encrypted` item, its cipher is in place
  (because it saw `InitCipher` earlier in the same FIFO).
- Receiver's own `decrypt` must stay serial. Moving decryption into a
  spawn would reorder counters; since every RPC dispatch begins by
  spawning (business task), the decrypt step has to happen *before*
  that spawn, in the receiver loop body itself.
