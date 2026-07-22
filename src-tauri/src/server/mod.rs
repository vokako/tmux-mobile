use crate::agent_notifications::AgentNotificationHub;
use crate::tmux;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::{accept_async_with_config, tungstenite::Message, tungstenite::protocol::WebSocketConfig};
mod wire;
pub use wire::{encode_wire_payload, decode_wire_payload, WIRE_PLAIN_JSON, WIRE_DEFLATE_JSON, COMPRESS_MIN_BYTES};
use wire::{bytes_to_hex, hex_to_bytes, provided_token_matches, derive_key, HalfCipher};
mod download;
use download::{looks_like_dl_request, handle_http_download};
mod team_rpc;
use team_rpc::{handle_team_request, handle_notification_request, notification_push_loop, team_push_loop};
mod rpc;
use rpc::{Request, Response, Subscriptions, handle_request, handle_subscribe, handle_unsubscribe, ERR_PARSE, ERR_INTERNAL, ERR_AUTH};

// ─── team multi-agent bus bridge ────────────────────────────────────────
// The Team tab talks to the team group-chat bus, which lives in a desktop-only
// sub-crate (heavy axum/rmcp/rusqlite deps the phone never builds). To keep
// server.rs compiling on Android/iOS, the bus is reached only through this
// JSON-only trait object: the desktop build supplies a concrete impl wrapping
// `agora::Bus`; mobile builds always pass `None` and never call it.
//
// All methods speak `serde_json::Value` so no team type crosses this boundary.
// Every chat operation is scoped to a `room` (= a team). Multiple teams are
// fully isolated rooms sharing one daemon/db; the phone passes the active
// room with each call, and pushes are tagged with their room so the client
// can filter to the team currently in view.
pub trait TeamBridge: Send + Sync {
    /// Recent messages for `room`, oldest first: `{ "messages": [...] }`.
    fn history(&self, room: &str, limit: i64) -> serde_json::Value;
    /// Roster + presence for `room`: `{ "roster": [...] }`.
    fn roster(&self, room: &str) -> serde_json::Value;
    /// Post as a participant in `room`. Returns the stored message JSON.
    fn post(&self, room: &str, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String>;
    /// Force an agent's stored status in `room` (supervisor idle-sleep:
    /// `"sleeping"` to park, `"idle"` to wake). No-op if the agent is unknown.
    fn set_agent_status(&self, room: &str, agent: &str, status: &str) -> Result<(), String>;
    /// Desired-roster employees for `room`: `{ "employees": [...] }`.
    fn employees(&self, room: &str) -> serde_json::Value;
    /// Seed an employee into `room`'s desired roster (used by the supervisor).
    fn seed_employee(&self, room: &str, name: &str, spec: &serde_json::Value) -> Result<(), String>;
    /// Raw employee list for `room` as `(name, spec, state)` for the
    /// supervisor's reconcile loop.
    fn employee_specs(&self, room: &str) -> Vec<(String, serde_json::Value, String)>;
    /// Whether `room` is still a registered (not-yet-closed) team. The
    /// supervisor uses this to exit cleanly when its team is closed.
    fn room_exists(&self, room: &str) -> bool;
    /// Start a team for `workspace` from `template` (named roster; empty =
    /// "default"): derive its stable room from workspace+template, seed the
    /// roster, and launch agents into a per-Team tmux session. Idempotent for
    /// the same workspace+template pair. Returns `{ room, started, workspace }`.
    fn start_team(&self, workspace: &str, template: &str) -> serde_json::Value;
    /// Stop a team: kill its tmux session and forget it (the chat log persists
    /// in the db). Returns true if the room was known.
    fn close_team(&self, room: &str) -> bool;
    /// All known teams: `[{ room, workspace, session, started, agents }]`.
    fn teams(&self) -> serde_json::Value;
    /// All roster templates: `[{ name, agents:[…] }]`.
    fn templates(&self) -> serde_json::Value;
    /// Save (overwrite) a template's agent array.
    fn save_template(&self, name: &str, agents: &serde_json::Value) -> Result<(), String>;
    /// Delete a template (the built-in "default" is protected).
    fn delete_template(&self, name: &str) -> Result<(), String>;
    /// The global system prompt prepended to every agent's brief.
    fn system_prompt(&self) -> String;
    /// Save the global system prompt (empty clears it).
    fn save_system_prompt(&self, text: &str) -> Result<(), String>;
    /// The default workspace to offer in the UI when none is chosen (the
    /// current terminal session's cwd if known, else the user's home).
    fn default_workspace(&self) -> String;
    /// A receiver of newly-broadcast messages across ALL rooms, each
    /// pre-serialized to a JSON string (the `room` field is inside each
    /// message). The client filters to the team currently in view.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String>;
}

pub type OptTeam = Option<Arc<dyn TeamBridge>>;
pub type NotificationHub = Arc<AgentNotificationHub>;

// Brute-force protection: track failed auth attempts per IP
pub type AuthTracker = Arc<Mutex<HashMap<IpAddr, (u32, tokio::time::Instant)>>>;

// Resize tracking: per-window state so that short disconnects (app
// backgrounded, network blip) don't trigger an immediate tmux
// `resize-window -A`, which reflows the pane to a non-mobile size and
// makes the re-connect feel like "页面刷新半天". On the last connection
// to a window dropping off, we schedule a restore task that sleeps for
// `grace_secs`; if any connection resizes the window again before it
// fires we abort the task and the window stays at mobile size.
//
// - `per_conn[conn_id]` : windows this connection has resized. Used at
//   disconnect time to know which windows to decrement.
// - `per_window[win]`   : aggregate state — how many still-connected
//   connections are "holding" the window at its current size, and any
//   in-flight grace timer.
#[derive(Default)]
pub struct ResizeTrackerInner {
    pub per_conn: HashMap<u64, std::collections::HashSet<String>>,
    pub per_window: HashMap<String, WindowResizeState>,
}

#[derive(Default)]
pub struct WindowResizeState {
    pub active_conns: u32,
    pub pending_restore: Option<tokio::task::JoinHandle<()>>,
}

pub type ResizeTracker = Arc<std::sync::Mutex<ResizeTrackerInner>>;

static CONN_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

const MAX_AUTH_FAILURES: u32 = 5;
const AUTH_LOCKOUT_SECS: u64 = 60;
// Drop auth-tracker entries whose last activity is older than this (in
// seconds). Otherwise a distributed scan from many IPs would grow the
// HashMap unboundedly; each IP only needs to be remembered for as long
// as the lockout could still apply.
const AUTH_TRACKER_GC_AFTER_SECS: u64 = 600;
const SUBSCRIPTION_POLL_MS: u64 = 200;
const MAX_CAPTURE_FAILURES: u32 = 5;

const WS_MAX_MESSAGE_BYTES: usize = 80 * 1024 * 1024;
const WS_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Configure aggressive TCP keepalive on a freshly accepted connection.
///
/// Why: home routers / cellular NAT gateways drop "idle" TCP entries from
/// their state tables after as little as ~30 s. Without keepalive the next
/// packet hits a dropped entry, the gateway sends RST, and the WebSocket
/// dies with a 1006 abnormal close (ECONNRESET on the server side).
///
/// We already have a 15 s WebSocket PING — but that runs in user space and
/// may queue behind other traffic, drift past 30 s in adverse scheduling,
/// or fail to reach the gateway when buffered. TCP keepalive is a kernel
/// timer emitting empty ACK packets that **always** count as activity to
/// stateful firewalls. Combining both is a small belt-and-suspenders.
///
/// Tunables:
///   IDLE = 20 s — start sending probes after 20 s without any data on
///                 the socket. Far below the typical 30–60 s NAT idle
///                 threshold.
///   INTERVAL = 5 s — probe every 5 s if the first one didn't ack.
///   COUNT = OS default (typically 3–9). The connection is considered
///          dead after IDLE + INTERVAL × COUNT, so ~35 s worst case.
fn enable_tcp_keepalive(stream: &TcpStream) {
    use socket2::{SockRef, TcpKeepalive};
    let ka = TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(20))
        .with_interval(std::time::Duration::from_secs(5));
    let sock = SockRef::from(stream);
    if let Err(e) = sock.set_tcp_keepalive(&ka) {
        eprintln!("⚠️  failed to enable TCP keepalive: {}", e);
    }
}

