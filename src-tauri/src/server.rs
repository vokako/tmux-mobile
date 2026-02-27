use crate::tmux;
use crate::fs as rfs;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::{accept_async, tungstenite::Message};

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
        Self { id, result: Some(result), error: None }
    }
    fn err(id: Option<u64>, code: i32, message: String) -> Self {
        Self { id, result: None, error: Some(ErrorInfo { code, message }) }
    }
}

// Per-connection subscription state: target -> last captured content
type Subscriptions = Arc<Mutex<HashMap<String, String>>>;

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    params.get(key)
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

        "new_session" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("untitled");
            match tmux::new_session(name) {
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
            let show_hidden = p.get("show_hidden").and_then(|v| v.as_bool()).unwrap_or(false);
            match rfs::list_dir(path, show_hidden) {
                Ok(entries) => Response::ok(id, serde_json::json!({ "entries": entries, "path": path })),
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
                Ok((name, data)) => Response::ok(id, serde_json::json!({ "name": name, "data": data })),
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

        _ => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown method: {}", req.method)),
    }
}

// Subscription polling task: captures pane content and sends diffs
async fn subscription_loop(
    sender: Arc<Mutex<futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>, Message
    >>>,
    subs: Subscriptions,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
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
            let new_content = match tokio::task::spawn_blocking(move || {
                tmux::capture_pane(&t, None)
            }).await {
                Ok(Ok(c)) => c,
                _ => continue,
            };
            if new_content == prev {
                continue;
            }
            // Update stored content
            subs.lock().await.insert(target.clone(), new_content.clone());
            // Push update to client
            let msg = serde_json::json!({
                "id": null,
                "method": "pane_output",
                "params": { "target": target, "content": new_content }
            });
            let text = serde_json::to_string(&msg).unwrap();
            let mut tx = sender.lock().await;
            if tx.send(Message::Text(text.into())).await.is_err() {
                return; // connection closed
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

async fn handle_connection(stream: TcpStream, addr: SocketAddr, token: Arc<String>) {
    println!("📱 Client connected: {}", addr);

    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("❌ WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    let (ws_sender, mut receiver) = ws_stream.split();
    let sender = Arc::new(Mutex::new(ws_sender));
    let subs: Subscriptions = Arc::new(Mutex::new(HashMap::new()));
    let mut authenticated = false;

    // Start subscription polling task
    let sub_handle = tokio::spawn(subscription_loop(sender.clone(), subs.clone()));

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
                let response = match serde_json::from_str::<Request>(&text) {
                    Ok(req) => {
                        // Auth gate: first message must be "auth"
                        if !authenticated {
                            if req.method == "auth" {
                                let provided = req.params.get("token")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                if provided == token.as_str() {
                                    authenticated = true;
                                    Response::ok(req.id, serde_json::json!({ "authenticated": true }))
                                } else {
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
                                _ => {
                                    tokio::task::spawn_blocking(move || handle_request(&req))
                                        .await
                                        .unwrap_or_else(|e| Response::err(None, ERR_INTERNAL, format!("task panic: {}", e)))
                                }
                            }
                        }
                    }
                    Err(e) => Response::err(None, ERR_PARSE, format!("invalid JSON: {}", e)),
                };

                let json = serde_json::to_string(&response).unwrap();
                let mut tx = sender.lock().await;
                if tx.send(Message::Text(json.into())).await.is_err() {
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
    println!("👋 Client disconnected: {}", addr);
}

pub async fn start(host: &str, port: u16, token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    let token = Arc::new(token.to_string());

    println!("🚀 tmux-mobile server listening on ws://{}", addr);
    println!("🔑 Token: {}", token);
    println!("   Methods: auth, list_sessions, list_panes, capture_pane, send_keys, send_command, new_session, kill_session, subscribe, unsubscribe");

    loop {
        let (stream, addr) = listener.accept().await?;
        let token = token.clone();
        tokio::spawn(handle_connection(stream, addr, token));
    }
}
