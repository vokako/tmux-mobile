/// tmux 操作封装层
/// 通过调用 tmux CLI 来管理 session/window/pane
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct TmuxSession {
    pub name: String,
    pub windows: usize,
    pub attached: bool,
    pub created: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmuxPane {
    pub session: String,
    pub window: usize,
    pub pane: usize,
    pub width: usize,
    pub height: usize,
    pub current_command: String,
}

use std::sync::{OnceLock, RwLock};

static TMUX_SOCKET: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn socket_lock() -> &'static RwLock<Option<String>> {
    TMUX_SOCKET.get_or_init(|| RwLock::new(None))
}

pub fn set_socket(socket: Option<String>) {
    *socket_lock().write().unwrap() = socket;
}

pub fn get_socket() -> Option<String> {
    socket_lock().read().unwrap().clone()
}

/// 执行 tmux 命令，返回 stdout
fn run_tmux(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("tmux");
    if let Some(socket) = get_socket() {
        cmd.args(["-S", &socket]);
    }
    let output = cmd
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run tmux: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("tmux error: {}", stderr))
    }
}

/// 列出所有 session
pub fn list_sessions() -> Result<Vec<TmuxSession>, String> {
    let output = run_tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}|#{session_windows}|#{session_attached}|#{session_activity}",
    ])?;

    let sessions = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            TmuxSession {
                name: parts.get(0).unwrap_or(&"").to_string(),
                windows: parts.get(1).unwrap_or(&"0").parse().unwrap_or(0),
                attached: parts.get(2).unwrap_or(&"0") == &"1",
                created: parts.get(3).unwrap_or(&"").to_string(),
            }
        })
        .collect();

    Ok(sessions)
}

/// 列出某个 session 的所有 pane
pub fn list_panes(session: &str) -> Result<Vec<TmuxPane>, String> {
    let output = run_tmux(&[
        "list-panes",
        "-s",
        "-t",
        session,
        "-F",
        "#{session_name}|#{window_index}|#{pane_index}|#{pane_width}|#{pane_height}|#{pane_current_command}",
    ])?;

    let panes = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            TmuxPane {
                session: parts.get(0).unwrap_or(&"").to_string(),
                window: parts.get(1).unwrap_or(&"0").parse().unwrap_or(0),
                pane: parts.get(2).unwrap_or(&"0").parse().unwrap_or(0),
                width: parts.get(3).unwrap_or(&"0").parse().unwrap_or(0),
                height: parts.get(4).unwrap_or(&"0").parse().unwrap_or(0),
                current_command: parts.get(5).unwrap_or(&"").to_string(),
            }
        })
        .collect();

    Ok(panes)
}

/// Get current command of a pane
pub fn pane_command(target: &str) -> Result<String, String> {
    run_tmux(&["display-message", "-t", target, "-p", "#{pane_current_command}"])
        .map(|s| s.trim().to_string())
}

/// 捕获 pane 内容（屏幕输出，保留 ANSI 转义序列）
pub fn capture_pane(target: &str, lines: Option<usize>) -> Result<String, String> {
    let start_line = lines.map(|n| format!("-{}", n)).unwrap_or("-200".to_string());
    run_tmux(&[
        "capture-pane",
        "-t",
        target,
        "-p",           // 输出到 stdout
        "-e",           // 保留 ANSI escape sequences (颜色/粗体等)
        "-J",           // 合并屏幕宽度导致的自动换行
        "-S",
        &start_line,    // 从多少行前开始
    ])
}

/// 向 pane 发送按键
pub fn send_keys(target: &str, keys: &str, literal: bool) -> Result<(), String> {
    let mut args = vec!["send-keys", "-t", target];
    if literal {
        args.push("-l"); // literal mode，不解析特殊键
    }
    args.push(keys);
    run_tmux(&args)?;
    Ok(())
}

/// 向 pane 发送文本 + Enter
pub fn send_command(target: &str, command: &str) -> Result<(), String> {
    send_keys(target, command, true)?;
    send_keys(target, "Enter", false)?;
    Ok(())
}

/// 创建新 session
pub fn new_session(name: &str) -> Result<(), String> {
    run_tmux(&["new-session", "-d", "-s", name])?;
    Ok(())
}

/// 关闭 session
pub fn kill_session(name: &str) -> Result<(), String> {
    run_tmux(&["kill-session", "-t", name])?;
    Ok(())
}

/// 检查 tmux server 是否运行
pub fn is_server_running() -> bool {
    run_tmux(&["list-sessions"]).is_ok()
}
