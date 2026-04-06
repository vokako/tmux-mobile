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
use tokio_tungstenite::{accept_async, tungstenite::Message};

// Brute-force protection: track failed auth attempts per IP
type AuthTracker = Arc<Mutex<HashMap<IpAddr, (u32, tokio::time::Instant)>>>;

// Track which windows each connection has resized, so we can restore on disconnect.
// Key = conn_id, Value = set of "session:window" targets that were resized.
type ResizeTracker = Arc<std::sync::Mutex<HashMap<u64, std::collections::HashSet<String>>>>;

static CONN_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

const MAX_AUTH_FAILURES: u32 = 5;
const AUTH_LOCKOUT_SECS: u64 = 60;

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

/// Per-connection cipher state for encrypting/decrypting messages.
struct SessionCipher {
    cipher: Aes256Gcm,
    send_counter: u64,
    recv_counter: u64,
}

impl SessionCipher {
    fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(key).unwrap(),
            send_counter: 0,
            recv_counter: 0,
        }
    }

    fn encrypt(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.send_counter.to_be_bytes());
        self.send_counter += 1;
        self.cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .unwrap()
    }

    fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.recv_counter.to_be_bytes());
        self.recv_counter += 1;
        self.cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext)
            .map_err(|_| "decryption failed".to_string())
    }
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

fn handle_request(req: &Request) -> Response {
    let id = req.id;
    let p = &req.params;

    match req.method.as_str() {
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
            let content = match require_str(p, "content") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
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
                if arg.contains(|c: char| matches!(c, '|' | ';' | '&' | '$' | '`' | '\n')) {
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

        _ => Response::err(
            id,
            ERR_METHOD_NOT_FOUND,
            format!("unknown method: {}", req.method),
        ),
    }
}

// Subscription polling task: captures pane content and sends diffs
async fn subscription_loop(
    sender: Arc<Mutex<dyn futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Unpin>>,
    subs: Subscriptions,
    cipher: Arc<Mutex<Option<SessionCipher>>>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
    let mut fail_counts: HashMap<String, u32> = HashMap::new();
    loop {
        interval.tick().await;
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
            let (new_content, cursor, trailing_trimmed) = match tokio::task::spawn_blocking(move || {
                let cursor = tmux::cursor_info(&t2).unwrap_or((0, 0, 24, 80));
                let (content, trailing) = tmux::capture_pane_with_width(&t, None, cursor.3)?;
                Ok::<_, String>((content, cursor, trailing))
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
                    if *count >= 15 {
                        subs.lock().await.remove(&target);
                        fail_counts.remove(&target);
                    }
                    continue;
                }
            };
            let state_key = format!("{}\x00{},{},{},{}", new_content, cursor.0, cursor.1, cursor.2, cursor.3);
            if state_key == prev {
                continue;
            }
            subs.lock()
                .await
                .insert(target.clone(), state_key);
            let content_changed = !prev.is_empty()
                && prev.split('\x00').next().unwrap_or("") != new_content;
            // Send raw tmux cursor position + trailing trimmed count for xterm.js row mapping
            let cursor_obj = serde_json::json!({ "x": cursor.0, "y": cursor.1, "w": cursor.3, "h": cursor.2, "t": trailing_trimmed });
            let msg = if content_changed || prev.is_empty() {
                serde_json::json!({
                    "id": null,
                    "method": "pane_output",
                    "params": { "target": target, "content": new_content, "cursor": cursor_obj }
                })
            } else {
                serde_json::json!({
                    "id": null,
                    "method": "pane_output",
                    "params": { "target": target, "cursor": cursor_obj }
                })
            };
            let text = serde_json::to_string(&msg).unwrap();
            let out = {
                let mut c = cipher.lock().await;
                if let Some(ref mut sc) = *c {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.encode(sc.encrypt(text.as_bytes()))
                } else {
                    text
                }
            };
            let mut tx = sender.lock().await;
            if tx.send(Message::Text(out.into())).await.is_err() {
                return;
            }
        }
    }
}