fn ws_config() -> WebSocketConfig {
    let mut cfg = WebSocketConfig::default();
    cfg.max_message_size = Some(WS_MAX_MESSAGE_BYTES);
    cfg.max_frame_size = Some(WS_MAX_FRAME_BYTES);
    cfg
}

/// Outbound message types funneled through the single send task. The send
/// task is what guarantees encrypt-counter ordering: even if many business
/// tasks finish out of order, they enqueue into this channel and the one
/// consumer encrypts + ws.send in strict FIFO order.
enum Outbound {
    /// Plain text frame — used only for the initial `server_nonce` handshake
    /// and the plain-fallback auth response (legacy path for http:// clients
    /// without Web Crypto).
    Plain(String),
    /// Ciphertext path. Once the send task has been given its cipher (via
    /// `InitCipher`), every payload here is encrypted in enqueue order.
    Encrypted(String),
    /// Same wire treatment as `Encrypted`, but flags a pane snapshot whose
    /// completion decrements the shared in-flight counter. Snapshots are
    /// latest-frame-wins: the subscription loop refuses to enqueue a new one
    /// while a previous one is still queued or being written to a slow
    /// socket. Without this, a link slower than the 200 ms capture cadence
    /// accumulates stale frames without bound (channel + kernel buffer), the
    /// client renders seconds-old content, and small RPC replies queue
    /// behind megabytes of dead snapshots until they time out.
    Snapshot(String),
    /// Hand a freshly-built send-side cipher to the send task. Emitted once,
    /// right after successful encrypted auth. Must be enqueued *before* any
    /// `Encrypted` message for that session, otherwise the send task drops
    /// the message with a warning.
    InitCipher(HalfCipher),
    /// Protocol-level WebSocket PING frame. Browsers auto-reply with PONG
    /// without application code running, so this probes TCP liveness without
    /// contending with JSON-RPC traffic for the encrypt/send mutex.
    Ping(Vec<u8>),
}

