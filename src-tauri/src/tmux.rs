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
    /// Unix seconds of the last time this session was opened via tmux-mobile,
    /// persisted across server restarts. `None` if never opened from the app.
    /// Clients use this to sort the most-recently-used sessions to the top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_opened: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmuxPane {
    pub session: String,
    pub window: usize,
    pub pane: usize,
    pub width: usize,
    pub height: usize,
    pub current_command: String,
    pub window_name: String,
    pub pane_title: String,
}

use std::sync::{OnceLock, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};

static TMUX_SOCKET: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static SCROLLBACK: AtomicUsize = AtomicUsize::new(500);

pub fn set_scrollback(n: usize) { SCROLLBACK.store(n, Ordering::Relaxed); }
pub fn get_scrollback() -> usize { SCROLLBACK.load(Ordering::Relaxed) }

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
fn find_tmux() -> String {
    // Try PATH first via `which`
    if let Ok(output) = Command::new("which").arg("tmux").output() {
        if output.status.success() {
            let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !p.is_empty() {
                return p;
            }
        }
    }
    // Fallback: common locations
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/tmux", home),
        "/opt/homebrew/bin/tmux".into(),
        "/usr/local/bin/tmux".into(),
        "/usr/bin/tmux".into(),
        "/bin/tmux".into(),
        "/opt/local/bin/tmux".into(),
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return path.clone();
        }
    }
    "tmux".to_string()
}

fn run_tmux(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(find_tmux());
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
    // Use unit separator (\x1f) as delimiter — cannot appear in session names
    let output = run_tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}\x1f#{session_windows}\x1f#{session_attached}\x1f#{session_activity}",
    ])?;

    let mut sessions: Vec<TmuxSession> = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\x1f').collect();
            TmuxSession {
                name: parts.get(0).unwrap_or(&"").to_string(),
                windows: parts.get(1).unwrap_or(&"0").parse().unwrap_or(0),
                attached: parts.get(2).unwrap_or(&"0") == &"1",
                created: parts.get(3).unwrap_or(&"0").to_string(),
                last_opened: None,
            }
        })
        .collect();

    // Annotate with the last time each session was opened via tmux-mobile
    // (persisted, per-server). The client uses this to sort MRU-first.
    let usage = crate::config::get_session_usage();
    for s in sessions.iter_mut() {
        s.last_opened = usage.get(&s.name).copied();
    }

    // Sort by tmux session_activity timestamp descending as a baseline; the
    // client applies its own MRU-first sort on top using `last_opened`.
    sessions.sort_by(|a, b| {
        let ta: u64 = b.created.parse().unwrap_or(0);
        let tb: u64 = a.created.parse().unwrap_or(0);
        ta.cmp(&tb)
    });

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
        "#{session_name}\x1f#{window_index}\x1f#{pane_index}\x1f#{pane_width}\x1f#{pane_height}\x1f#{pane_current_command}\x1f#{window_name}\x1f#{pane_title}",
    ])?;

    let panes = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\x1f').collect();
            TmuxPane {
                session: parts.get(0).unwrap_or(&"").to_string(),
                window: parts.get(1).unwrap_or(&"0").parse().unwrap_or(0),
                pane: parts.get(2).unwrap_or(&"0").parse().unwrap_or(0),
                width: parts.get(3).unwrap_or(&"0").parse().unwrap_or(0),
                height: parts.get(4).unwrap_or(&"0").parse().unwrap_or(0),
                current_command: parts.get(5).unwrap_or(&"").to_string(),
                window_name: parts.get(6).unwrap_or(&"").to_string(),
                pane_title: parts.get(7).unwrap_or(&"").to_string(),
            }
        })
        .collect();

    Ok(panes)
}

/// Get current command of a pane
pub fn pane_command(target: &str) -> Result<String, String> {
    run_tmux(&[
        "display-message",
        "-t",
        target,
        "-p",
        "#{pane_current_command}",
    ])
    .map(|s| s.trim().to_string())
}

