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
    /// The one place an `RpcError` becomes wire bytes (board #146): the code
    /// is the variant's, the message passes through untouched.
    pub(super) fn from_error(id: Option<u64>, e: RpcError) -> Self {
        match e {
            RpcError::InvalidParams(m) => Self::err(id, ERR_INVALID_PARAMS, m),
            RpcError::Internal(m) => Self::err(id, ERR_INTERNAL, m),
            RpcError::MethodNotFound(m) => Self::err(id, ERR_METHOD_NOT_FOUND, m),
        }
    }
    /// Fold a dispatcher's `Result` into a response.
    pub(super) fn from_outcome(id: Option<u64>, r: Result<serde_json::Value, RpcError>) -> Self {
        match r {
            Ok(v) => Self::ok(id, v),
            Err(e) => Self::from_error(id, e),
        }
    }
}

/// A dispatcher error that already knows its wire code (board #146). The
/// dispatchers used to spell `match x { Ok(v) => v, Err(e) => return
/// Response::err(id, CODE, e) }` at every site — ~40 times — with the code
/// chosen per site; this carries that choice so the site can say `?`. It is
/// an enum and not a string prefix on purpose: `handle_project_request`
/// classifies by sniffing `starts_with("missing required param")`, and that
/// rule would file the hub's `"id required"` under INTERNAL when every such
/// site says INVALID_PARAMS. The codes are contracts (`tmm` exit classes,
/// the client's old-server detection), so they stay explicit per site.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RpcError {
    InvalidParams(String),
    Internal(String),
    MethodNotFound(String),
}