async fn subscription_loop(
    out_tx: tokio::sync::mpsc::UnboundedSender<Outbound>,
    subs: Subscriptions,
    snapshots_inflight: Arc<std::sync::atomic::AtomicUsize>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(SUBSCRIPTION_POLL_MS));
    let mut fail_counts: HashMap<String, u32> = HashMap::new();
    loop {
        interval.tick().await;
        // Backpressure: latest-frame-wins. While a previous snapshot is still
        // queued or being written to a slow socket, skip the whole tick —
        // don't even run the tmux captures. The next tick after the link
        // drains captures *current* content, so the client always converges
        // on the freshest frame instead of replaying a backlog of stale ones.
        if snapshots_inflight.load(std::sync::atomic::Ordering::Acquire) > 0 {
            continue;
        }
        let targets: Vec<(String, String)> = {
            let map = subs.lock().await;
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        if targets.is_empty() {
            continue;
        }
        for (target, prev) in targets {
            let t = target.clone();
            let t2 = target.clone();
            let (new_content, cursor, trailing_trimmed, current_cmd) = match tokio::task::spawn_blocking(move || {
                // Get cursor first for pane width, then capture content immediately after
                // to minimize race window between the two tmux calls
                let info = tmux::cursor_info_with_cmd(&t2).unwrap_or((0, 0, 24, 80, String::new()));
                let (content, trailing) = tmux::capture_pane_with_width(&t, None, info.3)?;
                // Re-read cursor to get position matching the captured content.
                // Reuse the previous current_cmd — its value rarely flips
                // mid-tick and one tmux call is cheaper than two.
                let info2 = tmux::cursor_info_with_cmd(&t2).unwrap_or(info.clone());
                Ok::<_, String>((content, (info2.0, info2.1, info2.2, info2.3), trailing, info2.4))
            })
            .await
            {
                Ok(Ok(v)) => {
                    fail_counts.remove(&target);
                    v
                }
                _ => {
                    let count = fail_counts.entry(target.clone()).or_insert(0);
                    *count += 1;
                    if *count >= MAX_CAPTURE_FAILURES {
                        subs.lock().await.remove(&target);
                        fail_counts.remove(&target);
                        // Notify client that pane is gone
                        let msg = serde_json::json!({
                            "id": null,
                            "method": "pane_closed",
                            "params": { "target": target }
                        });
                        let _ = out_tx.send(Outbound::Encrypted(serde_json::to_string(&msg).unwrap()));
                    }
                    continue;
                }
            };
            // state_key is only used for change detection; including the
            // current command makes "command changed but content didn't"
            // (rare but possible: `clear` + new shell prompt) a real diff.
            let state_key = format!(
                "{}\x00{},{},{},{}\x00{}",
                new_content, cursor.0, cursor.1, cursor.2, cursor.3, current_cmd
            );
            if state_key == prev {
                continue;
            }
            subs.lock()
                .await
                .insert(target.clone(), state_key);
            let content_changed = !prev.is_empty()
                && prev.split('\x00').next().unwrap_or("") != new_content;
            // Detect command changes by comparing the third \x00-delimited
            // segment. Empty `prev` means first push → always include cmd.
            let prev_cmd = prev.split('\x00').nth(2).unwrap_or("");
            let cmd_changed = prev.is_empty() || prev_cmd != current_cmd;
            // Send raw tmux cursor position + trailing trimmed count for xterm.js row mapping
            let cursor_obj = serde_json::json!({ "x": cursor.0, "y": cursor.1, "w": cursor.3, "h": cursor.2, "t": trailing_trimmed });
            // Only include current_command when it actually changed, to keep
            // the hot path frame ~zero bytes heavier than before.
            let mut params = serde_json::Map::new();
            params.insert("target".into(), serde_json::Value::String(target.clone()));
            if content_changed || prev.is_empty() {
                params.insert("content".into(), serde_json::Value::String(new_content));
            }
            params.insert("cursor".into(), cursor_obj);
            if cmd_changed {
                params.insert("current_command".into(), serde_json::Value::String(current_cmd));
            }
            let msg = serde_json::json!({
                "id": null,
                "method": "pane_output",
                "params": params,
            });
            // Increment BEFORE send: the send task decrements after the
            // frame hits the socket, so the counter can never be observed
            // at 0 while a snapshot is actually pending.
            snapshots_inflight.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if out_tx.send(Outbound::Snapshot(serde_json::to_string(&msg).unwrap())).is_err() {
                snapshots_inflight.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                return; // receiver has shut down
            }
        }
    }
}

