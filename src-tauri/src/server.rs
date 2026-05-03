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

// Brute-force protection: track failed auth attempts per IP
pub type AuthTracker = Arc<Mutex<HashMap<IpAddr, (u32, tokio::time::Instant)>>>;

// Track which windows each connection has resized, so we can restore on disconnect.
// Key = conn_id, Value = set of "session:window" targets that were resized.
pub type ResizeTracker = Arc<std::sync::Mutex<HashMap<u64, std::collections::HashSet<String>>>>;

static CONN_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

const MAX_AUTH_FAILURES: u32 = 5;
const AUTH_LOCKOUT_SECS: u64 = 60;
const SUBSCRIPTION_POLL_MS: u64 = 200;
const MAX_CAPTURE_FAILURES: u32 = 5;

// WebSocket frame / message limits. A legitimate `fs_upload` can carry a
// file up to fs::MAX_READ_SIZE (50 MB) inside a base64 string (~67 MB text),
// plus JSON envelope + encryption overhead. 80 MB accommodates that with
// margin; anything bigger is almost certainly malformed or abusive.
// tokio-tungstenite's default (64 MB) is too small for a max-size upload,
// and without an explicit cap an attacker could force per-connection buffer
// growth up to that limit on every frame.
const WS_MAX_MESSAGE_BYTES: usize = 80 * 1024 * 1024;
const WS_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

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
    /// Hand a freshly-built send-side cipher to the send task. Emitted once,
    /// right after successful encrypted auth. Must be enqueued *before* any
    /// `Encrypted` message for that session, otherwise the send task drops
    /// the message with a warning.
    InitCipher(HalfCipher),
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
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(SUBSCRIPTION_POLL_MS));
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
                // Get cursor first for pane width, then capture content immediately after
                // to minimize race window between the two tmux calls
                let cursor = tmux::cursor_info(&t2).unwrap_or((0, 0, 24, 80));
                let (content, trailing) = tmux::capture_pane_with_width(&t, None, cursor.3)?;
                // Re-read cursor to get position matching the captured content
                let cursor = tmux::cursor_info(&t2).unwrap_or(cursor);
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
            if out_tx.send(Outbound::Encrypted(serde_json::to_string(&msg).unwrap())).is_err() {
                return; // receiver has shut down
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

pub async fn handle_connection(stream: TcpStream, addr: SocketAddr, token: Arc<String>, machine_id: Arc<String>, auth_tracker: AuthTracker, resize_tracker: ResizeTracker) {
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

    let ws_stream = match accept_async_with_config(stream, Some(ws_config())).await {
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
    let subs: Subscriptions = Arc::new(Mutex::new(HashMap::new()));
    let conn_id = CONN_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let mut authenticated = false;
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
    let addr_for_send = addr;
    let send_task = tokio::spawn(async move {
        let mut ws_sender = ws_sender;
        let mut cipher: Option<HalfCipher> = None;
        while let Some(msg) = out_rx.recv().await {
            let bytes: String = match msg {
                Outbound::Plain(s) => s,
                Outbound::InitCipher(c) => { cipher = Some(c); continue; }
                Outbound::Encrypted(s) => {
                    if let Some(ref mut c) = cipher {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD.encode(c.encrypt(s.as_bytes()))
                    } else {
                        eprintln!("send [{}]: Encrypted before InitCipher — dropped", addr_for_send);
                        continue;
                    }
                }
            };
            if ws_sender.send(Message::Text(bytes.into())).await.is_err() {
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
    let sub_handle = tokio::spawn(subscription_loop(out_tx.clone(), subs.clone()));

    while let Some(msg) = tokio::select! {
        m = receiver.next() => m,
        _ = shutdown.notified() => None,  // send task died — tear down
    } {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                eprintln!("❌ Read error from {}: {}", addr, e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                // Decrypt in strict order (recv_cipher counter is monotonic).
                let plaintext = if let Some(ref mut rc) = recv_cipher {
                    use base64::Engine;
                    let ct = base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).unwrap_or_default();
                    match rc.decrypt(&ct) {
                        Ok(pt) => String::from_utf8_lossy(&pt).to_string(),
                        Err(_) => { eprintln!("❌ Decrypt failed from {}", addr); break; }
                    }
                } else {
                    text.to_string()
                };

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
                                        tracker.lock().unwrap().entry(conn_id).or_default().insert(win);
                                        Response::ok(id, serde_json::json!({ "ok": true }))
                                    }
                                    Err(e) => Response::err(id, ERR_INTERNAL, e),
                                }
                            }).await.unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e)))
                        }
                        _ => tokio::task::spawn_blocking(move || handle_request(&req))
                            .await
                            .unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e))),
                    };
                    let _ = out_tx_c.send(Outbound::Encrypted(serde_json::to_string(&response).unwrap()));
                });
            }
            Message::Close(_) => break,
            Message::Ping(data) => {
                // OS-level WS ping: respond with Pong inline (cheap, no cipher).
                // Note: this still races on ws_sender which the send task owns.
                // To keep things simple we let the client rely on application
                // pings (ping RPC) which flow through out_tx; browser WS
                // doesn't normally initiate protocol-level pings anyway.
                let _ = out_tx.send(Outbound::Plain(String::from_utf8_lossy(&data).into_owned()));
            }
            _ => {}
        }
    }

    sub_handle.abort();
    drop(out_tx); // close the channel so the send task finishes
    let _ = send_task.await;
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
    // Best-effort harden existing config.toml so upgraded installs with the
    // old loose permissions get fixed on next start.
    crate::config::harden_config_perms();
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
                        let ws_stream = match tokio_tungstenite::accept_async_with_config(tls_stream, Some(ws_config())).await {
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