fn handle_subscribe(params: &serde_json::Value, subs: &mut HashMap<String, String>) -> Response {
    let target = match require_str(params, "target") {
        Ok(s) => s,
        Err(e) => return Response::err(None, ERR_INVALID_PARAMS, e),
    };
    subs.insert(target.to_string(), String::new());
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

async fn handle_connection(stream: TcpStream, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker) {
    println!("📱 Client connected: {}", addr);

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

    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("❌ WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, resize_tracker).await;
}

async fn handle_connection_ws<S>(ws_stream: tokio_tungstenite::WebSocketStream<S>, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker)
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
    let sender: Arc<Mutex<dyn futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Unpin>> = Arc::new(Mutex::new(ws_sender));
    let subs: Subscriptions = Arc::new(Mutex::new(HashMap::new()));
    let conn_id = CONN_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let mut authenticated = false;
    let shared_cipher: Arc<Mutex<Option<SessionCipher>>> = Arc::new(Mutex::new(None));

    // Step 1: Send server_nonce
    let mut server_nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut server_nonce);
    {
        let msg = serde_json::json!({ "server_nonce": bytes_to_hex(&server_nonce) });
        let mut tx = sender.lock().await;
        if tx.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await.is_err() {
            return;
        }
    }

    // Start subscription polling task
    let sub_handle = tokio::spawn(subscription_loop(sender.clone(), subs.clone(), shared_cipher.clone()));

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                eprintln!("❌ Read error from {}: {}", addr, e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                // Decrypt if encrypted session is active
                let plaintext = {
                    let mut c = shared_cipher.lock().await;
                    if let Some(ref mut sc) = *c {
                        match sc.decrypt(&{
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).unwrap_or_default()
                        }) {
                            Ok(pt) => String::from_utf8_lossy(&pt).to_string(),
                            Err(_) => { eprintln!("❌ Decrypt failed from {}", addr); break; }
                        }
                    } else {
                        text.to_string()
                    }
                };

                let response = match serde_json::from_str::<Request>(&plaintext) {
                    Ok(req) => {
                        if !authenticated {
                            if req.method == "auth" {
                                // Encrypted auth: client sends {client_nonce, proof}
                                let client_nonce_hex = req.params.get("client_nonce").and_then(|v| v.as_str()).unwrap_or("");
                                let proof_hex = req.params.get("proof").and_then(|v| v.as_str()).unwrap_or("");
                                // Also support legacy plain token auth
                                let plain_token = req.params.get("token").and_then(|v| v.as_str()).unwrap_or("");

                                if !client_nonce_hex.is_empty() && !proof_hex.is_empty() {
                                    // Encrypted auth flow
                                    let client_nonce_bytes = hex_to_bytes(client_nonce_hex).unwrap_or_default();
                                    let proof_bytes = hex_to_bytes(proof_hex).unwrap_or_default();
                                    if client_nonce_bytes.len() != 16 {
                                        let r = Response::err(req.id, ERR_AUTH, "invalid client_nonce".into());
                                        let json = serde_json::to_string(&r).unwrap();
                                        let mut tx = sender.lock().await;
                                        let _ = tx.send(Message::Text(json.into())).await;
                                        let _ = tx.send(Message::Close(None)).await;
                                        break;
                                    }
                                    let mut cn = [0u8; 16];
                                    cn.copy_from_slice(&client_nonce_bytes);
                                    let key = derive_key(&token, &server_nonce, &cn);
                                    // Verify proof = HMAC-SHA256(key, server_nonce)
                                    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).unwrap();
                                    mac.update(&server_nonce);
                                    if mac.verify_slice(&proof_bytes).is_ok() {
                                        authenticated = true;
                                        auth_tracker.lock().await.remove(&addr.ip());
                                        let mut sc = SessionCipher::new(&key);
                                        let resp = serde_json::to_string(&serde_json::json!({"result":{"authenticated":true,"machine_id":*machine_id,"hostname":&hostname}})).unwrap();
                                        let ct = sc.encrypt(resp.as_bytes());
                                        use base64::Engine;
                                        let b64 = base64::engine::general_purpose::STANDARD.encode(&ct);
                                        let mut tx = sender.lock().await;
                                        let _ = tx.send(Message::Text(b64.into())).await;
                                        drop(tx);
                                        *shared_cipher.lock().await = Some(sc);
                                        continue;
                                    } else {
                                        let mut tracker = auth_tracker.lock().await;
                                        let entry = tracker.entry(addr.ip()).or_insert((0, tokio::time::Instant::now()));
                                        if entry.1.elapsed().as_secs() >= AUTH_LOCKOUT_SECS { *entry = (0, tokio::time::Instant::now()); }
                                        entry.0 += 1;
                                        eprintln!("🚫 Auth failed from {} (attempt {})", addr, entry.0);
                                        drop(tracker);
                                        let r = Response::err(req.id, ERR_AUTH, "invalid proof".into());
                                        let json = serde_json::to_string(&r).unwrap();
                                        let mut tx = sender.lock().await;
                                        let _ = tx.send(Message::Text(json.into())).await;
                                        let _ = tx.send(Message::Close(None)).await;
                                        break;
                                    }
                                } else if provided_token_matches(plain_token, &token) {
                                    // Legacy plain token auth (for wss:// or local connections)
                                    authenticated = true;
                                    auth_tracker.lock().await.remove(&addr.ip());
                                    Response::ok(req.id, serde_json::json!({ "authenticated": true, "machine_id": *machine_id, "hostname": &hostname }))
                                } else {
                                    // Track failure
                                    let mut tracker = auth_tracker.lock().await;
                                    let entry = tracker.entry(addr.ip()).or_insert((0, tokio::time::Instant::now()));
                                    if entry.1.elapsed().as_secs() >= AUTH_LOCKOUT_SECS {
                                        *entry = (0, tokio::time::Instant::now());
                                    }
                                    entry.0 += 1;
                                    let fails = entry.0;
                                    drop(tracker);
                                    eprintln!("🚫 Auth failed from {} (attempt {})", addr, fails);
                                    let r = Response::err(req.id, ERR_AUTH, "invalid token".into());
                                    let json = serde_json::to_string(&r).unwrap();
                                    let mut tx = sender.lock().await;
                                    let _ = tx.send(Message::Text(json.into())).await;
                                    let _ = tx.send(Message::Close(None)).await;
                                    break;
                                }
                            } else {
                                let r = Response::err(req.id, ERR_AUTH, "auth required — send {\"method\":\"auth\",\"params\":{\"token\":\"...\"}} first".into());
                                let json = serde_json::to_string(&r).unwrap();
                                let mut tx = sender.lock().await;
                                let _ = tx.send(Message::Text(json.into())).await;
                                let _ = tx.send(Message::Close(None)).await;
                                break;
                            }
                        } else {
                            match req.method.as_str() {
                                "subscribe" => {
                                    let mut map = subs.lock().await;
                                    handle_subscribe(&req.params, &mut map)
                                }
                                "unsubscribe" => {
                                    let mut map = subs.lock().await;
                                    handle_unsubscribe(&req.params, &mut map)
                                }
                                "resize_pane" => {
                                    let id = req.id;
                                    let p = &req.params;
                                    let target = p.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as usize;
                                    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as usize;
                                    let tracker = resize_tracker.clone();
                                    tokio::task::spawn_blocking(move || {
                                        match tmux::resize_pane(&target, cols, rows) {
                                            Ok(()) => {
                                                let win = target.split('.').next().unwrap_or(&target).to_string();
                                                let session = target.split(':').next().unwrap_or(&target);
                                                // Set tmux hook so next real client auto-restores size
                                                let _ = tmux::set_resize_hook(session);
                                                tracker.lock().unwrap().entry(conn_id).or_default().insert(win);
                                                Response::ok(id, serde_json::json!({ "ok": true }))
                                            }
                                            Err(e) => Response::err(id, ERR_INTERNAL, e),
                                        }
                                    }).await.unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e)))
                                }
                                _ => tokio::task::spawn_blocking(move || handle_request(&req))
                                    .await
                                    .unwrap_or_else(|e| {
                                        Response::err(
                                            None,
                                            ERR_INTERNAL,
                                            format!("task panic: {}", e),
                                        )
                                    }),
                            }
                        }
                    }
                    Err(e) => Response::err(None, ERR_PARSE, format!("invalid JSON: {}", e)),
                };

                let json = serde_json::to_string(&response).unwrap();
                let out = {
                    let mut c = shared_cipher.lock().await;
                    if let Some(ref mut sc) = *c {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD.encode(sc.encrypt(json.as_bytes()))
                    } else {
                        json
                    }
                };
                let mut tx = sender.lock().await;
                if tx.send(Message::Text(out.into())).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            Message::Ping(data) => {
                let mut tx = sender.lock().await;
                let _ = tx.send(Message::Pong(data)).await;
            }
            _ => {}
        }
    }

    sub_handle.abort();
    // Restore any windows this connection resized (tmux auto-fits to remaining clients)
    {
        let mut tracker = resize_tracker.lock().unwrap();
        if let Some(windows) = tracker.remove(&conn_id) {
            for win in &windows {
                let _ = tmux::run_resize_window_auto(win);
                eprintln!("📐 Restored window '{}' to auto-size", win);
            }
        }
    }
    println!("👋 Client disconnected: {} (conn_id={})", addr, conn_id);
}

