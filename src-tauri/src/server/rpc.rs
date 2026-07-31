//! The JSON-RPC request/response shapes and the main method dispatch
//! (everything a single request can do that doesn't need the connection's
//! push machinery), plus subscribe/unsubscribe bookkeeping.
//! Split from server.rs 2026-07-22 — content unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::fs as rfs;
use crate::tmux;

use super::download::sign_download;

// JSON-RPC style request/response

#[derive(Deserialize, Debug)]
pub(super) struct Request {
    pub(super) id: Option<u64>,
    pub(super) method: String,
    #[serde(default)]
    pub(super) params: serde_json::Value,
}

#[derive(Serialize, Clone)]
pub(super) struct Response {
    pub(super) id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<ErrorInfo>,
}

#[derive(Serialize, Clone)]
pub(super) struct ErrorInfo {
    pub(super) code: i32,
    pub(super) message: String,
}

// Error codes
pub(super) const ERR_PARSE: i32 = -32700;
pub(super) const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub(super) const ERR_INVALID_PARAMS: i32 = -32602;
pub(super) const ERR_INTERNAL: i32 = -32603;
pub(super) const ERR_AUTH: i32 = -32000;

impl Response {
    pub(super) fn ok(id: Option<u64>, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }
    pub(super) fn err(id: Option<u64>, code: i32, message: String) -> Self {
        Self {
            id,
            result: None,
            error: Some(ErrorInfo { code, message }),
        }
    }
}

// Per-connection subscription state: target -> last captured content
pub(super) type Subscriptions = Arc<Mutex<HashMap<String, String>>>;

pub(super) fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing required param: {}", key))
}

pub(super) fn valid_process_arg(arg: &str) -> bool {
    !arg.contains('\0')
}

pub(super) fn handle_request(req: &Request, token: &str) -> Response {
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

        "paste_text" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let text = match require_str(p, "text") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match tmux::paste_text(target, text) {
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
            // Arguments go directly to Command::args; no shell parses them.
            // Characters such as `|` are data (git log format separators),
            // not operators. Only NUL is impossible to represent in argv.
            for arg in &args {
                if !valid_process_arg(arg) {
                    return Response::err(id, ERR_INVALID_PARAMS, "invalid characters in argument".into());
                }
            }

            let mut child = std::process::Command::new("git");
            child.arg(subcmd);
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

        // ---- projects (declarative workspaces) ----------------------------
        // Desktop-only: state.db and the reconciler are not compiled for
        // Android/iOS, where these methods report method-not-found and the
        // client hides the page (same contract as the team_* methods).
        "project_list" | "project_create" | "project_adopt" | "project_up" | "project_down"
        | "project_archive" | "project_autostart" | "project_snapshots" | "project_restore" => {
            handle_project_request(req.method.as_str(), id, p)
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
            match ext.as_str() {
                "pptx" => match crate::pptx::to_html(std::path::Path::new(path)) {
                    Ok(html) => Response::ok(id, serde_json::json!({ "html": html })),
                    Err(e) => Response::err(id, ERR_INTERNAL, e),
                },
                _ => Response::err(id, ERR_INVALID_PARAMS, format!("unsupported file type: .{}", ext)),
            }
        }

        _ => Response::err(
            id,
            ERR_METHOD_NOT_FOUND,
            format!("unknown method: {}", req.method),
        ),
    }
}

/// The `project_*` methods. One function so the platform gate lives in exactly
/// one place: on mobile every method reports method-not-found and the client
/// hides the Projects page.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn handle_project_request(method: &str, id: Option<u64>, p: &serde_json::Value) -> Response {
    use crate::projects;

    let need_id = |key: &str| -> Result<String, String> { require_str(p, key).map(str::to_string) };
    let flag = |key: &str, default: bool| p.get(key).and_then(|v| v.as_bool()).unwrap_or(default);

    let outcome = match method {
        "project_list" => projects::list(flag("include_archived", false)),
        "project_create" => need_id("path").and_then(|path| {
            projects::create(
                &path,
                p.get("name").and_then(|v| v.as_str()),
                p.get("session").and_then(|v| v.as_str()),
                p.get("agent").and_then(|v| v.as_str()),
            )
        }),
        "project_adopt" => need_id("session").and_then(|session| {
            projects::adopt(&session, p.get("name").and_then(|v| v.as_str()))
        }),
        "project_up" => need_id("id").and_then(|id| projects::up(&id)),
        "project_down" => need_id("id").and_then(|id| projects::down(&id)),
        "project_archive" => {
            need_id("id").and_then(|id| projects::set_archived(&id, flag("archived", true)))
        }
        "project_autostart" => {
            need_id("id").and_then(|id| projects::set_autostart(&id, flag("autostart", true)))
        }
        "project_snapshots" => need_id("id").and_then(|id| projects::snapshots(&id)),
        "project_restore" => need_id("id").and_then(|id| {
            match p.get("snapshot_id").and_then(|v| v.as_i64()) {
                Some(snapshot_id) => projects::restore(&id, snapshot_id),
                None => Err("missing required param: snapshot_id".into()),
            }
        }),
        other => Err(format!("unknown project method: {other}")),
    };

    match outcome {
        Ok(value) => Response::ok(id, value),
        // A missing/blank param is the client's fault, everything else is ours.
        Err(e) if e.starts_with("missing required param") => {
            Response::err(id, ERR_INVALID_PARAMS, e)
        }
        Err(e) => Response::err(id, ERR_INTERNAL, e),
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn handle_project_request(method: &str, id: Option<u64>, _p: &serde_json::Value) -> Response {
    Response::err(
        id,
        ERR_METHOD_NOT_FOUND,
        format!("{method} is unavailable on this platform"),
    )
}

// Subscription polling task: captures pane content and sends diffs
pub(super) fn handle_subscribe(params: &serde_json::Value, subs: &mut HashMap<String, String>) -> Response {
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

pub(super) fn handle_unsubscribe(params: &serde_json::Value, subs: &mut HashMap<String, String>) -> Response {
    let target = match require_str(params, "target") {
        Ok(s) => s,
        Err(e) => return Response::err(None, ERR_INVALID_PARAMS, e),
    };
    subs.remove(target);
    Response::ok(None, serde_json::json!({ "unsubscribed": target }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_arguments_allow_literal_log_separators() {
        assert!(valid_process_arg("--format=%h|%s|%ar|%an"));
        assert!(valid_process_arg("subject; $HOME & <literal>"));
        assert!(!valid_process_arg("bad\0argument"));
    }

    fn convert(path: &str) -> Response {
        let req = Request {
            id: Some(1),
            method: "fs_convert".into(),
            params: serde_json::json!({ "path": path }),
        };
        handle_request(&req, "")
    }

    #[test]
    fn fs_convert_reports_errors_without_shelling_out() {
        // Conversion runs in-process, so failures are short messages — not a
        // Python traceback leaking into the preview pane.
        let err = convert("/nonexistent/deck.pptx").error.expect("error");
        assert_eq!(err.code, ERR_INTERNAL);
        assert!(err.message.starts_with("open:"), "{}", err.message);

        let err = convert("/tmp/notes.txt").error.expect("error");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
        assert!(err.message.contains("unsupported file type: .txt"), "{}", err.message);
    }
}