pub async fn handle_connection(stream: TcpStream, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker, grace_secs: u64, team: OptTeam, notifications: NotificationHub) {
    // Peek at the request prelude to distinguish HTTP download from
    // WebSocket. 256 bytes covers the request line even with a reverse-proxy
    // path prefix; peek doesn't consume, so the WS handshake still sees the
    // full request.
    let mut buf = [0u8; 256];
    let n = match stream.peek(&mut buf).await {
        Ok(n) => n,
        Err(_) => return,
    };
    if looks_like_dl_request(&buf[..n]) {
        handle_http_download(stream, addr, token).await;
        return;
    }

    let conn_id = CONN_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let conn_started_at = std::time::Instant::now();
    println!("📱 Client connected: {} (conn_id={})", addr, conn_id);

    // Check if IP is locked out, and opportunistically GC old entries so
    // the tracker doesn't grow unbounded under a distributed scan.
    {
        let mut tracker = auth_tracker.lock().await;
        tracker.retain(|_ip, (_fails, since)| {
            since.elapsed().as_secs() < AUTH_TRACKER_GC_AFTER_SECS
        });
        if let Some((fails, since)) = tracker.get(&addr.ip()) {
            if *fails >= MAX_AUTH_FAILURES && since.elapsed().as_secs() < AUTH_LOCKOUT_SECS {
                eprintln!("🚫 Rejected {} (locked out, {} failures)", addr, fails);
                return;
            }
        }
    }

    let ws_stream = match accept_async_with_config(stream, Some(ws_config())).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("❌ WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, resize_tracker, conn_id, conn_started_at, grace_secs, team, notifications).await;
}

async fn handle_connection_ws<S>(ws_stream: tokio_tungstenite::WebSocketStream<S>, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker, conn_id: u64, conn_started_at: std::time::Instant, grace_secs: u64, team: OptTeam, notifications: NotificationHub)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // Check if IP is locked out
    {
        let tracker = auth_tracker.lock().await;
        if let Some((fails, since)) = tracker.get(&addr.ip()) {
            if *fails >= MAX_AUTH_FAILURES && since.elapsed().as_secs() < AUTH_LOCKOUT_SECS {
                eprintln!("🚫 Rejected {} (locked out, {} failures)", addr, fails);
                return;
            }
        }
    }

    let (ws_sender, mut receiver) = ws_stream.split();
    let subs: Subscriptions = Arc::new(Mutex::new(HashMap::new()));
    // conn_id allocated above so the "connected" log line carries it.
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let mut authenticated = false;
    // team message-push task: started once, right after auth succeeds (it
    // enqueues Encrypted frames, which need the session cipher in place).
    // Aborted at teardown alongside the other per-connection tasks.
    let mut team_push_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut notification_push_handle: Option<tokio::task::JoinHandle<()>> = None;
    // Receive-side cipher lives in this task and guards strict decrypt
    // ordering. Send-side cipher is handed off to the dedicated send task
    // (below) so business tasks can finish out of order without corrupting
    // the encrypt counter.
    let mut recv_cipher: Option<HalfCipher> = None;

    // Outbound channel: every response / push goes through this single
    // consumer, which encrypts (in FIFO) and ws.send()s. That is what lets
    // multiple business tasks run in parallel — they just enqueue strings.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Outbound>();

    // --- Send task ---
    // shutdown: raised if the send task fails. The receiver uses it to
    // break out of ws.recv() promptly so we don't sit in onmessage while
    // responses are silently piling up in the out_rx queue (whose consumer
    // is gone).
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let shutdown_tx = shutdown.clone();
    // Snapshot in-flight counter shared between the subscription loop
    // (producer, increments + skips ticks while > 0) and the send task
    // (consumer, decrements after ws.send resolves).
    let snapshots_inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let snapshots_inflight_send = snapshots_inflight.clone();
    let send_task = tokio::spawn(async move {
        let mut ws_sender = ws_sender;
        let mut cipher: Option<HalfCipher> = None;
        while let Some(msg) = out_rx.recv().await {
            let is_snapshot = matches!(msg, Outbound::Snapshot(_));
            let frame = match msg {
                Outbound::Plain(s) => Message::Text(s.into()),
                Outbound::InitCipher(c) => { cipher = Some(c); continue; }
                Outbound::Ping(data) => Message::Ping(data.into()),
                Outbound::Encrypted(s) | Outbound::Snapshot(s) => {
                    if let Some(ref mut c) = cipher {
                        // 1. Frame + (optionally) deflate the JSON.
                        // 2. Encrypt the framed plaintext.
                        // 3. Send as a BINARY WebSocket frame — no base64.
                        // The framing byte rides INSIDE the ciphertext so the
                        // receiver only learns it after decrypting (it's not
                        // sensitive in itself, but keeping it inside avoids
                        // any accidental side-channel where wire size +
                        // tag together leak more than wire size alone).
                        let plaintext = encode_wire_payload(&s);
                        let ciphertext = c.encrypt(&plaintext);
                        Message::Binary(ciphertext.into())
                    } else {
                        // Plain-token fallback (no session cipher). Stays
                        // TEXT + raw JSON — same as before this change. We
                        // never compress here; the path is reserved for
                        // legacy http:// clients without Web Crypto, and
                        // simplicity matters more than bytes.
                        Message::Text(s.into())
                    }
                }
            };
            let send_result = ws_sender.send(frame).await;
            // Decrement regardless of send outcome — on error we break and
            // the connection tears down, but a clean counter avoids the
            // subscription loop spinning on a stale "in flight" forever if
            // teardown is slow.
            if is_snapshot {
                snapshots_inflight_send.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            }
            if send_result.is_err() {
                break;
            }
        }
        // Channel closed OR ws.send failed. Either way, whoever's still
        // writing to out_tx should stop as soon as possible — tell the
        // receiver to bail.
        shutdown_tx.notify_waiters();
    });

    // Step 1: Send server_nonce (plain, pre-cipher)
    let mut server_nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut server_nonce);
    {
        let msg = serde_json::json!({ "server_nonce": bytes_to_hex(&server_nonce) });
        if out_tx.send(Outbound::Plain(serde_json::to_string(&msg).unwrap())).is_err() {
            return;
        }
    }

    // Start subscription polling task (enqueues to out_tx like any task)
    let sub_handle = tokio::spawn(subscription_loop(out_tx.clone(), subs.clone(), snapshots_inflight.clone()));

    // Start keepalive: server sends WS PING every 15s; browsers auto-reply
    // with PONG in the WS layer without running app code, so this probes
    // TCP liveness without contending for the encrypt/send mutex. If we
    // haven't seen a PONG in PING_DEADLINE_SECS, assume the link is dead
    // and tell the connection to tear down.
    let last_pong_at = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    const PING_INTERVAL_SECS: u64 = 15;
    const PING_DEADLINE_SECS: u64 = 45;
    let ping_handle = {
        let out_tx = out_tx.clone();
        let last_pong_at = last_pong_at.clone();
        let shutdown_for_ping = shutdown.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(PING_INTERVAL_SECS));
            tick.tick().await; // discard immediate first tick
            loop {
                tick.tick().await;
                let elapsed = last_pong_at.lock().unwrap().elapsed().as_secs();
                if elapsed > PING_DEADLINE_SECS {
                    eprintln!("💀 ping deadline exceeded ({}s > {}s) conn_id={}; tearing down", elapsed, PING_DEADLINE_SECS, conn_id);
                    shutdown_for_ping.notify_waiters();
                    return;
                }
                if out_tx.send(Outbound::Ping(b"hb".to_vec())).is_err() {
                    return; // send task gone
                }
            }
        })
    };

    while let Some(msg) = tokio::select! {
        m = receiver.next() => m,
        _ = shutdown.notified() => None,  // send task died — tear down
    } {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                // Annotate with conn_id + how long the connection had been
                // up. This is the path that fires on TCP RST / network
                // teardown; the "uptime" number is the most useful clue
                // when chasing NAT-dropped sessions.
                let up = conn_started_at.elapsed().as_secs();
                eprintln!(
                    "❌ Read error from {} (conn_id={}, authed={}, up={}s): {}",
                    addr, conn_id, authenticated, up, e
                );
                break;
            }
        };

        // Resolve every data-frame variant to plaintext JSON, then run the
        // existing request pipeline below. Control frames (Close/Ping/Pong)
        // fall through to the inner match.
        let plaintext: Option<String> = match &msg {
            Message::Text(text) => {
                if let Some(ref mut rc) = recv_cipher {
                    // Legacy: text + base64-encoded ciphertext + raw JSON
                    // plaintext. Kept for any old clients still using it;
                    // the new wire path goes through Message::Binary.
                    use base64::Engine;
                    let ct = base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).unwrap_or_default();
                    match rc.decrypt(&ct) {
                        Ok(pt) => Some(String::from_utf8_lossy(&pt).to_string()),
                        Err(_) => { eprintln!("❌ Decrypt failed (text path) from {} conn_id={}", addr, conn_id); break; }
                    }
                } else {
                    Some(text.to_string())
                }
            }
            Message::Binary(bytes) => {
                // New wire: binary ciphertext, plaintext = framing byte +
                // (optionally deflated) JSON. Auth must already be set up.
                let Some(rc) = recv_cipher.as_mut() else {
                    eprintln!("❌ Binary frame before auth from {} conn_id={}", addr, conn_id);
                    break;
                };
                match rc.decrypt(bytes) {
                    Ok(pt) => match decode_wire_payload(&pt) {
                        Ok(s) => Some(s),
                        Err(e) => { eprintln!("❌ Wire decode failed from {} conn_id={}: {}", addr, conn_id, e); break; }
                    },
                    Err(_) => { eprintln!("❌ Decrypt failed (binary path) from {} conn_id={} bytes_len={}", addr, conn_id, bytes.len()); break; }
                }
            }
            _ => None,
        };

        if let Some(plaintext) = plaintext {
                let req = match serde_json::from_str::<Request>(&plaintext) {
                    Ok(r) => r,
                    Err(e) => {
                        let r = Response::err(None, ERR_PARSE, format!("invalid JSON: {}", e));
                        let _ = out_tx.send(if recv_cipher.is_some() {
                            Outbound::Encrypted(serde_json::to_string(&r).unwrap())
                        } else {
                            Outbound::Plain(serde_json::to_string(&r).unwrap())
                        });
                        continue;
                    }
                };

                if !authenticated {
                    // Auth must stay strictly serial (it sets up the ciphers).
                    if req.method != "auth" {
                        let r = Response::err(req.id, ERR_AUTH, "auth required — send {\"method\":\"auth\",\"params\":{\"token\":\"...\"}} first".into());
                        let _ = out_tx.send(Outbound::Plain(serde_json::to_string(&r).unwrap()));
                        break;
                    }
                    let client_nonce_hex = req.params.get("client_nonce").and_then(|v| v.as_str()).unwrap_or("");
                    let proof_hex = req.params.get("proof").and_then(|v| v.as_str()).unwrap_or("");
                    let plain_token = req.params.get("token").and_then(|v| v.as_str()).unwrap_or("");

                    if !client_nonce_hex.is_empty() && !proof_hex.is_empty() {
                        // Encrypted auth
                        let client_nonce_bytes = hex_to_bytes(client_nonce_hex).unwrap_or_default();
                        let proof_bytes = hex_to_bytes(proof_hex).unwrap_or_default();
                        if client_nonce_bytes.len() != 16 {
                            let r = Response::err(req.id, ERR_AUTH, "invalid client_nonce".into());
                            let _ = out_tx.send(Outbound::Plain(serde_json::to_string(&r).unwrap()));
                            break;
                        }
                        let mut cn = [0u8; 16];
                        cn.copy_from_slice(&client_nonce_bytes);
                        let key = derive_key(&token, &server_nonce, &cn);
                        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).unwrap();
                        // Bind proof to BOTH nonces so a captured proof from
                        // a previous handshake can't be replayed: each
                        // session has a fresh (server_nonce, client_nonce)
                        // pair, and the MAC commits to both.
                        mac.update(&server_nonce);
                        mac.update(&cn);
                        if mac.verify_slice(&proof_bytes).is_ok() {
                            authenticated = true;
                            auth_tracker.lock().await.remove(&addr.ip());
                            // Split into two independent cipher halves — one
                            // for the receive path (this task), one for the
                            // send path (send task).
                            recv_cipher = Some(HalfCipher::new(&key));
                            // The InitCipher message must be queued before
                            // any Encrypted message so the send task has its
                            // cipher ready. We enqueue it first, then the
                            // auth response.
                            let _ = out_tx.send(Outbound::InitCipher(HalfCipher::new(&key)));
                            let resp = serde_json::to_string(&serde_json::json!({"result":{"authenticated":true,"machine_id":*machine_id,"hostname":&hostname}})).unwrap();
                            let _ = out_tx.send(Outbound::Encrypted(resp));
                            if let Some(ref a) = team {
                                team_push_handle = Some(tokio::spawn(team_push_loop(out_tx.clone(), a.clone())));
                            }
                            notification_push_handle = Some(tokio::spawn(notification_push_loop(out_tx.clone(), notifications.clone())));
                            continue;
                        } else {
                            let mut tracker = auth_tracker.lock().await;
                            let entry = tracker.entry(addr.ip()).or_insert((0, tokio::time::Instant::now()));
                            if entry.1.elapsed().as_secs() >= AUTH_LOCKOUT_SECS { *entry = (0, tokio::time::Instant::now()); }
                            entry.0 += 1;
                            eprintln!("🚫 Auth failed from {} (attempt {})", addr, entry.0);
                            drop(tracker);
                            let r = Response::err(req.id, ERR_AUTH, "invalid proof".into());
                            let _ = out_tx.send(Outbound::Plain(serde_json::to_string(&r).unwrap()));
                            break;
                        }
                    } else if provided_token_matches(plain_token, &token) {
                        // Legacy plain token auth (http:// fallback)
                        authenticated = true;
                        auth_tracker.lock().await.remove(&addr.ip());
                        let r = Response::ok(req.id, serde_json::json!({ "authenticated": true, "machine_id": *machine_id, "hostname": &hostname }));
                        let _ = out_tx.send(Outbound::Plain(serde_json::to_string(&r).unwrap()));
                        if let Some(ref a) = team {
                            team_push_handle = Some(tokio::spawn(team_push_loop(out_tx.clone(), a.clone())));
                        }
                        notification_push_handle = Some(tokio::spawn(notification_push_loop(out_tx.clone(), notifications.clone())));
                        continue;
                    } else {
                        let mut tracker = auth_tracker.lock().await;
                        let entry = tracker.entry(addr.ip()).or_insert((0, tokio::time::Instant::now()));
                        if entry.1.elapsed().as_secs() >= AUTH_LOCKOUT_SECS { *entry = (0, tokio::time::Instant::now()); }
                        entry.0 += 1;
                        let fails = entry.0;
                        drop(tracker);
                        eprintln!("🚫 Auth failed from {} (attempt {})", addr, fails);
                        let r = Response::err(req.id, ERR_AUTH, "invalid token".into());
                        let _ = out_tx.send(Outbound::Plain(serde_json::to_string(&r).unwrap()));
                        break;
                    }
                }

                // Post-auth dispatch. Each request is processed in its own
                // task so a slow one (fs_download, fs_upload) doesn't block
                // the receive loop from reading / dispatching the next one.
                // Responses are funneled through out_tx and the send task
                // keeps their encrypt + ws.send in FIFO order.
                let subs_c = subs.clone();
                let tracker_c = resize_tracker.clone();
                let out_tx_c = out_tx.clone();
                let token_c = token.clone();
                let team_c = team.clone();
                let notifications_c = notifications.clone();
                tokio::spawn(async move {
                    let response = match req.method.as_str() {
                        "subscribe" => {
                            let mut map = subs_c.lock().await;
                            handle_subscribe(&req.params, &mut map)
                        }
                        "unsubscribe" => {
                            let mut map = subs_c.lock().await;
                            handle_unsubscribe(&req.params, &mut map)
                        }
                        m if m.starts_with("team_") => handle_team_request(&req, team_c.as_deref()),
                        m if m.starts_with("agent_notifications_") || m.starts_with("agent_hooks_") => handle_notification_request(&req, &notifications_c),
                        "resize_pane" => {
                            let id = req.id;
                            let p = &req.params;
                            let target = p.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as usize;
                            let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as usize;
                            let tracker = tracker_c.clone();
                            tokio::task::spawn_blocking(move || {
                                match tmux::resize_pane(&target, cols, rows) {
                                    Ok(()) => {
                                        let win = target.split('.').next().unwrap_or(&target).to_string();
                                        let session = target.split(':').next().unwrap_or(&target);
                                        let _ = tmux::set_resize_hook(session);
                                        // Register this conn as an active holder
                                        // of `win`, and cancel any in-flight
                                        // restore task — the window is actively
                                        // being used at a known size again.
                                        let mut t = tracker.lock().unwrap();
                                        let first_time_for_conn = t
                                            .per_conn
                                            .entry(conn_id)
                                            .or_default()
                                            .insert(win.clone());
                                        let state = t.per_window.entry(win).or_default();
                                        if let Some(h) = state.pending_restore.take() {
                                            h.abort();
                                        }
                                        if first_time_for_conn {
                                            state.active_conns = state.active_conns.saturating_add(1);
                                        }
                                        Response::ok(id, serde_json::json!({ "ok": true }))
                                    }
                                    Err(e) => Response::err(id, ERR_INTERNAL, e),
                                }
                            }).await.unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e)))
                        }
                        _ => tokio::task::spawn_blocking(move || handle_request(&req, &token_c))
                            .await
                            .unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e))),
                    };
                    let _ = out_tx_c.send(Outbound::Encrypted(serde_json::to_string(&response).unwrap()));
                });
            continue; // data frame handled
        }
        // Control frames fall through to here.
        match msg {
            Message::Close(_) => break,
            Message::Ping(_) => {
                // Browsers don't usually send WS PINGs (they only reply to
                // ours). If some non-browser client does, we just ignore it
                // here — the connection survives fine without an explicit
                // Pong because our own keepalive going the other way is what
                // we rely on.
            }
            Message::Pong(_) => {
                // Reply to our own keepalive ping: mark the link as alive.
                *last_pong_at.lock().unwrap() = std::time::Instant::now();
            }
            _ => {}
        }
    }

    sub_handle.abort();
    ping_handle.abort();
    if let Some(h) = team_push_handle.take() {
        h.abort();
    }
    if let Some(h) = notification_push_handle.take() {
        h.abort();
    }
    drop(out_tx); // close the channel so the send task finishes
    let _ = send_task.await;
    // Schedule restoration of any windows this connection resized, but only
    // for windows where this conn was the last remaining holder. Other
    // still-connected clients might be actively driving the same window at
    // a different size; in that case we just decrement and leave the window
    // alone. A grace timer absorbs short reconnects so the pane doesn't get
    // reflowed twice (once on disconnect, once on the reconnect resize).
    let windows = {
        let mut t = resize_tracker.lock().unwrap();
        t.per_conn.remove(&conn_id).unwrap_or_default()
    };
    // Decision made under the lock; side-effects (tmux call / await) happen
    // after the guard is dropped so the future stays Send.
    enum Decision { Skip, RestoreNow, Scheduled }
    for win in windows {
        let decision = {
            let mut t = resize_tracker.lock().unwrap();
            let Some(state) = t.per_window.get_mut(&win) else { continue };
            state.active_conns = state.active_conns.saturating_sub(1);
            if state.active_conns > 0 {
                Decision::Skip
            } else {
                // Defensively abort any existing pending (shouldn't be one,
                // since we only reach here as the final holder leaves).
                if let Some(h) = state.pending_restore.take() {
                    h.abort();
                }
                if grace_secs == 0 {
                    Decision::RestoreNow
                } else {
                    let tracker_c = resize_tracker.clone();
                    let w = win.clone();
                    let grace = std::time::Duration::from_secs(grace_secs);
                    let handle = tokio::spawn(async move {
                        tokio::time::sleep(grace).await;
                        // Re-check: a reconnect during the sleep would have
                        // aborted us, but abort is delivered at the next
                        // await point — we may have already passed the
                        // sleep. Confirm active_conns is still 0 before
                        // touching tmux.
                        let should_restore = {
                            let t = tracker_c.lock().unwrap();
                            t.per_window.get(&w).map(|s| s.active_conns == 0).unwrap_or(false)
                        };
                        if !should_restore { return; }
                        let w2 = w.clone();
                        let _ = tokio::task::spawn_blocking(move || tmux::run_resize_window_auto(&w2)).await;
                        eprintln!("📐 Restored window '{}' to auto-size (after {}s grace)", w, grace.as_secs());
                        let mut t = tracker_c.lock().unwrap();
                        if let Some(s) = t.per_window.get(&w) {
                            if s.active_conns == 0 {
                                t.per_window.remove(&w);
                            }
                        }
                    });
                    state.pending_restore = Some(handle);
                    Decision::Scheduled
                }
            }
        };
        match decision {
            Decision::Skip => {}
            Decision::RestoreNow => {
                let w = win.clone();
                let _ = tokio::task::spawn_blocking(move || tmux::run_resize_window_auto(&w)).await;
                eprintln!("📐 Restored window '{}' to auto-size", win);
                let mut t = resize_tracker.lock().unwrap();
                if let Some(s) = t.per_window.get(&win) {
                    if s.active_conns == 0 && s.pending_restore.is_none() {
                        t.per_window.remove(&win);
                    }
                }
            }
            Decision::Scheduled => {
                eprintln!("⏳ Window '{}' scheduled for restore in {}s", win, grace_secs);
            }
        }
    }
    let up = conn_started_at.elapsed().as_secs();
    println!(
        "👋 Client disconnected: {} (conn_id={}, authed={}, up={}s)",
        addr, conn_id, authenticated, up
    );
}

