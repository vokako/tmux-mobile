use crate::fs as rfs;
use crate::tmux;
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::{accept_async_with_config, tungstenite::Message, tungstenite::protocol::WebSocketConfig};

// ─── Wire framing for the encrypted binary path ──────────────────────────
// Encrypted messages now travel as WebSocket BINARY frames. The first byte
// of the *plaintext* (post-decrypt) tells the receiver how to decode the
// rest:
//   0x00 = raw UTF-8 JSON (backward compatible with the old base64 path)
//   0x01 = raw deflate (RFC 1951) of UTF-8 JSON
//
// This avoids paying base64's 33% overhead and lets large pane snapshots
// ride deflate's LZ77 window, which collapses inter-frame redundancy by
// 20–50× in practice. Plaintext-token connections (no Web Crypto) keep
// using TEXT frames without framing.
pub const WIRE_PLAIN_JSON: u8 = 0x00;
pub const WIRE_DEFLATE_JSON: u8 = 0x01;
// Below this size, deflate's overhead (header + dict warm-up) makes the
// output bigger than the input. Skip compression for small payloads.
pub const COMPRESS_MIN_BYTES: usize = 256;

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
    /// Desired-roster employees for `room`: `{ "employees": [...] }`.
    fn employees(&self, room: &str) -> serde_json::Value;
    /// Seed an employee into `room`'s desired roster (used by the supervisor).
    fn seed_employee(&self, room: &str, name: &str, spec: &serde_json::Value) -> Result<(), String>;
    /// Raw employee list for `room` as `(name, spec, state)` for the
    /// supervisor's reconcile loop.
    fn employee_specs(&self, room: &str) -> Vec<(String, serde_json::Value, String)>;
    /// Start a team for `workspace`: derive its room (= workspace slug), seed
    /// the default roster, and launch agents into a per-workspace tmux session.
    /// Idempotent per room. Returns `{ room, started, workspace }`.
    fn start_team(&self, workspace: &str) -> serde_json::Value;
    /// Stop a team: kill its tmux session and forget it (the chat log persists
    /// in the db). Returns true if the room was known.
    fn close_team(&self, room: &str) -> bool;
    /// All known teams: `[{ room, workspace, session, started, agents }]`.
    fn teams(&self) -> serde_json::Value;
    /// The default workspace to offer in the UI when none is chosen (the
    /// current terminal session's cwd if known, else the user's home).
    fn default_workspace(&self) -> String;
    /// A receiver of newly-broadcast messages across ALL rooms, each
    /// pre-serialized to a JSON string (the `room` field is inside each
    /// message). The client filters to the team currently in view.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String>;
}

pub type OptTeam = Option<Arc<dyn TeamBridge>>;

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
const DL_TOKEN_TTL_SECS: u64 = 60;

fn sign_download(token: &str, path: &str, ts: u64) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).unwrap();
    mac.update(format!("dl:{}:{}", path, ts).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn verify_download(token: &str, path: &str, ts: u64, sig: &str) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    if now.saturating_sub(ts) > DL_TOKEN_TTL_SECS { return false; }
    sign_download(token, path, ts) == sig
}

// WebSocket frame / message limits. A legitimate `fs_upload` can carry a
// file up to fs::MAX_READ_SIZE (50 MB) inside a base64 string (~67 MB text),
// plus JSON envelope + encryption overhead. 80 MB accommodates that with
// margin; anything bigger is almost certainly malformed or abusive.
// tokio-tungstenite's default (64 MB) is too small for a max-size upload,
// and without an explicit cap an attacker could force per-connection buffer
// growth up to that limit on every frame.
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

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn provided_token_matches(provided: &str, token: &str) -> bool {
    !provided.is_empty() && provided == token
}

/// Encode a JSON string into the wire plaintext (pre-encryption) byte
/// stream: a 1-byte framing tag followed by the body. Compresses with
/// raw deflate (level=1) when the JSON is large enough to benefit; falls
/// back to plain bytes otherwise.
///
/// "Level 1" picks the speed end of zlib's spectrum — it's typically
/// ~3× faster than level 6 and gives up only a small ratio (the gains
/// come almost entirely from LZ77 back-references, which level 1 still
/// finds aggressively). Pane snapshots compress 20–50× even at level 1.
pub fn encode_wire_payload(json: &str) -> Vec<u8> {
    let bytes = json.as_bytes();
    if bytes.len() < COMPRESS_MIN_BYTES {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(WIRE_PLAIN_JSON);
        out.extend_from_slice(bytes);
        return out;
    }
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;
    let mut enc = DeflateEncoder::new(Vec::with_capacity(bytes.len() / 4 + 32), Compression::fast());
    // write_all + finish() never fail on a Vec writer.
    enc.write_all(bytes).expect("deflate write to Vec");
    let compressed = enc.finish().expect("deflate finish");
    // Pathological case: tiny string that deflate inflates due to overhead.
    // Fall back to plain so the receiver doesn't waste cycles on inflate.
    if compressed.len() + 1 >= bytes.len() + 1 {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(WIRE_PLAIN_JSON);
        out.extend_from_slice(bytes);
        return out;
    }
    let mut out = Vec::with_capacity(1 + compressed.len());
    out.push(WIRE_DEFLATE_JSON);
    out.extend_from_slice(&compressed);
    out
}