/// `require_str` as a `?`-able INVALID_PARAMS — the missing-param message is
/// byte-identical to what every `match require_str` site returned.
pub(super) fn param<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, RpcError> {
    require_str(params, key).map_err(RpcError::InvalidParams)
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

/// The dispatcher proper (board #146): every arm is a `Result`, so a missing
/// param or a tmux/fs error is a `?` with its wire code already chosen
/// (`RpcError`), and `handle_request` below is the one place a `Response`
/// is built. Arm order and every message are exactly what the inline
/// `match … return Response::err` shape produced.
fn dispatch(req: &Request, token: &str) -> Result<Response, RpcError> {
    let id = req.id;
    let p = &req.params;

    match req.method.as_str() {
        "ping" => Ok(Response::ok(id, serde_json::json!("pong"))),

        // The backends this server can spawn, with the client's resource
        // names for each (board #130). No params, no session, no gate: the
        // list lives in the ungated `backends` leaf, so a phone-hosted
        // server answers too. The client drops its hand-kept mirrors for
        // this and falls back to them only when an OLDER server says
        // method-not-found.
        "backends_list" => {
            Ok(Response::ok(id, serde_json::json!({ "backends": crate::backends::Backend::list_json() })))
        }

        // ---- server system vitals (board #56) ------------------------------
        // Desktop-only like the project_* methods: the sampler is not
        // compiled for Android/iOS (a phone is a client of a desktop server),
        // where this reports method-not-found and the client shows nothing.
        // No params, fail-soft by construction: a reading is always returned,
        // with unknowable fields as null/0 for the client's verdict rule.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        "system_status" => {
            Ok(Response::ok(id, serde_json::to_value(crate::system_status::read()).unwrap()))
        }

        "list_sessions" => match tmux::list_sessions() {
            Ok(sessions) => Ok(Response::ok(id, serde_json::to_value(&sessions).unwrap())),
            Err(e) => Err(RpcError::Internal(e)),
        },

        "list_panes" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::list_panes(session) {
                Ok(panes) => Ok(Response::ok(id, serde_json::to_value(&panes).unwrap())),
                Err(e) => Err(RpcError::Internal(e)),
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
                Err(e) => return Err(RpcError::Internal(e)),
            };
            let panes = match tmux::list_all_panes() {
                Ok(v) => v,
                Err(e) => return Err(RpcError::Internal(e)),
            };
            Ok(Response::ok(id, serde_json::json!({
                "sessions": sessions,
                "panes": panes,
            })))
        }

        "capture_pane" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let lines = p.get("lines").and_then(|v| v.as_u64()).map(|n| n as usize);
            match tmux::capture_pane(target, lines) {
                Ok(output) => Ok(Response::ok(id, serde_json::json!({ "output": output }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "send_keys" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let keys = match require_str(p, "keys") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let literal = p.get("literal").and_then(|v| v.as_bool()).unwrap_or(false);
            match tmux::send_keys(target, keys, literal) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "paste_text" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let text = match require_str(p, "text") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::paste_text(target, text) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "send_command" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let command = match require_str(p, "command") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::send_command(target, command) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        // resize_pane is handled in the connection message loop (needs per-connection state)
        "resize_pane" => Err(RpcError::Internal("resize_pane handled elsewhere".into())),

        "new_session" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("untitled");
            let path = p.get("path").and_then(|v| v.as_str());
            let command = p.get("command").and_then(|v| v.as_str());
            match tmux::new_session(name, path, command) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "kill_session" => {
            let name = match require_str(p, "name") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::kill_session(name) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "new_window" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::new_window(session) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "kill_window" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::kill_window(target) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "pane_command" => {
            let target = match require_str(p, "target") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match tmux::pane_command(target) {
                Ok(cmd) => Ok(Response::ok(id, serde_json::json!({ "command": cmd }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "set_socket" => {
            let socket = p
                .get("socket")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tmux::set_socket(socket);
            Ok(Response::ok(id, serde_json::json!({ "ok": true })))
        }

        "get_bookmarks" => {
            let bookmarks = crate::config::get_bookmarks();
            Ok(Response::ok(id, serde_json::json!({ "bookmarks": bookmarks })))
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
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "get_prefs" => {
            Ok(Response::ok(id, crate::config::get_prefs()))
        }

        "set_pref" => {
            let key = match require_str(p, "key") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let value = p.get("value").cloned().unwrap_or(serde_json::Value::Null);
            match crate::config::set_prefs(key, value) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_cwd" => {
            let session = match require_str(p, "session") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::get_cwd(session) {
                Ok(path) => Ok(Response::ok(id, serde_json::json!({ "path": path }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_list" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let show_hidden = p
                .get("show_hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match rfs::list_dir(path, show_hidden) {
                Ok(entries) => {
                    Ok(Response::ok(id, serde_json::json!({ "entries": entries, "path": path })))
                }
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_stat" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::stat_file(path) {
                Ok(stat) => Ok(Response::ok(id, serde_json::to_value(&stat).unwrap())),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_read" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::read_file(path) {
                Ok(content) => Ok(Response::ok(id, serde_json::json!({ "content": content }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_write" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            // Allow empty content (creating empty files is valid)
            let content = p.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match rfs::write_file(path, content) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_mkdir" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::create_dir(path) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_delete" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::delete_path(path) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_rename" => {
            let from = match require_str(p, "from") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let to = match require_str(p, "to") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::rename_path(from, to) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_download" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::download_file(path) {
                Ok((name, data)) => {
                    Ok(Response::ok(id, serde_json::json!({ "name": name, "data": data })))
                }
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "fs_download_url" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let sig = sign_download(token, path, ts);
            let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let qs = format!("/dl?path={}&ts={}&sig={}", urlencoding::encode(path), ts, sig);
            Ok(Response::ok(id, serde_json::json!({ "url": qs, "name": name })))
        }

        "fs_upload" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let data = match require_str(p, "data") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            match rfs::upload_file(path, data) {
                Ok(()) => Ok(Response::ok(id, serde_json::json!({ "ok": true }))),
                Err(e) => Err(RpcError::Internal(e)),
            }
        }

        "git" => {
            let subcmd = match require_str(p, "subcmd") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
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
                return Err(RpcError::InvalidParams(format!("git subcommand not allowed: {}", subcmd)));
            }
            // Arguments go directly to Command::args; no shell parses them.
            // Characters such as `|` are data (git log format separators),
            // not operators. Only NUL is impossible to represent in argv.
            for arg in &args {
                if !valid_process_arg(arg) {
                    return Err(RpcError::InvalidParams("invalid characters in argument".into()));
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
                    Ok(Response::ok(
                        id,
                        serde_json::json!({ "stdout": stdout, "stderr": stderr, "code": output.status.code() }),
                    ))
                }
                Err(e) => Err(RpcError::Internal(e.to_string())),
            }
        }

        // ---- projects (declarative workspaces) ----------------------------
        // Desktop-only: state.db and the reconciler are not compiled for
        // Android/iOS, where these methods report method-not-found and the
        // client hides the page (same contract as the team_* methods).
        "project_list" | "project_create" | "project_adopt" | "project_up" | "project_down"
        | "project_archive" | "project_delete" | "project_autostart" | "project_rename"
        | "registry_list" | "registry_save" | "registry_delete" | "models_list"
        | "global_prompt_get" | "global_prompt_set"
        | "teams_list" | "teams_save" | "teams_delete"
        | "skills_list" | "skills_save" | "skills_delete" | "skills_refresh" | "skills_read"
        | "skills_import" | "skills_files" | "skills_file"
        | "mcp_list" | "mcp_save" | "mcp_delete" => {
            Ok(handle_project_request(req.method.as_str(), id, p))
        }

        "fs_convert" => {
            let path = match require_str(p, "path") {
                Ok(s) => s,
                Err(e) => return Err(RpcError::InvalidParams(e)),
            };
            let format = p.get("format").and_then(|v| v.as_str()).unwrap_or("html");
            if format != "html" {
                return Err(RpcError::InvalidParams("only html format supported".into()));
            }
            let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            match ext.as_str() {
                "pptx" => match crate::pptx::to_html(std::path::Path::new(path)) {
                    Ok(html) => Ok(Response::ok(id, serde_json::json!({ "html": html }))),
                    Err(e) => Err(RpcError::Internal(e)),
                },
                _ => Err(RpcError::InvalidParams(format!("unsupported file type: .{}", ext))),
            }
        }

        _ => Err(RpcError::MethodNotFound(format!("unknown method: {}", req.method))),
    }
}

pub(super) fn handle_request(req: &Request, token: &str) -> Response {
    match dispatch(req, token) {
        Ok(r) => r,
        Err(e) => Response::from_error(req.id, e),
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
        // The name is a label, so this touches nothing else: the session stays
        // the project's identity and the chat room keeps its key.
        "project_rename" => need_id("id")
            .and_then(|id| need_id("name").and_then(|name| projects::rename(&id, &name))),
        // Archive hides and is reversible; delete forgets the project and wipes
        // the isolated homes of the agents it owned.
        "project_delete" => need_id("id").and_then(|id| projects::delete(&id)),
        "project_archive" => {
            need_id("id").and_then(|id| projects::set_archived(&id, flag("archived", true)))
        }
        "project_autostart" => {
            need_id("id").and_then(|id| projects::set_autostart(&id, flag("autostart", true)))
        }
        // Agent registry (agents-v2): centrally-defined agents, spawnable into
        // any project. See docs/design-docs/features/tmm-cli.md.
        "registry_list" => projects::registry_list(),
        "registry_save" => match p.get("def") {
            Some(def) => projects::registry_save(def),
            None => Err("missing required param: def".into()),
        },
        "registry_delete" => need_id("name").and_then(|n| projects::registry_delete(&n)),
        // The app-wide agent instructions (`<config>/AGENTS.md`): one file,
        // prepended to every managed agent's prompt at spawn.
        "global_prompt_get" => projects::global_prompt_get(),
        "global_prompt_set" => match p.get("text").and_then(|v| v.as_str()) {
            Some(text) => projects::global_prompt_set(text),
            None => Err("missing required param: text".into()),
        },
        // Agent teams (board #74): a named list of members derived from
        // registry agents (+ role) or defined inline for this team only.
        "teams_list" => projects::teams_list(),
        "teams_save" => match p.get("def") {
            Some(def) => projects::teams_save(def),
            None => Err("missing required param: def".into()),
        },
        "teams_delete" => need_id("name").and_then(|n| projects::teams_delete(&n)),
        // The model ids a backend accepts, so the agent editor can offer them
        // instead of letting a one-character typo through. `null` means the
        // backend cannot enumerate them (claude/codex) — the field stays free
        // text there.
        "models_list" => {
            // Absent backend = the documented default (board #127); the literal
            // lived inline here and was invisible to the backend map.
            let backend = p.get("backend").and_then(|v| v.as_str()).unwrap_or(crate::backends::Backend::DEFAULT.name());
            Ok(serde_json::json!({
                "backend": backend,
                "models": projects::models::list(backend),
            }))
        }
        // Central skills / MCP assets (owner: "集中化管理") — referenced from
        // agent defs by name, resolved at spawn.
        "skills_list" => projects::skills_list(),
        "skills_save" => match p.get("def") {
            Some(def) => projects::skill_save(def),
            None => Err("missing required param: def".into()),
        },
        "skills_delete" => need_id("name").and_then(|n| projects::skill_delete(&n)),
        "skills_refresh" => need_id("name").and_then(|n| projects::skill_refresh(&n)),
        "skills_read" => need_id("name").and_then(|n| projects::skill_read(&n)),
        "skills_import" => require_str(p, "source").and_then(|s| projects::skill_import(s)),
        "skills_files" => need_id("name").and_then(|n| projects::skill_files(&n)),
        "skills_file" => need_id("name").and_then(|n| {
            require_str(p, "path").and_then(|f| projects::skill_file(&n, f))
        }),
        "mcp_list" => projects::mcp_list(),
        "mcp_save" => match p.get("def") {
            Some(def) => projects::mcp_save(def),
            None => Err("missing required param: def".into()),
        },
        "mcp_delete" => need_id("name").and_then(|n| projects::mcp_delete(&n)),
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

    /// The typed error reproduces the exact wire shape of the hand-written
    /// sites it replaces (board #146).
    #[test]
    fn rpc_error_carries_its_code_and_passes_the_message_through() {
        let r = Response::from_error(Some(7), RpcError::InvalidParams("missing required param: x".into()));
        let e = r.error.unwrap();
        assert_eq!((r.id, e.code, e.message.as_str()), (Some(7), ERR_INVALID_PARAMS, "missing required param: x"));
        let e = Response::from_error(None, RpcError::Internal("boom".into())).error.unwrap();
        assert_eq!((e.code, e.message.as_str()), (ERR_INTERNAL, "boom"));
        let e = Response::from_error(None, RpcError::MethodNotFound("unknown method: z".into())).error.unwrap();
        assert_eq!((e.code, e.message.as_str()), (ERR_METHOD_NOT_FOUND, "unknown method: z"));
        assert_eq!(param(&serde_json::json!({}), "session"), Err(RpcError::InvalidParams("missing required param: session".into())));
        assert_eq!(param(&serde_json::json!({"session": ""}), "session"), Err(RpcError::InvalidParams("missing required param: session".into())));
        assert_eq!(param(&serde_json::json!({"session": "s"}), "session"), Ok("s"));
        let ok = Response::from_outcome(Some(1), Ok(serde_json::json!({"a": 1})));
        assert!(ok.error.is_none() && ok.result.is_some());
    }


    #[test]
    fn git_arguments_allow_literal_log_separators() {
        assert!(valid_process_arg("--format=%h|%s|%ar|%an"));
        assert!(valid_process_arg("subject; $HOME & <literal>"));
        assert!(!valid_process_arg("bad\0argument"));
    }

    /// Board #56: the router answers `system_status` with the documented
    /// wire shape — no params required, never an error (fail-soft is the
    /// module's contract; the router must not add a failure mode).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn system_status_routes_and_answers_the_wire_shape() {
        let req = Request {
            id: Some(7),
            method: "system_status".into(),
            params: serde_json::json!({}),
        };
        let resp = handle_request(&req, "");
        assert!(resp.error.is_none(), "system_status must be fail-soft");
        let v = resp.result.expect("a reading");
        for k in ["cpu_pct", "mem_used", "mem_total", "disk_used", "disk_total"] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
        assert!(v["mem_total"].as_u64().unwrap_or(0) > 0, "memory reads on a live machine");
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

/// The `agent_notifications_*` unread-inbox RPCs retired 2026-09-01 with the
/// old notification-dot UI (owner: "原来我用的感觉不是很好用") — the project
/// room's auto-post + read cursor and the derived status dots replaced it.
/// Only the hook management surface remains. (Lived in team_rpc.rs until the
/// Team system was deleted whole, board #100.)
pub(super) fn handle_notification_request(req: &Request, hub: &crate::agent_notifications::AgentNotificationHub) -> Response {
    let id = req.id;
    match req.method.as_str() {
        "agent_hooks_status" => Response::ok(id, serde_json::to_value(hub.hook_status()).unwrap()),
        "agent_hooks_install" => match hub.install_hooks() {
            Ok(status) => Response::ok(id, serde_json::to_value(status).unwrap()),
            Err(error) => Response::err(id, ERR_INTERNAL, error),
        },
        "agent_hooks_remove" => match hub.remove_hooks() {
            Ok(status) => Response::ok(id, serde_json::to_value(status).unwrap()),
            Err(error) => Response::err(id, ERR_INTERNAL, error),
        },
        other => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown agent notification method: {other}")),
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    use super::super::test_util::req;
    use crate::agent_notifications::AgentNotificationHub;

    // ─── the retired unread-inbox RPCs stay retired (board #37) ─────────
    /// An old client still calls `agent_notifications_list`/`mark_read`; it
    /// must get a soft METHOD_NOT_FOUND — never a panic, never a resurrected
    /// snapshot — while the hook-management surface on the SAME dispatcher
    /// keeps answering.
    #[test]
    fn retired_notification_rpcs_degrade_soft_and_hooks_survive() {
        let root = std::env::temp_dir().join(format!("tmm-retired-rpc-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at_for_tests(root.clone());
        for method in ["agent_notifications_list", "agent_notifications_mark_read"] {
            let resp = handle_notification_request(
                &req(method, serde_json::json!({ "session": "s", "window": 0 })),
                &hub,
            );
            let err = resp.error.expect("retired method answers with an error");
            assert_eq!(err.code, ERR_METHOD_NOT_FOUND, "{method}");
            assert!(resp.result.is_none(), "{method} must not return a snapshot");
        }
        let resp = handle_notification_request(&req("agent_hooks_status", serde_json::json!({})), &hub);
        assert!(resp.error.is_none() && resp.result.is_some(), "hook status still answers");
        let _ = std::fs::remove_dir_all(root);
    }
}