/// Get cursor position (x, y), pane height and pane width
pub fn cursor_info(target: &str) -> Result<(usize, usize, usize, usize), String> {
    let out = run_tmux(&[
        "display-message",
        "-t",
        target,
        "-p",
        "#{cursor_x},#{cursor_y},#{pane_height},#{pane_width}",
    ])?;
    let parts: Vec<&str> = out.trim().split(',').collect();
    let x = parts.get(0).unwrap_or(&"0").parse().unwrap_or(0);
    let y = parts.get(1).unwrap_or(&"0").parse().unwrap_or(0);
    let h = parts.get(2).unwrap_or(&"24").parse().unwrap_or(24);
    let w = parts.get(3).unwrap_or(&"80").parse().unwrap_or(80);
    Ok((x, y, h, w))
}

/// 捕获 pane 内容（屏幕输出，保留 ANSI 转义序列）
/// Uses -J to join tmux-wrapped lines, then fixes CJK double-width
/// wrapping where tmux doesn't set the WRAPPED flag (last column left
/// empty because a 2-cell character didn't fit).
pub fn capture_pane(target: &str, lines: Option<usize>) -> Result<String, String> {
    capture_pane_with_width(target, lines, 0).map(|(content, _)| content)
}

/// Capture with a known pane width (avoids extra tmux call).
/// Returns (content, lines_trimmed_from_end).
pub fn capture_pane_with_width(
    target: &str,
    lines: Option<usize>,
    width: usize,
) -> Result<(String, usize), String> {
    let start_line = lines
        .map(|n| format!("-{}", n))
        .unwrap_or_else(|| format!("-{}", get_scrollback()));
    let output = run_tmux(&[
        "capture-pane",
        "-t",
        target,
        "-p",
        "-e",
        "-J",
        "-S",
        &start_line,
    ])?;

    let trimmed = output.trim_end();

    // Count trailing empty pane rows that were removed by trim_end.
    // tmux terminates EVERY line with \n (including the last non-empty one),
    // so we subtract 1 to avoid counting that terminator as an empty line.
    let trailing_newlines = output[trimmed.len()..]
        .bytes()
        .filter(|&b| b == b'\n')
        .count();
    let trailing_empty = if trimmed.is_empty() {
        trailing_newlines
    } else {
        trailing_newlines.saturating_sub(1)
    };

    if width == 0 {
        return Ok((trimmed.to_string(), trailing_empty));
    }

    let raw_lines: Vec<&str> = trimmed.split('\n').collect();
    let mut result = String::with_capacity(trimmed.len());
    let mut i = 0;
    while i < raw_lines.len() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(raw_lines[i]);
        while i + 1 < raw_lines.len() {
            let vlen = visible_width(raw_lines[i]);
            if (vlen == width || vlen == width - 1) && !raw_lines[i + 1].is_empty() {
                i += 1;
                result.push_str(raw_lines[i]);
            } else {
                break;
            }
        }
        i += 1;
    }
    Ok((result, trailing_empty))
}

/// Count visible character width, skipping ANSI escapes, counting CJK as 2.
pub fn visible_width(s: &str) -> usize {
    let mut w = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            w += if is_wide_char(c) { 2 } else { 1 };
        }
    }
    w
}

fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x1100..=0x115F | 0x231A..=0x231B | 0x2329..=0x232A |
        0x23E9..=0x23F3 | 0x23F8..=0x23FA | 0x25FD..=0x25FE |
        0x2614..=0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 |
        0x26A1 | 0x26AA..=0x26AB | 0x26BD..=0x26BE |
        0x26C4..=0x26C5 | 0x26CE | 0x26D4 | 0x26EA |
        0x26F2..=0x26F3 | 0x26F5 | 0x26FA | 0x26FD |
        0x2702 | 0x2705 | 0x2708..=0x270D | 0x270F |
        0x2712 | 0x2714 | 0x2716 | 0x271D | 0x2721 |
        0x2728 | 0x2733..=0x2734 | 0x2744 | 0x2747 |
        0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 |
        0x2763..=0x2764 | 0x2795..=0x2797 | 0x27A1 |
        0x27B0 | 0x27BF | 0x2934..=0x2935 |
        0x2B05..=0x2B07 | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55 |
        0x2E80..=0x303E | 0x3040..=0x33BF | 0x3400..=0x4DBF |
        0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xA960..=0xA97F |
        0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE10..=0xFE6F |
        0xFF01..=0xFF60 | 0xFFE0..=0xFFE6 |
        0x1F000..=0x1FAFF | 0x20000..=0x2FA1F | 0x30000..=0x3134F
    )
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

