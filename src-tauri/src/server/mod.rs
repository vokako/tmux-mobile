use crate::agent_notifications::AgentNotificationHub;
use crate::tmux;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
mod wire;
pub use wire::{encode_wire_payload, decode_wire_payload, WIRE_PLAIN_JSON, WIRE_DEFLATE_JSON, COMPRESS_MIN_BYTES};
use wire::HalfCipher;
mod download;
use download::{looks_like_dl_request, handle_http_download};
mod team_rpc;
mod hub_rpc;
mod rpc;
mod connection;
pub use connection::handle_connection;
use connection::{enable_tcp_keepalive, handle_connection_ws, ws_config};

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
    /// One PAGE of `room`'s messages, oldest first, walking backwards:
    /// `{ messages, has_more, head_seq }`. `before_seq` is exclusive; `None` is
    /// the newest page, which is byte-for-byte what `history` answers.
    ///
    /// The room is never pruned — the transcript is the record — so a client that
    /// wants a small first load needs a way to ask for the REST later, and the
    /// bus's `seq` (a message's log position) is the cursor for it: stable,
    /// gapless, and already on every message the client holds, which a millisecond
    /// timestamp is not.
    ///
    /// Defaulted rather than required: a bridge that cannot page (a test double,
    /// an older impl) answers the newest page and says there is no more, which is
    /// exactly the behaviour every caller had before paging existed.
    fn history_page(&self, room: &str, before_seq: Option<i64>, limit: i64) -> serde_json::Value {
        let mut v = self.history(room, limit);
        if let Some(obj) = v.as_object_mut() {
            obj.entry("has_more").or_insert(serde_json::json!(false));
            let _ = before_seq;
        }
        v
    }
    /// ONE message by its id, however old it is: `Some(message)` or `None`.
    ///
    /// An exact lookup, not a scan of the newest page — which is what the archive
    /// path used to do (`history(room, 1000)`), so a message older than that was
    /// invisible to it. Now that a client can scroll back to any message, "the
    /// newest 1000" is not a place where correctness may live.
    ///
    /// Defaulted for the same reason `history_page` is: a bridge that cannot look
    /// up falls back to searching the newest page, i.e. the old behaviour.
    fn message_by_id(&self, room: &str, id: &str) -> Option<serde_json::Value> {
        self.history(room, 1000)
            .get("messages")?
            .as_array()?
            .iter()
            .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(id))
            .cloned()
    }
    /// Roster + presence for `room`: `{ "roster": [...] }`.
    fn roster(&self, room: &str) -> serde_json::Value;
    /// Post as a participant in `room`. Returns the stored message JSON.
    fn post(&self, room: &str, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String>;
    /// Newest message timestamp (ms) per room: `{ "<room>": ts }`. Used to order
    /// projects by their conversation, so it must include rooms with no running
    /// team.
    fn room_latest(&self) -> serde_json::Value;
    /// Forget messages by id, for good. The archive is the reversible half of
    /// deleting; this is the half that is not.
    fn delete_messages(&self, room: &str, ids: &[String]) -> Result<usize, String>;
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
    /// Open (or re-open) a plain chat room on the bus — no tmux session, no
    /// roster, no supervisor. This is the project-hub substrate: room id is
    /// `proj:<session>` and the caller owns that convention. Idempotent.
    fn open_room(&self, room: &str) -> Result<(), String>;
}

pub type OptTeam = Option<Arc<dyn TeamBridge>>;
pub type NotificationHub = Arc<AgentNotificationHub>;

// ─── RoomPoster implementation ───────────────────────────────────────────────
// The notification hub needs to post into project rooms from the hook consumer
// (a background task without a per-request context). This adapter bridges the
// `RoomPoster` trait to the `TeamBridge` without exposing bus types.
//
// INVARIANT: `post_to_room` is called from `consume_file` only for STOP events
// on managed windows, always with `record_only = true`.
// The hub_post handler enforces that record_only posts never trigger delivery.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
struct TeamRoomPoster {
    team: Arc<dyn TeamBridge>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl crate::agent_notifications::RoomPoster for TeamRoomPoster {
    fn post_to_room(&self, session: &str, agent: &str, body: &str, record_only: bool) {
        let room = hub_rpc::project_room(session);
        let _ = self.team.open_room(&room);
        // record_only means: store in room, do NOT deliver (type) into panes.
        // We encode this as requires_reply = false and rely on hub_post's
        // record_only gate. Since we call bus.post directly (bypassing the RPC
        // layer), we must not call deliver_mentions — which is exactly what
        // record_only = true prevents in handle_hub_request. Direct bus call is
        // safe here because we never deliver; the RPC path's deliver_mentions
        // is the only thing we're skipping.
        let _ = self.team.post(&room, agent, body, false);
        // Delivery suppression: requires_reply=false means the agora bus
        // records the message but does not alert anyone. The record_only
        // parameter is not forwarded to the bus itself (it has no such concept);
        // it is purely a gate in handle_hub_request to prevent deliver_mentions.
        // Since we bypass handle_hub_request here, `record_only` is honoured
        // structurally: we never call deliver_mentions.
        let _ = record_only; // documented above; suppress unused-variable warning
    }
}

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

    // Fold live tmux state back into the project declarations. Nobody
    // hand-writes a project; the capturer is what makes "close it and reopen it
    // later" possible. The notification hub supplies the agent conversation
    // ids, so a restored window resumes instead of starting blank.
    // Desktop-only (state.db is not built for mobile).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        crate::projects::set_agent_sessions(notifications.clone());
        tokio::spawn(crate::projects::capture_loop());
        // Inject the room poster so hook-sourced stop events can auto-post
        // managed agents' final replies into the project chat room.
        // Degrades silently to a no-op when team is None (server without the
        // team bus, e.g. `npm run dev:server` without a team configured).
        if let Some(ref team_arc) = team {
            let poster: Arc<dyn crate::agent_notifications::RoomPoster> =
                Arc::new(TeamRoomPoster { team: team_arc.clone() });
            notifications.set_room_poster(poster);
        }
    }

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
    use super::rpc::Request;

    pub(super) fn req(method: &str, params: serde_json::Value) -> Request {
        Request { id: Some(1), method: method.to_string(), params }
    }
}