pub async fn start(host: &str, port: u16, token: &str) -> Result<(), Box<dyn std::error::Error>> {
    start_with_socket(host, port, token, "unknown", None, None, None, 600, None).await
}

#[allow(clippy::too_many_arguments)]
pub async fn start_with_socket(
    host: &str,
    port: u16,
    token: &str,
    machine_id: &str,
    socket: Option<String>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    disconnect_grace_secs: u64,
    team: OptTeam,
) -> Result<(), Box<dyn std::error::Error>> {
    tmux::set_socket(socket);
    // Best-effort harden existing config.toml so upgraded installs with the
    // old loose permissions get fixed on next start.
    crate::config::harden_config_perms();
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    let token = Arc::new(token.to_string());
    let machine_id = Arc::new(machine_id.to_string());
    let auth_tracker: AuthTracker = Arc::new(Mutex::new(HashMap::new()));
    let resize_tracker: ResizeTracker = Arc::new(std::sync::Mutex::new(ResizeTrackerInner::default()));
    let notifications = Arc::new(AgentNotificationHub::load());
    notifications.ensure_helper().map_err(|error| format!("Failed to prepare agent notification helper: {error}"))?;
    tokio::spawn(notifications.clone().run());

    // Load TLS config if cert+key provided
    let tls_acceptor = match (&tls_cert, &tls_key) {
        (Some(cert_path), Some(key_path)) => {
            let cert_data = std::fs::read(cert_path)
                .map_err(|e| format!("Failed to read TLS cert {}: {}", cert_path, e))?;
            let key_data = std::fs::read(key_path)
                .map_err(|e| format!("Failed to read TLS key {}: {}", key_path, e))?;

            let certs: Vec<_> = rustls_pemfile::certs(&mut &cert_data[..])
                .filter_map(|r| r.ok())
                .collect();
            let key = rustls_pemfile::private_key(&mut &key_data[..])
                .map_err(|e| format!("Failed to parse TLS key: {}", e))?
                .ok_or("No private key found in key file")?;

            let config = tokio_rustls::rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| format!("TLS config error: {}", e))?;
            Some(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
        }
        _ => None,
    };

    let scheme = if tls_acceptor.is_some() { "wss" } else { "ws" };
    println!("🚀 tmux-mobile server listening on {}://{}", scheme, addr);
    println!("🔑 Token: {}", token);
    println!("   Methods: auth, list_sessions, list_panes, capture_pane, send_keys, send_command, new_session, kill_session, subscribe, unsubscribe");

    loop {
        let (stream, addr) = listener.accept().await?;
        // OS-level NAT-friendly heartbeat. Must happen on the raw TcpStream
        // before tokio-rustls / tokio-tungstenite wrap it.
        enable_tcp_keepalive(&stream);
        // Disable Nagle. Our traffic is small JSON-RPC frames + occasional
        // big payloads — Nagle's 40 ms coalescing doesn't help here and
        // adds latency to interactive keystrokes.
        let _ = stream.set_nodelay(true);

        let token = token.clone();
        let machine_id = machine_id.clone();
        let auth_tracker = auth_tracker.clone();
        let control_mgr = resize_tracker.clone();
        let grace = disconnect_grace_secs;
        let team_c = team.clone();
        let notifications_c = notifications.clone();
        if let Some(ref acceptor) = tls_acceptor {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        // Peek first bytes after TLS handshake to tell HTTP
                        // /dl (large-file streaming) from a WebSocket upgrade.
                        // Plain-TCP uses TcpStream::peek; TlsStream has no
                        // peek, so we wrap in BufStream and use AsyncBufRead
                        // which fills an internal buffer and replays it on
                        // subsequent reads. The buffered stream is fed to
                        // whichever handler we dispatch to.
                        use tokio::io::AsyncBufReadExt;
                        let mut buf_stream = tokio::io::BufStream::new(tls_stream);
                        let is_http = match buf_stream.fill_buf().await {
                            Ok(b) => looks_like_dl_request(b),
                            Err(e) => {
                                eprintln!("❌ TLS read failed for {}: {}", addr, e);
                                return;
                            }
                        };
                        if is_http {
                            handle_http_download(buf_stream, addr, token).await;
                            return;
                        }
                        let ws_stream = match tokio_tungstenite::accept_async_with_config(buf_stream, Some(ws_config())).await {
                            Ok(ws) => ws,
                            Err(e) => { eprintln!("❌ WSS handshake failed for {}: {}", addr, e); return; }
                        };
                        let conn_id = CONN_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let conn_started_at = std::time::Instant::now();
                        println!("📱 Client connected (TLS): {} (conn_id={})", addr, conn_id);
                        handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, control_mgr, conn_id, conn_started_at, grace, team_c, notifications_c).await;
                    }
                    Err(e) => eprintln!("❌ TLS handshake failed for {}: {}", addr, e),
                }
            });
        } else {
            tokio::spawn(handle_connection(stream, addr, token, machine_id, auth_tracker, control_mgr, grace, team_c, notifications_c));
        }
    }
}

#[cfg(test)]
pub(super) mod test_util {
    use super::Request;

    pub(super) fn req(method: &str, params: serde_json::Value) -> Request {
        Request { id: Some(1), method: method.to_string(), params }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_util::req;

}