fn home_dir() -> String {
    dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default())
}

/// Get the working directory of the active pane in a session/target.
pub fn pane_cwd(target: &str) -> Result<String, String> {
    let path = run_tmux(&["display-message", "-t", target, "-p", "#{pane_current_path}"])?;
    let path = path.trim().to_string();
    if path.is_empty() {
        Ok(home_dir())
    } else {
        Ok(path)
    }
}

/// 创建新 session
pub fn new_session(name: &str, path: Option<&str>, command: Option<&str>) -> Result<(), String> {
    // Check if session name already exists to prevent grouped sessions
    if run_tmux(&["has-session", "-t", name]).is_ok() {
        return Err(format!("session '{}' already exists", name));
    }
    let mut args = vec!["new-session", "-d", "-s", name];
    let resolved;
    let home = home_dir();
    if let Some(p) = path {
        if !p.is_empty() {
            resolved = crate::fs::resolve(p);
            args.push("-c");
            args.push(&resolved);
        } else if !home.is_empty() {
            args.push("-c");
            args.push(&home);
        }
    } else if !home.is_empty() {
        args.push("-c");
        args.push(&home);
    }
    let cmd_str;
    if let Some(cmd) = command {
        if !cmd.is_empty() {
            cmd_str = cmd.to_string();
            args.push(&cmd_str);
        }
    }
    run_tmux(&args)?;
    Ok(())
}

/// 关闭 session
pub fn kill_session(name: &str) -> Result<(), String> {
    run_tmux(&["kill-session", "-t", name])?;
    Ok(())
}

/// 创建新 window（继承当前 pane 的工作目录）
pub fn new_window(session: &str) -> Result<(), String> {
    // Get the active pane's working directory so new window starts there
    let cwd = run_tmux(&["display-message", "-t", session, "-p", "#{pane_current_path}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let dir = if cwd.is_empty() { home_dir() } else { cwd };
    if dir.is_empty() {
        run_tmux(&["new-window", "-t", session])?;
    } else {
        run_tmux(&["new-window", "-t", session, "-c", &dir])?;
    }
    Ok(())
}

/// 关闭 window
pub fn kill_window(target: &str) -> Result<(), String> {
    run_tmux(&["kill-window", "-t", target])?;
    Ok(())
}

/// Resize the window containing target pane to given cols × rows.
/// Only effective when no terminal client is attached to the session.
pub fn resize_pane(target: &str, cols: usize, rows: usize) -> Result<(), String> {
    run_tmux(&[
        "resize-window",
        "-t",
        target,
        "-x",
        &cols.to_string(),
        "-y",
        &rows.to_string(),
    ])?;
    Ok(())
}

/// Restore a window to auto-size based on attached clients.
pub fn run_resize_window_auto(target: &str) -> Result<(), String> {
    run_tmux(&["resize-window", "-t", target, "-A"])?;
    Ok(())
}

/// Set a tmux hook so the next client that attaches to this session auto-resizes windows.
pub fn set_resize_hook(session: &str) -> Result<(), String> {
    // client-session-changed fires when a client switches to this session.
    // The hook runs resize-window -A (auto-fit) then removes itself (one-shot).
    let hook_cmd = format!(
        "resize-window -A ; set-hook -u -t {} client-session-changed",
        session
    );
    run_tmux(&["set-hook", "-t", session, "client-session-changed", &hook_cmd])?;
    Ok(())
}

/// 检查 tmux server 是否运行
pub fn is_server_running() -> bool {
    run_tmux(&["list-sessions"]).is_ok()
}
