use crate::agent_notifications::AgentNotificationHub;
use crate::tmux;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
mod wire;
pub use wire::{encode_wire_payload, decode_wire_payload, derive_session_keys, SessionKeys, E2E_VERSION, WIRE_PLAIN_JSON, WIRE_DEFLATE_JSON, COMPRESS_MIN_BYTES};
use wire::HalfCipher;
mod download;
use download::{looks_like_dl_request, handle_http_download};
mod hub_rpc;
mod rpc;
mod connection;
pub use connection::{handle_connection, ConnContext};
/// The one reading of who a chat body addresses (board #248), for `tmm send`.
pub use hub_rpc::mention_names;
use connection::{enable_tcp_keepalive, handle_connection_ws, ws_config};

pub type NotificationHub = Arc<AgentNotificationHub>;

// ─── RoomPoster implementation ───────────────────────────────────────────────
// The notification hub needs to post into project rooms from the hook consumer
// (a background task without a per-request context). This adapter bridges the
// `RoomPoster` trait to the hub's own message store (projects::rooms, board
// #107).
//
// A final reply is recorded once, then delivered only along the reply edge
// captured when the turn opened. The delivered `[reply]` envelope does not
// create a reverse edge, preventing ping-pong.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
struct HubRoomPoster;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl crate::agent_notifications::RoomPoster for HubRoomPoster {
    fn post_final(&self, session: &str, agent: &str, body: &str, reply_to: &[String]) {
        let room = hub_rpc::project_room(session);
        let _ = crate::projects::rooms::post_routed(&room, agent, body, reply_to);
        for target in reply_to {
            let line = format!("[tmm chat {}] {agent}: [reply] {body}", hub_rpc::stamp_now());
            hub_rpc::deliver_chat_line(session, target, &line);
        }
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
    start_with_socket(host, port, token, "unknown", None, None, None, 600).await
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
        // Pull any pre-#107 chat history out of the legacy team.db, exactly
        // once. Blocking SQLite work, so it runs before the listener loop.
        crate::projects::rooms::import_legacy();
        // Inject the room poster so hook-sourced stop events can auto-post
        // managed agents' final replies into the project chat room. The hub's
        // message store is always there on desktop (state.db), so this no
        // longer depends on a team bus being configured.
        let poster: Arc<dyn crate::agent_notifications::RoomPoster> = Arc::new(HubRoomPoster);
        notifications.set_room_poster(poster);
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
    // The token is the one secret. Show it to a person at a terminal (first
    // run, pairing a phone); never write it into a log — a supervised server's
    // stdout is a file, and that file would undo config.toml's 0600.
    {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() {
            println!("🔑 Token: {}", token);
        } else {
            println!("🔑 Token: not printed (stdout is not a terminal) — see {}", crate::config::config_dir().join("config.toml").display());
        }
    }
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

        let ctx = ConnContext {
            token: token.clone(),
            machine_id: machine_id.clone(),
            auth_tracker: auth_tracker.clone(),
            resize_tracker: resize_tracker.clone(),
            grace_secs: disconnect_grace_secs,
            notifications: notifications.clone(),
        };
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
                            handle_http_download(buf_stream, addr, ctx.token).await;
                            return;
                        }
                        let ws_stream = match tokio_tungstenite::accept_async_with_config(buf_stream, Some(ws_config())).await {
                            Ok(ws) => ws,
                            Err(e) => { eprintln!("❌ WSS handshake failed for {}: {}", addr, e); return; }
                        };
                        let conn_id = CONN_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let conn_started_at = std::time::Instant::now();
                        println!("📱 Client connected (TLS): {} (conn_id={})", addr, conn_id);
                        handle_connection_ws(ws_stream, addr, ctx, conn_id, conn_started_at).await;
                    }
                    Err(e) => eprintln!("❌ TLS handshake failed for {}: {}", addr, e),
                }
            });
        } else {
            tokio::spawn(handle_connection(stream, addr, ctx));
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

/// The wire error strings are a CONTRACT, not prose (board #146): `tmm` maps
/// -32601 to EXIT_NOT_FOUND and -32602 to EXIT_USAGE, the client tells an old
/// server apart by -32601, and agents read the messages. This table freezes
/// every hand-written error literal in rpc.rs / hub_rpc.rs and one row per
/// pass-through class (a missing param, a store error carried verbatim) BEFORE
/// the dispatchers are folded behind a `?`-returning inner fn, so each step of
/// that fold has to reproduce these bytes. A row moving here is a wire change
/// and needs its own reason.
#[cfg(test)]
mod golden_errors {
    use super::rpc::*;
    use super::test_util::req;

    fn check(tag: &str, r: Response, code: i32, message: &str) {
        let e = r.error.unwrap_or_else(|| panic!("{tag}: expected an error"));
        assert_eq!((e.code, e.message.as_str()), (code, message), "{tag}");
    }

    #[test]
    fn every_error_literal_and_class_is_pinned() {
        crate::projects::tests::use_test_store();
        let session = format!("golden-{}", uuid::Uuid::new_v4());
        let hub = |m: &str, mut p: serde_json::Value| {
            if let Some(o) = p.as_object_mut() {
                o.entry("session").or_insert(serde_json::Value::String(session.clone()));
            }
            super::hub_rpc::handle_hub_request(&req(m, p), None)
        };
        let rpc = |m: &str, p: serde_json::Value| handle_request(&req(m, p), "token");

        // ── pass-through classes ─────────────────────────────────────────
        check("missing param (hub)", super::hub_rpc::handle_hub_request(&req("hub_post", serde_json::json!({"body": "x"})), None),
            ERR_INVALID_PARAMS, "missing required param: session");
        check("missing param (rpc)", rpc("list_panes", serde_json::json!({})), ERR_INVALID_PARAMS, "missing required param: session");
        check("store error carried verbatim", hub("hub_board_note", serde_json::json!({"id": 999999, "body": "n"})),
            ERR_INVALID_PARAMS, "no issue #999999 on this board");

        // ── hub_rpc.rs literals ──────────────────────────────────────────
        check("hub_command", hub("hub_command", serde_json::json!({"agent": "a", "text": "model"})),
            ERR_INVALID_PARAMS, "a command must start with '/'");
        check("hub_command no such agent", hub("hub_command", serde_json::json!({"agent": "nobody-such", "text": "/model"})),
            ERR_INVALID_PARAMS, format!("no managed agent named 'nobody-such' in session '{session}'").as_str());
        check("hub_search", hub("hub_search", serde_json::json!({"grep": []})),
            ERR_INVALID_PARAMS, "grep must be a non-empty array of search terms");
        for m in ["hub_msg_archive", "hub_msg_restore", "hub_msg_purge"] {
            check(m, hub(m, serde_json::json!({"ids": []})), ERR_INVALID_PARAMS, "ids must be a non-empty array");
        }
        for m in ["hub_board_get", "hub_board_note", "hub_board_delete"] {
            check(m, hub(m, serde_json::json!({})), ERR_INVALID_PARAMS, "id required");
        }
        check("hub_board_delete absent", hub("hub_board_delete", serde_json::json!({"id": 999999})),
            ERR_INVALID_PARAMS, "no issue #999999 on this board");
        check("unknown hub method", hub("hub_nope", serde_json::json!({})), ERR_METHOD_NOT_FOUND, "unknown hub method: hub_nope");
        // Two multi-line literals the first census missed (found by the c2
        // conversion): the agent-control arms' identity checks.
        for m in ["hub_agent_interrupt", "hub_agent_stop", "hub_agent_restart"] {
            check(m, hub(m, serde_json::json!({"agent": "nobody-such"})),
                ERR_INVALID_PARAMS, "'nobody-such' is not an agent this app started");
        }

        // ── rpc.rs literals ──────────────────────────────────────────────
        check("git allowlist", rpc("git", serde_json::json!({"subcmd": "rm"})), ERR_INVALID_PARAMS, "git subcommand not allowed: rm");
        let nul = format!("a{}b", char::from(0u8));
        check("git nul", rpc("git", serde_json::json!({"subcmd": "status", "args": [nul]})), ERR_INVALID_PARAMS, "invalid characters in argument");
        check("fs_convert format", rpc("fs_convert", serde_json::json!({"path": "x.pptx", "format": "pdf"})),
            ERR_INVALID_PARAMS, "only html format supported");
        check("fs_convert ext", rpc("fs_convert", serde_json::json!({"path": "x.docx"})), ERR_INVALID_PARAMS, "unsupported file type: .docx");
        check("resize_pane", rpc("resize_pane", serde_json::json!({})), ERR_INTERNAL, "resize_pane handled elsewhere");
        check("unknown method", rpc("nope", serde_json::json!({})), ERR_METHOD_NOT_FOUND, "unknown method: nope");

        // `restart failed: {e}` needs a live tmux session with a managed agent
        // whose relaunch fails — not reachable here, so the literal is pinned
        // at the source with its code.
        let src = include_str!("hub_rpc.rs");
        assert!(src.contains(r#"Response::err(id, ERR_INTERNAL, format!("restart failed: {e}"))"#)
            || src.contains(r#"RpcError::Internal(format!("restart failed: {e}"))"#),
            "the restart-failed literal moved or changed");
    }
}