/// Decode a wire plaintext (post-decrypt) into the original JSON string.
/// Used only for tests / inbound — clients drive the inbound path, so this
/// rarely runs in production server, but keeping the inverse handy keeps
/// the protocol symmetric.
pub fn decode_wire_payload(buf: &[u8]) -> Result<String, String> {
    let (&tag, body) = buf.split_first().ok_or_else(|| "empty wire payload".to_string())?;
    match tag {
        WIRE_PLAIN_JSON => {
            String::from_utf8(body.to_vec()).map_err(|e| format!("plain wire payload not utf-8: {}", e))
        }
        WIRE_DEFLATE_JSON => {
            use flate2::read::DeflateDecoder;
            use std::io::Read;
            let mut dec = DeflateDecoder::new(body);
            let mut out = String::with_capacity(body.len() * 4);
            dec.read_to_string(&mut out).map_err(|e| format!("inflate failed: {}", e))?;
            Ok(out)
        }
        other => Err(format!("unknown wire framing tag: 0x{:02x}", other)),
    }
}

/// Derives AES-256-GCM key from token + nonces using HKDF-SHA256.
fn derive_key(token: &str, server_nonce: &[u8; 16], client_nonce: &[u8; 16]) -> [u8; 32] {
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(server_nonce);
    salt[16..].copy_from_slice(client_nonce);
    let hk = Hkdf::<Sha256>::new(Some(&salt), token.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"tmux-mobile-e2e", &mut key).unwrap();
    key
}

/// Unidirectional cipher half: after auth, the session splits into a
/// recv-side (owned by the receiver loop, decrypts incoming in strict
/// order) and a send-side (owned by the send task, encrypts outgoing in
/// strict order). Splitting lets multiple business tasks run in parallel
/// without fighting over a single cipher's counter.
struct HalfCipher {
    cipher: Aes256Gcm,
    counter: u64,
}

impl HalfCipher {
    fn new(key: &[u8; 32]) -> Self {
        Self { cipher: Aes256Gcm::new_from_slice(key).unwrap(), counter: 0 }
    }
    fn encrypt(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        self.cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext).unwrap()
    }
    fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        self.cipher.decrypt(Nonce::from_slice(&nonce_bytes), ciphertext)
            .map_err(|_| "decryption failed".to_string())
    }
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

// JSON-RPC style request/response

#[derive(Deserialize, Debug)]
struct Request {
    id: Option<u64>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Serialize, Clone)]
struct Response {
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorInfo>,
}

#[derive(Serialize, Clone)]
struct ErrorInfo {
    code: i32,
    message: String,
}

// Error codes
const ERR_PARSE: i32 = -32700;
const ERR_METHOD_NOT_FOUND: i32 = -32601;
const ERR_INVALID_PARAMS: i32 = -32602;
const ERR_INTERNAL: i32 = -32603;
const ERR_AUTH: i32 = -32000;

impl Response {
    fn ok(id: Option<u64>, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }
    fn err(id: Option<u64>, code: i32, message: String) -> Self {
        Self {
            id,
            result: None,
            error: Some(ErrorInfo { code, message }),
        }
    }
}

// Per-connection subscription state: target -> last captured content
type Subscriptions = Arc<Mutex<HashMap<String, String>>>;

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing required param: {}", key))
}