pub async fn start(host: &str, port: u16, token: &str) -> Result<(), Box<dyn std::error::Error>> {
    start_with_socket(host, port, token, "unknown", None, None, None).await
}

pub async fn start_with_socket(
    host: &str,
    port: u16,
    token: &str,
    machine_id: &str,
    socket: Option<String>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    tmux::set_socket(socket);
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    let token = Arc::new(token.to_string());
    let machine_id = Arc::new(machine_id.to_string());
    let auth_tracker: AuthTracker = Arc::new(Mutex::new(HashMap::new()));
    let resize_tracker: ResizeTracker = Arc::new(std::sync::Mutex::new(HashMap::new()));

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
        let token = token.clone();
        let machine_id = machine_id.clone();
        let auth_tracker = auth_tracker.clone();
        let control_mgr = resize_tracker.clone();
        if let Some(ref acceptor) = tls_acceptor {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let ws_stream = match tokio_tungstenite::accept_async(tls_stream).await {
                            Ok(ws) => ws,
                            Err(e) => { eprintln!("❌ WSS handshake failed for {}: {}", addr, e); return; }
                        };
                        handle_connection_ws(ws_stream, addr, token, machine_id, auth_tracker, control_mgr).await;
                    }
                    Err(e) => eprintln!("❌ TLS handshake failed for {}: {}", addr, e),
                }
            });
        } else {
            tokio::spawn(handle_connection(stream, addr, token, machine_id, auth_tracker, control_mgr));
        }
    }
}