fn handle_request(req: &Request, token: &str) -> Response {
    let id = req.id;
    let p = &req.params;

    match req.method.as_str() {
        "ping" => Response::ok(id, serde_json::json!("pong")),

        "list_sessions" => match tmux::list_sessions() {
            Ok(sessions) => Response::ok(id, serde_json::to_value(&sessions).unwrap()),
            Err(e) => Response::err(id, ERR_INTERNAL, e),
        },

        "list_panes" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::list_panes(session) {
                Ok(panes) => Response::ok(id, serde_json::to_value(&panes).unwrap()),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        // Combined sessions + panes in one round-trip. The Sessions page
        // needs both to render its summary chips (cwd, current command, AI
        // detection) — issuing them as 1 + N RPCs added perceivable latency
        // when N grew beyond a handful. Single tmux call now returns
        // everything; client groups panes by session_name client-side.
        "list_sessions_with_panes" => {
            let sessions = match tmux::list_sessions() {
                Ok(v) => v,
                Err(e) => return Response::err(id, ERR_INTERNAL, e),
            };
            let panes = match tmux::list_all_panes() {
                Ok(v) => v,
                Err(e) => return Response::err(id, ERR_INTERNAL, e),
            };
            Response::ok(id, serde_json::json!({
                "sessions": sessions,
                "panes": panes,
            }))
        }

        "capture_pane" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let lines = p.get("lines").and_then(|v| v.as_u64()).map(|n| n as usize);
            match tmux::capture_pane(target, lines) {
                Ok(output) => Response::ok(id, serde_json::json!({ "output": output })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "send_keys" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let keys = match require_str(p, "keys") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let literal = p.get("literal").and_then(|v| v.as_bool()).unwrap_or(false);
            match tmux::send_keys(target, keys, literal) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "send_command" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let command = match require_str(p, "command") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::send_command(target, command) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        // resize_pane is handled in the connection message loop (needs per-connection state)
        "resize_pane" => Response::err(id, ERR_INTERNAL, "resize_pane handled elsewhere".into()),

        "new_session" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("untitled");
            let path = p.get("path").and_then(|v| v.as_str());
            let command = p.get("command").and_then(|v| v.as_str());
            match tmux::new_session(name, path, command) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "kill_session" => {
            let name = match require_str(p, "name") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::kill_session(name) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "new_window" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::new_window(session) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "kill_window" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::kill_window(target) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "pane_command" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::pane_command(target) {
                Ok(cmd) => Response::ok(id, serde_json::json!({ "command": cmd })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "set_socket" => {
            let socket = p
                .get("socket")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tmux::set_socket(socket);
            Response::ok(id, serde_json::json!({ "ok": true }))
        }

        "get_bookmarks" => {
            let bookmarks = crate::config::get_bookmarks();
            Response::ok(id, serde_json::json!({ "bookmarks": bookmarks }))
        }

        "save_bookmarks" => {
            let bookmarks: Vec<String> = p
                .get("bookmarks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            match crate::config::save_bookmarks(&bookmarks) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "get_prefs" => {
            Response::ok(id, crate::config::get_prefs())
        }

        "set_pref" => {
            let key = match require_str(p, "key") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let value = p.get("value").cloned().unwrap_or(serde_json::Value::Null);
            match crate::config::set_prefs(key, value) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_cwd" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::get_cwd(session) {
                Ok(path) => Response::ok(id, serde_json::json!({ "path": path })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_list" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let show_hidden = p
                .get("show_hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match rfs::list_dir(path, show_hidden) {
                Ok(entries) => {
                    Response::ok(id, serde_json::json!({ "entries": entries, "path": path }))
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_stat" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::stat_file(path) {
                Ok(stat) => Response::ok(id, serde_json::to_value(&stat).unwrap()),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_read" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::read_file(path) {
                Ok(content) => Response::ok(id, serde_json::json!({ "content": content })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_write" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            // Allow empty content (creating empty files is valid)
            let content = p.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match rfs::write_file(path, content) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_mkdir" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::create_dir(path) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_delete" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::delete_path(path) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_rename" => {
            let from = match require_str(p, "from") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let to = match require_str(p, "to") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::rename_path(from, to) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_download" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::download_file(path) {
                Ok((name, data)) => {
                    Response::ok(id, serde_json::json!({ "name": name, "data": data }))
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "fs_download_url" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let sig = sign_download(token, path, ts);
            let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let qs = format!("/dl?path={}&ts={}&sig={}", urlencoding::encode(path), ts, sig);
            Response::ok(id, serde_json::json!({ "url": qs, "name": name }))
        }

        "fs_upload" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let data = match require_str(p, "data") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match rfs::upload_file(path, data) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        "git" => {
            let subcmd = match require_str(p, "subcmd") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let args: Vec<String> = p
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let cwd = p.get("cwd").and_then(|v| v.as_str());

            const ALLOWED: &[&str] = &[
                "status", "diff", "log", "show", "branch", "rev-parse", "push", "add", "commit", "restore",
            ];
            if !ALLOWED.contains(&subcmd) {
                return Response::err(id, ERR_INVALID_PARAMS, format!("git subcommand not allowed: {}", subcmd));
            }
            // Reject args containing shell metacharacters
            for arg in &args {
                if arg.contains(|c: char| matches!(c, ';' | '&' | '$' | '`' | '|' | '>' | '<' | '\n' | '\r')) {
                    return Response::err(id, ERR_INVALID_PARAMS, "invalid characters in argument".into());
                }
            }

            let mut child = std::process::Command::new("git");
            child.arg(&subcmd);
            child.args(&args);
            if let Some(d) = cwd {
                child.current_dir(d);
            }
            match child.output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    Response::ok(
                        id,
                        serde_json::json!({ "stdout": stdout, "stderr": stderr, "code": output.status.code() }),
                    )
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e.to_string()),
            }
        }

        "fs_convert" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let format = p.get("format").and_then(|v| v.as_str()).unwrap_or("html");
            if format != "html" {
                return Response::err(id, ERR_INVALID_PARAMS, "only html format supported".into());
            }
            let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let script = match ext.as_str() {
                "pptx" => r#"import sys,pptx,html as h;p=pptx.Presentation(sys.argv[1]);o=""
for i,s in enumerate(p.slides):
 o+=f"<div style='border:1px solid #ccc;border-radius:8px;padding:16px;margin:12px 0'><b>Slide {i+1}</b><br>"
 for sh in s.shapes:
  if sh.has_text_frame:
   for pa in sh.text_frame.paragraphs:
    t=h.escape("".join(r.text for r in pa.runs))
    if t.strip():o+=f"<p>{t}</p>"
  if sh.has_table:
   o+="<table border=1 cellpadding=4 style='border-collapse:collapse;margin:8px 0'>"
   for row in sh.table.rows:
    o+="<tr>"+"".join(f"<td>{h.escape(c.text)}</td>" for c in row.cells)+"</tr>"
   o+="</table>"
 o+="</div>"
print(o)"#.to_string(),
                _ => return Response::err(id, ERR_INVALID_PARAMS, format!("unsupported file type: .{}", ext)),
            };
            match std::process::Command::new("python3").arg("-c").arg(&script).arg(path).output() {
                Ok(output) => {
                    if output.status.success() {
                        let html = String::from_utf8_lossy(&output.stdout).to_string();
                        Response::ok(id, serde_json::json!({ "html": html }))
                    } else {
                        let err = String::from_utf8_lossy(&output.stderr).to_string();
                        Response::err(id, ERR_INTERNAL, err)
                    }
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e.to_string()),
            }
        }

        _ => Response::err(
            id,
            ERR_METHOD_NOT_FOUND,
            format!("unknown method: {}", req.method),
        ),
    }
}

// Subscription polling task: captures pane content and sends diffs
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

/// Dispatch `team_*` RPC methods to the in-process bus bridge. Returns a
/// method-not-found error when no bus is wired (mobile builds, or desktop with
/// team disabled) so the client can degrade gracefully (hide the Team tab).
fn handle_team_request(req: &Request, team: Option<&dyn TeamBridge>) -> Response {
    let id = req.id;
    let p = &req.params;
    let Some(bus) = team else {
        return Response::err(id, ERR_METHOD_NOT_FOUND, "team bus not available on this server".into());
    };
    // Most methods operate on a specific team `room` (the phone's active team).
    let room = p.get("room").and_then(|v| v.as_str()).unwrap_or("");
    match req.method.as_str() {
        // Bus availability + the team list + a default workspace to pre-fill.
        // Team-agnostic, so the Team tab can render the switcher before picking
        // an active room.
        "team_status" => Response::ok(id, serde_json::json!({
            "available": true,
            "teams": bus.teams().get("teams").cloned().unwrap_or(serde_json::json!([])),
            "default_workspace": bus.default_workspace(),
        })),
        "team_teams" => Response::ok(id, bus.teams()),
        "team_history" => {
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
            Response::ok(id, bus.history(room, limit))
        }
        "team_roster" => Response::ok(id, bus.roster(room)),
        "team_employees" => Response::ok(id, bus.employees(room)),
        // Operator action: spin up a team in `workspace` (its room = the
        // workspace slug). Idempotent per room; returns { room, started, workspace }.
        "team_start_team" => {
            let workspace = p.get("workspace").and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| bus.default_workspace());
            Response::ok(id, bus.start_team(&workspace))
        }
        // Stop a team: kill its tmux session, forget it (chat log persists).
        "team_close_team" => {
            if room.is_empty() {
                return Response::err(id, ERR_INVALID_PARAMS, "missing required param: room".into());
            }
            Response::ok(id, serde_json::json!({ "closed": bus.close_team(room) }))
        }
        "team_post" => {
            if room.is_empty() {
                return Response::err(id, ERR_INVALID_PARAMS, "missing required param: room".into());
            }
            let body = match require_str(p, "body") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            // The human is "you" on the phone; default sender name "human"
            // matches team's dashboard/CLI convention so the operator shows
            // up consistently across surfaces.
            let from = p.get("from").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("human");
            // Mirror the dashboard: an @mention implies a reply is wanted
            // unless the client says otherwise.
            let requires_reply = p
                .get("requires_reply")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| body.contains('@'));
            match bus.post(room, from, body, requires_reply) {
                Ok(msg) => Response::ok(id, serde_json::json!({ "message": msg })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }
        other => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown team method: {}", other)),
    }
}

/// Push newly-broadcast team messages to this client as `team_message`
/// notifications, so the Team tab updates live without polling. Mirrors the
/// dashboard's SSE stream, but rides the existing encrypted Outbound channel.
async fn team_push_loop(out_tx: tokio::sync::mpsc::UnboundedSender<Outbound>, team: Arc<dyn TeamBridge>) {
    let mut rx = team.subscribe();
    loop {
        match rx.recv().await {
            Ok(msg_json) => {
                let frame = serde_json::json!({
                    "id": null,
                    "method": "team_message",
                    "params": { "message": serde_json::from_str::<serde_json::Value>(&msg_json).unwrap_or(serde_json::Value::Null) },
                });
                if out_tx.send(Outbound::Encrypted(serde_json::to_string(&frame).unwrap())).is_err() {
                    return; // send task gone
                }
            }
            // Lagged: the client re-syncs via team_history on demand, so just
            // keep receiving from the current tail.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

fn handle_subscribe(params: &serde_json::Value, subs: &mut HashMap<String, String>) -> Response {
    let target = match require_str(params, "target") {
        Ok(s) => s,
        Err(e) => return Response::err(None, ERR_INVALID_PARAMS, e),
    };
    subs.insert(target.to_string(), String::new());
    // Record "last opened from tmux-mobile" for MRU sorting on the Sessions
    // page. Target is "name:window.pane"; the session name is everything
    // before the first colon.
    let session_name = target.split(':').next().unwrap_or(target);
    if !session_name.is_empty() {
        let _ = crate::config::touch_session(session_name);
    }
    Response::ok(None, serde_json::json!({ "subscribed": target }))
}

fn handle_unsubscribe(params: &serde_json::Value, subs: &mut HashMap<String, String>) -> Response {
    let target = match require_str(params, "target") {
        Ok(s) => s,
        Err(e) => return Response::err(None, ERR_INVALID_PARAMS, e),
    };
    subs.remove(target);
    Response::ok(None, serde_json::json!({ "unsubscribed": target }))
}

/// Find the end of the HTTP header block (offset of the CRLFCRLF terminator).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse a `Range: bytes=N-` request header out of a raw header block.
/// Only the open-ended single-range form is supported — that's the only
/// form our resume client emits. Any other form is ignored (a server is
/// allowed to ignore Range and answer 200 with the full body).
fn parse_range_start(req: &str) -> Option<u64> {
    for line in req.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        if !k.trim().eq_ignore_ascii_case("range") {
            continue;
        }
        let spec = v.trim().strip_prefix("bytes=")?;
        let (start, rest) = spec.split_once('-')?;
        if !rest.is_empty() {
            return None; // "N-M" / "-N" suffix form: ignore, serve full body
        }
        return start.parse().ok();
    }
    None
}

/// Decide whether a peeked request prelude is an HTTP /dl download rather
/// than a WebSocket upgrade. Both arrive as HTTP GET; we look for the
/// "/dl?" path segment in the request LINE only, tolerating a reverse-proxy
/// path prefix (e.g. "GET /tmux/dl?path=..." when the proxy doesn't strip
/// its location prefix).
fn looks_like_dl_request(prelude: &[u8]) -> bool {
    if !prelude.starts_with(b"GET ") {
        return false;
    }
    let line_end = prelude
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(prelude.len());
    prelude[..line_end].windows(4).any(|w| w == b"/dl?")
}

async fn handle_http_download<S>(mut stream: S, addr: SocketAddr, token: Arc<String>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

    // Read until the full header block has arrived. Reverse proxies often
    // deliver the request line and headers across multiple TCP segments;
    // a single read() truncates the query string mid-signature and rejects
    // a perfectly valid request with 403. (Direct LAN/Tailscale clients
    // virtually always deliver everything in one segment, which is why
    // this only ever bit through a proxy.)
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > 16 * 1024 {
            return; // oversized header block — not a legitimate /dl request
        }
        match stream.read(&mut tmp).await {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => return,
        }
    };
    let req = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let first_line = req.lines().next().unwrap_or("");

    // Parse "GET <path>/dl?path=...&ts=...&sig=... HTTP/1.1". Locate the
    // "/dl?" segment instead of assuming it starts the path, so a proxy
    // prefix doesn't break query extraction.
    let url_part = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = match url_part.find("/dl?") {
        Some(i) => &url_part[i + 4..],
        None => "",
    };
    let params: HashMap<&str, &str> = query.split('&')
        .filter_map(|p| p.split_once('='))
        .collect();

    let path = match params.get("path") {
        Some(p) => urlencoding::decode(p).unwrap_or_default().to_string(),
        None => { let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await; return; }
    };
    let ts: u64 = params.get("ts").and_then(|s| s.parse().ok()).unwrap_or(0);
    let sig = params.get("sig").unwrap_or(&"");

    if !verify_download(&token, &path, ts, sig) {
        eprintln!("🚫 HTTP download rejected for {} (invalid sig)", addr);
        let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").await;
        let _ = stream.flush().await;
        return;
    }

    // Read file and stream response
    let file_path = std::path::Path::new(&path);
    let metadata = match std::fs::metadata(file_path) {
        Ok(m) => m,
        Err(_) => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
            let _ = stream.flush().await;
            return;
        }
    };
    let name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let size = metadata.len();

    // Range support: lets the client RESUME an interrupted transfer instead
    // of restarting from byte 0. Critical through reverse proxies on the
    // public internet, where long-lived large responses get cut by proxy
    // idle/total timeouts — without resume, a 100 MB file that dies at 95%
    // restarts from scratch and may never complete.
    // `Connection: close` matters behind reverse proxies: we serve one
    // request per TCP connection and then drop it. Without the header,
    // HTTP/1.1 defaults to keep-alive and the proxy may pool the (already
    // closed) backend connection, surfacing as intermittent 502s on the
    // next download.
    let range_start = parse_range_start(&req).filter(|&s| s > 0 && s < size);
    let header = match range_start {
        Some(start) => format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            name, size - start, start, size - 1, size
        ),
        None => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            name, size
        ),
    };
    if stream.write_all(header.as_bytes()).await.is_err() { return; }

    // Stream file in chunks
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return,
    };
    if let Some(start) = range_start {
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return;
        }
    }
    let mut chunk = vec![0u8; 65536];
    loop {
        let n = match file.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if stream.write_all(&chunk[..n]).await.is_err() { break; }
    }
    // Flush any data still sitting in BufStream's write buffer — on drop
    // that buffer is discarded and the tail of the file would be lost.
    let _ = stream.flush().await;
}

pub async fn handle_connection(stream: TcpStream, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker, grace_secs: u64, team: OptTeam) {
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

    handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, resize_tracker, conn_id, conn_started_at, grace_secs, team).await;
}

async fn handle_connection_ws<S>(ws_stream: tokio_tungstenite::WebSocketStream<S>, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker, conn_id: u64, conn_started_at: std::time::Instant, grace_secs: u64, team: OptTeam)
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
                        handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, control_mgr, conn_id, conn_started_at, grace, team_c).await;
                    }
                    Err(e) => eprintln!("❌ TLS handshake failed for {}: {}", addr, e),
                }
            });
        } else {
            tokio::spawn(handle_connection(stream, addr, token, machine_id, auth_tracker, control_mgr, grace, team_c));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── team WS proxy dispatch ─────────────────────────────────────────
    // A tiny in-memory TeamBridge stand-in so we can exercise
    // handle_team_request without pulling in the real (desktop-only) bus.
    struct MockAgora;
    impl TeamBridge for MockAgora {
        fn history(&self, _room: &str, limit: i64) -> serde_json::Value {
            serde_json::json!({ "messages": [], "echo_limit": limit })
        }
        fn roster(&self, _room: &str) -> serde_json::Value {
            serde_json::json!({ "roster": [{ "name": "worker", "status": "waiting" }] })
        }
        fn post(&self, room: &str, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({ "room": room, "from": from, "body": body, "requires_reply": requires_reply }))
        }
        fn employees(&self, _room: &str) -> serde_json::Value {
            serde_json::json!({ "employees": [] })
        }
        fn seed_employee(&self, _room: &str, _name: &str, _spec: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn employee_specs(&self, _room: &str) -> Vec<(String, serde_json::Value, String)> {
            Vec::new()
        }
        fn start_team(&self, workspace: &str) -> serde_json::Value {
            serde_json::json!({ "started": true, "room": "ws", "workspace": workspace })
        }
        fn close_team(&self, _room: &str) -> bool {
            true
        }
        fn teams(&self) -> serde_json::Value {
            serde_json::json!({ "teams": [] })
        }
        fn default_workspace(&self) -> String {
            "/tmp/ws".to_string()
        }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
            let (tx, rx) = tokio::sync::broadcast::channel(4);
            let _ = tx; // keep the sender alive only as long as needed for the type
            rx
        }
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request { id: Some(1), method: method.to_string(), params }
    }

    #[test]
    fn team_request_without_bus_is_method_not_found() {
        let r = handle_team_request(&req("team_roster", serde_json::json!({})), None);
        assert_eq!(r.error.as_ref().map(|e| e.code), Some(ERR_METHOD_NOT_FOUND));
    }

    #[test]
    fn team_roster_returns_roster() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_roster", serde_json::json!({ "room": "ws" })), Some(&bus));
        let roster = r.result.unwrap();
        assert_eq!(roster["roster"][0]["name"], "worker");
    }

    #[test]
    fn team_post_requires_room_and_body_and_infers_reply_from_mention() {
        let bus = MockAgora;
        // Missing room → invalid params.
        let no_room = handle_team_request(&req("team_post", serde_json::json!({ "body": "hi" })), Some(&bus));
        assert_eq!(no_room.error.as_ref().map(|e| e.code), Some(ERR_INVALID_PARAMS));

        // Missing body → invalid params.
        let bad = handle_team_request(&req("team_post", serde_json::json!({ "room": "ws" })), Some(&bus));
        assert_eq!(bad.error.as_ref().map(|e| e.code), Some(ERR_INVALID_PARAMS));

        // @mention with no explicit flag → requires_reply inferred true.
        let mentioned = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "@worker do X" })),
            Some(&bus),
        );
        assert_eq!(mentioned.result.unwrap()["message"]["requires_reply"], true);

        // Plain broadcast → requires_reply inferred false; default sender "human".
        let plain = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "hello team" })),
            Some(&bus),
        );
        let msg = plain.result.unwrap();
        assert_eq!(msg["message"]["requires_reply"], false);
        assert_eq!(msg["message"]["from"], "human");

        // Explicit requires_reply=false overrides the @mention inference.
        let forced = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "@worker fyi", "requires_reply": false })),
            Some(&bus),
        );
        assert_eq!(forced.result.unwrap()["message"]["requires_reply"], false);
    }

    #[test]
    fn team_status_lists_teams_without_a_room() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_status", serde_json::json!({})), Some(&bus));
        let s = r.result.unwrap();
        assert_eq!(s["available"], true);
        assert!(s["teams"].is_array());
    }

    #[test]
    fn team_unknown_method_is_method_not_found() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_bogus", serde_json::json!({})), Some(&bus));
        assert_eq!(r.error.as_ref().map(|e| e.code), Some(ERR_METHOD_NOT_FOUND));
    }

    // ─── encode/decode_wire_payload roundtrip ────────────────────────────

    #[test]
    fn wire_small_payload_skips_compression() {
        let json = r#"{"id":1,"result":"pong"}"#;
        let encoded = encode_wire_payload(json);
        assert_eq!(encoded[0], WIRE_PLAIN_JSON, "small payload must stay uncompressed");
        assert_eq!(&encoded[1..], json.as_bytes());
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_large_payload_compresses() {
        // 4 KB of repeated content — perfect deflate target.
        let json = format!(r#"{{"content":"{}"}}"#, "ABCDEFGH".repeat(500));
        assert!(json.len() > COMPRESS_MIN_BYTES);
        let encoded = encode_wire_payload(&json);
        assert_eq!(encoded[0], WIRE_DEFLATE_JSON, "large payload must be compressed");
        assert!(
            encoded.len() < json.len() / 5,
            "compression ratio too weak: {} → {}",
            json.len(),
            encoded.len()
        );
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_payload_with_ansi_codes_roundtrip() {
        // Realistic pane snapshot: ANSI SGR escape sequences should pass
        // through (deflate is byte-clean; we just don't want any UTF-8 mishap).
        let json = format!(
            r#"{{"content":"{}"}}"#,
            "\u{001b}[38;2;255;100;200mhello\u{001b}[0m world\n".repeat(60)
        );
        assert!(json.len() > COMPRESS_MIN_BYTES);
        let encoded = encode_wire_payload(&json);
        assert_eq!(encoded[0], WIRE_DEFLATE_JSON);
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_payload_unicode_roundtrip() {
        // 多字节 UTF-8 不能被 deflate / inflate 弄坏。
        let json = format!(r#"{{"msg":"{}"}}"#, "中文 日本語 한국어 🎉 ".repeat(50));
        let encoded = encode_wire_payload(&json);
        let decoded = decode_wire_payload(&encoded).unwrap();
        assert_eq!(decoded, json);
    }

    #[test]
    fn wire_pathological_random_skips_compression() {
        // High-entropy data that deflate can't compress: encode_wire_payload
        // should detect "compressed >= original" and emit plain instead.
        let mut rng_bytes = vec![0u8; 600];
        for (i, b) in rng_bytes.iter_mut().enumerate() {
            // Deterministic pseudo-random pattern, byte-clean ASCII so JSON-like.
            *b = ((i * 2654435761) % 94 + 32) as u8;
        }
        let json = String::from_utf8(rng_bytes).unwrap();
        let encoded = encode_wire_payload(&json);
        // Either compressed or plain is acceptable — both must roundtrip.
        // The important invariant is encoded.len() < 2 * json.len() (no
        // runaway expansion) and lossless decode.
        assert!(encoded.len() < json.len() * 2);
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_decode_rejects_unknown_tag() {
        let bogus = vec![0xff, 1, 2, 3];
        let err = decode_wire_payload(&bogus).unwrap_err();
        assert!(err.contains("unknown wire framing tag"), "got: {}", err);
    }

    #[test]
    fn wire_decode_rejects_empty() {
        let err = decode_wire_payload(&[]).unwrap_err();
        assert!(err.contains("empty"), "got: {}", err);
    }

    // ─── Full encryption + framing roundtrip ─────────────────────────────

    fn make_paired_ciphers() -> (HalfCipher, HalfCipher) {
        // Send half writes, recv half reads — both initialised from the
        // same key. (In production the two sides live on different
        // hosts; for tests we just need any matched pair.)
        let key = [0x42u8; 32];
        (HalfCipher::new(&key), HalfCipher::new(&key))
    }

    #[test]
    fn encrypted_compressed_roundtrip_typical_pane_snapshot() {
        // Mimic a `pane_output` notification with a 50-line pane payload.
        let pane = (0..50)
            .map(|i| format!("\u{001b}[38;5;{}m line {} content content content\u{001b}[0m", (i % 200) + 16, i))
            .collect::<Vec<_>>()
            .join("\n");
        let msg = serde_json::json!({
            "id": null,
            "method": "pane_output",
            "params": {
                "target": "test:0.0",
                "content": pane,
                "cursor": {"x": 4, "y": 24, "w": 80, "h": 24, "t": 0}
            }
        });
        let json = serde_json::to_string(&msg).unwrap();

        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let plaintext = encode_wire_payload(&json);
        let ciphertext = send_c.encrypt(&plaintext);
        // Verify wire size advantage: ciphertext should be smaller than the
        // old base64(encrypted(json)) path. base64 inflates by ~4/3, plus
        // we still have to encrypt the full uncompressed JSON.
        assert!(
            ciphertext.len() < (json.len() * 4 / 3) / 4,
            "expected at least 4× shrink vs base64; got json={} ct={}",
            json.len(),
            ciphertext.len()
        );

        let recovered_pt = recv_c.decrypt(&ciphertext).expect("decrypt ok");
        let recovered_json = decode_wire_payload(&recovered_pt).expect("decode ok");
        assert_eq!(recovered_json, json);
    }

    #[test]
    fn encrypted_small_message_roundtrip_no_compression() {
        let json = r#"{"id":42,"result":{"ok":true}}"#;
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let plaintext = encode_wire_payload(json);
        // Verify framing: small → plain.
        assert_eq!(plaintext[0], WIRE_PLAIN_JSON);
        let ciphertext = send_c.encrypt(&plaintext);
        let recovered_pt = recv_c.decrypt(&ciphertext).expect("decrypt ok");
        let recovered_json = decode_wire_payload(&recovered_pt).expect("decode ok");
        assert_eq!(recovered_json, json);
    }

    #[test]
    fn cipher_counter_advances_strictly() {
        // Two consecutive encrypts must produce different ciphertexts even
        // for the same plaintext (because the nonce counter advances). And
        // the recv half must decrypt them in matching order.
        let pt = b"identical message";
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let ct1 = send_c.encrypt(pt);
        let ct2 = send_c.encrypt(pt);
        assert_ne!(ct1, ct2, "GCM must emit distinct ciphertext per nonce");
        assert_eq!(recv_c.decrypt(&ct1).unwrap(), pt);
        assert_eq!(recv_c.decrypt(&ct2).unwrap(), pt);
    }

    #[test]
    fn cipher_rejects_out_of_order_decrypt() {
        // If the receiver's counter is ahead of the actual ciphertext's
        // nonce, GCM authentication fails — that's the property we rely on
        // for replay protection.
        let pt = b"hello";
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let ct1 = send_c.encrypt(pt);
        let _ct2 = send_c.encrypt(pt);
        // Skip ahead on the receive side, then try to decrypt ct1.
        let _ = recv_c.decrypt(&_ct2); // advances counter past ct1
        assert!(recv_c.decrypt(&ct1).is_err(), "ct1 must fail under wrong nonce");
    }

    // ─── HTTP /dl request parsing ────────────────────────────────────────

    #[test]
    fn range_header_open_ended_parses() {
        let req = "GET /dl?path=x HTTP/1.1\r\nHost: h\r\nRange: bytes=12345-\r\n";
        assert_eq!(parse_range_start(req), Some(12345));
    }

    #[test]
    fn range_header_case_insensitive() {
        let req = "GET /dl?path=x HTTP/1.1\r\nrange: bytes=7-\r\n";
        assert_eq!(parse_range_start(req), Some(7));
    }

    #[test]
    fn range_header_bounded_form_ignored() {
        // "N-M" and suffix forms are not emitted by our client; server
        // falls back to a full 200 response.
        assert_eq!(parse_range_start("Range: bytes=0-499\r\n"), None);
        assert_eq!(parse_range_start("Range: bytes=-500\r\n"), None);
        assert_eq!(parse_range_start("GET / HTTP/1.1\r\nHost: h\r\n"), None);
    }

    #[test]
    fn dl_detection_with_and_without_proxy_prefix() {
        assert!(looks_like_dl_request(b"GET /dl?path=a&ts=1&sig=b HTTP/1.1\r\n"));
        // Reverse proxy that forwards its location prefix unstripped.
        assert!(looks_like_dl_request(b"GET /tmux/dl?path=a HTTP/1.1\r\n"));
        assert!(!looks_like_dl_request(b"GET / HTTP/1.1\r\nUpgrade: websocket\r\n"));
        // "/dl?" appearing only in a header (not the request line) must not match.
        assert!(!looks_like_dl_request(b"GET /ws HTTP/1.1\r\nReferer: /dl?x\r\n"));
        assert!(!looks_like_dl_request(b"POST /dl?path=a HTTP/1.1\r\n"));
    }

    #[test]
    fn header_end_detection() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: h\r\n\r\nbody"), Some(23));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: h\r\n"), None);
    }

    #[test]
    fn compression_ratio_demo_pane_snapshot() {
        // Demonstrates the bandwidth win on a realistic snapshot. Not a
        // strict assertion (avoid making future deflate library updates
        // flap the test), just bounds.
        let lines: Vec<String> = (0..24)
            .map(|i| format!("$ command --flag-{}={}\u{001b}[0m output for row {}", i, i * 7, i))
            .collect();
        let snapshot = lines.join("\n");
        let json = serde_json::json!({"content": snapshot}).to_string();
        let plaintext = encode_wire_payload(&json);
        eprintln!(
            "[compression demo] json={} bytes, wire={} bytes, ratio={:.2}×",
            json.len(),
            plaintext.len(),
            json.len() as f64 / plaintext.len() as f64
        );
        assert!(plaintext.len() < json.len(), "wire payload should shrink");
    }
}

