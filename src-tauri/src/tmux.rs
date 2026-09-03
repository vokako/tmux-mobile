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
    /// Current working directory of the pane's process (tmux pane_current_path).
    /// Used by the client to show a cwd hint alongside the command name.
    pub current_path: String,
    /// Whether this is the window's active pane. Clients use it to pick the
    /// representative pane for window chips (active pane's command/title,
    /// not whatever pane happens to list first).
    pub active: bool,
    /// argv of the foreground-most descendant of the pane's shell, e.g.
    /// "node /…/@openai/codex/bin/codex.js". `pane_current_command` only
    /// reports the immediate process name ("node", "2.1.141"), which is
    /// useless for detecting interpreter-launched agent CLIs — the real
    /// identity lives in the argv. Empty when the pane runs a bare shell.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub child_cmd: String,
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

pub(crate) fn run_tmux(args: &[&str]) -> Result<String, String> {
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

/// A session NAME as an exact `-t` target: `=name:`. tmux resolves a bare
/// target as exact name, then PREFIX of a name, then fnmatch glob — so with
/// only `dev-2` alive, `has-session -t dev` says yes and `kill-session -t dev`
/// kills `dev-2`; a name holding `*` or `?` is a pattern. `=` turns that off,
/// and the trailing `:` says "the session half of a target" — without it,
/// commands that take a target-WINDOW/pane (`list-panes -s`, `new-window`,
/// `display-message`) read `=dev` as a window name and fall back to the prefix
/// search anyway. `=name:` is accepted uniformly by has-session, kill-session,
/// rename-session, list-windows, list-panes, new-window and display-message
/// (verified on tmux 3.6a). `set-option` and `set-hook` targets reject `=`,
/// which is why `ensure_session`/`set_resize_hook` pass the bare name.
fn exact_session(session: &str) -> String {
    if session.starts_with('=') { session.to_string() } else { format!("={session}:") }
}

/// Quote a string for a tmux COMMAND LINE (a hook body, a key binding), where
/// tmux's own parser splits on spaces and `;` and expands `$VAR`. Double quotes
/// with `\`, `"` and `$` escaped survive that parser byte-for-byte (verified on
/// tmux 3.6a); single quotes cannot carry an embedded `'`. Not needed for argv
/// arguments — those go to tmux as separate strings and are never re-parsed.
fn tmux_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if matches!(c, '\\' | '"' | '$') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// 列出所有 session
pub fn list_sessions() -> Result<Vec<TmuxSession>, String> {
    // Delimiter: a printable token, NOT a control byte. tmux >= 3.4
    // octal-escapes control bytes (e.g. 0x1f -> "\037") in `-F` output, so a
    // unit-separator delimiter never survives and the whole line lands in one
    // field. A distinctive printable token is emitted verbatim and is
    // implausible in real session/window names or paths. See TMUX_FIELD_SEP.
    let output = run_tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}<TMM_SEP>#{session_windows}<TMM_SEP>#{session_attached}<TMM_SEP>#{session_activity}",
    ])?;

    let mut sessions: Vec<TmuxSession> = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split("<TMM_SEP>").collect();
            TmuxSession {
                name: parts.first().unwrap_or(&"").to_string(),
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

/// `session_name -> session_created` (unix seconds) for every live session.
/// The projects auto-tracker needs a session's AGE, which `list_sessions`
/// cannot answer: its `created` field actually carries `session_activity`
/// (it drives MRU sorting) and that ticks forward on every keystroke.
pub fn session_created_times() -> Vec<(String, u64)> {
    let out = match run_tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}<TMM_SEP>#{session_created}",
    ]) {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };
    out.lines()
        .filter_map(|line| {
            let mut it = line.split("<TMM_SEP>");
            match (it.next(), it.next()) {
                (Some(name), Some(created)) if !name.is_empty() => {
                    Some((name.to_string(), created.parse().unwrap_or(0)))
                }
                _ => None,
            }
        })
        .collect()
}

/// (window index, last activity unix seconds) for every window of `session`.
/// The telemetry fallback signal: a backend with no tool hooks still shows
/// "working" while its pane produces output.
pub fn window_activity_times(session: &str) -> Vec<(usize, u64)> {
    let target = exact_session(session);
    let out = match run_tmux(&[
        "list-windows",
        "-t",
        &target,
        "-F",
        "#{window_index}<TMM_SEP>#{window_activity}",
    ]) {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };
    out.lines()
        .filter_map(|line| {
            let mut it = line.split("<TMM_SEP>");
            match (it.next(), it.next()) {
                (Some(idx), Some(ts)) => Some((idx.parse().ok()?, ts.parse().unwrap_or(0))),
                _ => None,
            }
        })
        .collect()
}

/// 列出某个 session 的所有 pane
pub fn list_panes(session: &str) -> Result<Vec<TmuxPane>, String> {
    let target = exact_session(session);
    let output = run_tmux(&[
        "list-panes",
        "-s",
        "-t",
        &target,
        "-F",
        PANE_FORMAT,
    ])?;
    Ok(parse_pane_lines(&output))
}

const PANE_FORMAT: &str = "#{session_name}<TMM_SEP>#{window_index}<TMM_SEP>#{pane_index}<TMM_SEP>#{pane_width}<TMM_SEP>#{pane_height}<TMM_SEP>#{pane_current_command}<TMM_SEP>#{window_name}<TMM_SEP>#{pane_title}<TMM_SEP>#{pane_current_path}<TMM_SEP>#{pane_active}<TMM_SEP>#{pane_pid}";

/// Snapshot of the process table: pid -> (ppid, args). One `ps` subprocess
/// per pane-listing call, shared across all panes — far cheaper than a
/// per-pane lookup and portable across macOS / Linux.
fn process_table() -> std::collections::HashMap<u32, (u32, String)> {
    let mut map = std::collections::HashMap::new();
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,ppid=,args="]).output() else {
        return map;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `ps` right-aligns the numeric pid/ppid columns, so consecutive
        // fields are separated by RUNS of spaces, not single spaces. Read the
        // two leading numbers off a whitespace-collapsing iterator, then take
        // args as the rest of the line after the ppid token. (splitn on a
        // single whitespace char would yield an empty ppid field on every
        // padded row and drop the entire process table — the bug this fixes.)
        let mut it = line.split_whitespace();
        let (Some(pid_tok), Some(ppid_tok)) = (it.next(), it.next()) else { continue };
        let (Ok(pid), Ok(ppid)) = (pid_tok.parse::<u32>(), ppid_tok.parse::<u32>()) else { continue };
        // args = whatever the iterator has left, rejoined with single spaces.
        // (Original spacing inside argv is irrelevant — we only substring-match
        // against it.)
        let args = it.collect::<Vec<_>>().join(" ");
        map.insert(pid, (ppid, args));
    }
    map
}

/// Concatenated argv of the first-child chain under `root` (the pane's
/// shell), up to 4 levels deep. Agent CLIs sit directly under the shell or
/// behind one wrapper level (script launcher → node), but they also spawn
/// their own subprocesses (tool executions) — so we keep EVERY level's
/// argv, not just the deepest, and let the caller's substring matching
/// find the agent's name anywhere in the chain. Each level is capped so a
/// pathological argv doesn't bloat every pane listing.
fn descendant_cmd(table: &std::collections::HashMap<u32, (u32, String)>, root: u32) -> String {
    const MAX_ARGS_PER_LEVEL: usize = 160;
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for (&pid, &(ppid, _)) in table.iter() {
        children.entry(ppid).or_default().push(pid);
    }
    let mut cur = root;
    let mut acc = String::new();
    for _ in 0..4 {
        let Some(kids) = children.get(&cur) else { break };
        // Lowest pid = first-spawned ≈ the foreground job.
        let Some(&next) = kids.iter().min() else { break };
        if let Some((_, args)) = table.get(&next) {
            if !args.is_empty() {
                if !acc.is_empty() {
                    acc.push(' ');
                }
                let mut end = MAX_ARGS_PER_LEVEL.min(args.len());
                while end < args.len() && !args.is_char_boundary(end) {
                    end += 1;
                }
                acc.push_str(&args[..end]);
            }
        }
        cur = next;
    }
    acc
}

fn parse_pane_lines(output: &str) -> Vec<TmuxPane> {
    let mut panes: Vec<(TmuxPane, u32)> = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split("<TMM_SEP>").collect();
            let pane = TmuxPane {
                session: parts.first().unwrap_or(&"").to_string(),
                window: parts.get(1).unwrap_or(&"0").parse().unwrap_or(0),
                pane: parts.get(2).unwrap_or(&"0").parse().unwrap_or(0),
                width: parts.get(3).unwrap_or(&"0").parse().unwrap_or(0),
                height: parts.get(4).unwrap_or(&"0").parse().unwrap_or(0),
                current_command: parts.get(5).unwrap_or(&"").to_string(),
                window_name: parts.get(6).unwrap_or(&"").to_string(),
                pane_title: parts.get(7).unwrap_or(&"").to_string(),
                current_path: parts.get(8).unwrap_or(&"").to_string(),
                active: parts.get(9).unwrap_or(&"0") == &"1",
                child_cmd: String::new(),
            };
            let pid: u32 = parts.get(10).unwrap_or(&"0").parse().unwrap_or(0);
            (pane, pid)
        })
        .collect();
    // Single ps snapshot serves every pane in this listing.
    if panes.iter().any(|(_, pid)| *pid > 0) {
        let table = process_table();
        for (pane, pid) in panes.iter_mut() {
            if *pid > 0 {
                pane.child_cmd = descendant_cmd(&table, *pid);
            }
        }
    }
    panes.into_iter().map(|(p, _)| p).collect()
}

/// List panes across ALL sessions in one tmux call. Avoids the N+1 RPC
/// pattern where the client previously did `list_sessions` + N ×
/// `list_panes(session)`. tmux's `-a` flag iterates every server-known
/// session in a single subprocess; the client groups by session_name.
pub fn list_all_panes() -> Result<Vec<TmuxPane>, String> {
    let output = run_tmux(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    Ok(parse_pane_lines(&output))
}

/// Resolve tmux's stable pane id (for example `%18`) to the current target.
/// Agent lifecycle hooks inherit `TMUX_PANE`, so this is the authoritative
/// association between an agent event and the window shown by the client.
pub fn resolve_pane_id(pane_id: &str) -> Result<(String, usize, usize), String> {
    if !pane_id.starts_with('%') || !pane_id[1..].chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid tmux pane id".into());
    }
    let output = run_tmux(&[
        "display-message", "-t", pane_id, "-p",
        "#{session_name}<TMM_SEP>#{window_index}<TMM_SEP>#{pane_index}",
    ])?;
    let mut fields = output.trim().split("<TMM_SEP>");
    let session = fields.next().filter(|s| !s.is_empty()).ok_or("missing session")?.to_string();
    let window = fields.next().ok_or("missing window")?.parse().map_err(|_| "invalid window")?;
    let pane = fields.next().ok_or("missing pane")?.parse().map_err(|_| "invalid pane")?;
    Ok((session, window, pane))
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
    let x = parts.first().unwrap_or(&"0").parse().unwrap_or(0);
    let y = parts.get(1).unwrap_or(&"0").parse().unwrap_or(0);
    let h = parts.get(2).unwrap_or(&"24").parse().unwrap_or(24);
    let w = parts.get(3).unwrap_or(&"80").parse().unwrap_or(80);
    Ok((x, y, h, w))
}

/// Same as `cursor_info` but also returns `pane_current_command`. tmux's
/// `display-message` formats can carry both in a single subprocess call,
/// so this lets the subscription loop piggyback the running command on
/// every snapshot tick — replacing a separate per-pane 3 s polling RPC
/// from the client. Use this where the running command actually matters
/// (currently only `subscription_loop`); other call sites stay on the
/// shorter signature to avoid threading a String everywhere.
pub fn cursor_info_with_cmd(
    target: &str,
) -> Result<(usize, usize, usize, usize, String), String> {
    let out = run_tmux(&[
        "display-message",
        "-t",
        target,
        "-p",
        // Printable delimiter (see list_sessions): tmux 3.4 octal-escapes
        // control bytes in -F output, so a 0x1f delimiter does not survive.
        "#{cursor_x}<TMM_SEP>#{cursor_y}<TMM_SEP>#{pane_height}<TMM_SEP>#{pane_width}<TMM_SEP>#{pane_current_command}",
    ])?;
    let parts: Vec<&str> = out.trim_end_matches('\n').split("<TMM_SEP>").collect();
    let x = parts.first().unwrap_or(&"0").parse().unwrap_or(0);
    let y = parts.get(1).unwrap_or(&"0").parse().unwrap_or(0);
    let h = parts.get(2).unwrap_or(&"24").parse().unwrap_or(24);
    let w = parts.get(3).unwrap_or(&"80").parse().unwrap_or(80);
    let cmd = parts.get(4).unwrap_or(&"").to_string();
    Ok((x, y, h, w, cmd))
}

/// 捕获 pane 内容（屏幕输出，保留 ANSI 转义序列）
/// Uses -J to join tmux-wrapped lines, then fixes CJK double-width
/// wrapping where tmux doesn't set the WRAPPED flag (last column left
/// empty because a 2-cell character didn't fit).
pub fn capture_pane(target: &str, lines: Option<usize>) -> Result<String, String> {
    capture_pane_with_width(target, lines, 0).map(|(content, _)| content)
}

/// Capture pane text without ANSI escapes for prompt/control-flow detection.
pub fn capture_pane_plain(target: &str, lines: Option<usize>) -> Result<String, String> {
    let start_line = lines
        .map(|n| format!("-{}", n))
        .unwrap_or_else(|| format!("-{}", get_scrollback()));
    let output = run_tmux(&[
        "capture-pane",
        "-t",
        target,
        "-p",
        "-J",
        "-S",
        &start_line,
    ])?;
    Ok(output.trim_end().to_string())
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

    Ok((join_unflagged_wraps(trimmed, width), trailing_empty))
}

/// Join wrapped lines that tmux -J missed.
///
/// tmux -J already joins wraps that carry the WRAPPED flag. The one case it
/// misses: a double-width char didn't fit in the last column, so tmux left
/// that column empty WITHOUT setting the flag. Signature: visible width ==
/// pane_width - 1 AND the next line starts with the wide char that didn't
/// fit. Anything else — including exactly-full lines — is a complete row;
/// joining it shifts every following row (kimi's full-width box borders
/// swallowed their next line and the whole screen sheared).
pub fn join_unflagged_wraps(trimmed: &str, width: usize) -> String {
    let raw_lines: Vec<&str> = trimmed.split('\n').collect();
    let mut result = String::with_capacity(trimmed.len());
    let mut i = 0;
    while i < raw_lines.len() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(raw_lines[i]);
        while i + 1 < raw_lines.len() {
            if visible_width(raw_lines[i]) == width.saturating_sub(1)
                && first_visible_char(raw_lines[i + 1]).is_some_and(is_wide_char)
            {
                i += 1;
                result.push_str(raw_lines[i]);
            } else {
                break;
            }
        }
        i += 1;
    }
    result
}

/// First visible (non-escape) character of a line, skipping ANSI escapes
/// with the same rules as `visible_width`.
fn first_visible_char(s: &str) -> Option<char> {
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            return Some(c);
        }
    }
    None
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

/// tmux named key for a C0 control byte, for bytes that must not be sent
/// literally (see send_keys). `\t` `\n` `\r` and ESC stay literal: they are
/// ordinary text bytes (or, for ESC, the prefix of a longer sequence).
fn c0_key_name(c: char) -> Option<&'static str> {
    const CTRL_LETTERS: [&str; 26] = [
        "C-a", "C-b", "C-c", "C-d", "C-e", "C-f", "C-g", "C-h", "C-i", "C-j",
        "C-k", "C-l", "C-m", "C-n", "C-o", "C-p", "C-q", "C-r", "C-s", "C-t",
        "C-u", "C-v", "C-w", "C-x", "C-y", "C-z",
    ];
    match c {
        '\t' | '\n' | '\r' | '\x1b' => None,
        '\0' => Some("C-Space"),
        '\x01'..='\x1a' => Some(CTRL_LETTERS[c as usize - 1]),
        '\x1c' => Some("C-\\"),
        '\x1d' => Some("C-]"),
        '\x1e' => Some("C-^"),
        '\x1f' => Some("C-_"),
        _ => None,
    }
}

/// 向 pane 发送按键
///
/// Literal mode splits C0 control bytes out as tmux *named* keys: with
/// `extended-keys on`, tmux silently DROPS raw C0 bytes sent via
/// `send-keys -l` to panes whose app enabled an extended keyboard protocol
/// (modifyOtherKeys / kitty — `#{pane_key_mode}` = `Ext …`, which every
/// modern agent TUI does). Named keys are re-encoded by tmux to match the
/// pane's key mode, so they work for both legacy and extended panes.
/// An ESC directly before a control byte is the client's Ctrl+Alt encoding;
/// it becomes one `M-C-x` key so no stray ESC leaks into the pane.
pub fn send_keys(target: &str, keys: &str, literal: bool) -> Result<(), String> {
    if !literal {
        // `--` ends option parsing: a key string starting with `-` (`-R`,
        // `-X`) is a key name — or literal text, tmux's fallback for a name it
        // does not know — never a send-keys flag. Verified on tmux 3.6a.
        run_tmux(&["send-keys", "-t", target, "--", keys])?;
        return Ok(());
    }
    let flush = |buf: &mut String| -> Result<(), String> {
        if !buf.is_empty() {
            run_tmux(&["send-keys", "-t", target, "-l", "--", buf])?;
            buf.clear();
        }
        Ok(())
    };
    let mut buf = String::new();
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(name) = chars.peek().copied().and_then(c0_key_name) {
                chars.next();
                flush(&mut buf)?;
                run_tmux(&["send-keys", "-t", target, "--", &format!("M-{name}")])?;
                continue;
            }
        }
        match c0_key_name(c) {
            Some(name) => {
                flush(&mut buf)?;
                run_tmux(&["send-keys", "-t", target, "--", name])?;
            }
            None => buf.push(c),
        }
    }
    flush(&mut buf)?;
    Ok(())
}

/// 向 pane 发送文本 + Enter
pub fn send_command(target: &str, command: &str) -> Result<(), String> {
    if command.contains('\n') {
        // A newline cannot ride send-keys: literal mode passes it as a raw C0
        // byte inside `-l`, and with `extended-keys on` tmux silently DROPS
        // raw C0 bytes for panes in an extended key mode — every modern agent
        // TUI. The lines arrived GLUED together ("AgenticAI\nAgentic" reached
        // the composer as "AgenticAIAgentic"), which both corrupted what the
        // agent read and broke the delivery receipt (owner, 2026-08-24: "多行
        // 内容…没办法正确已读"). Naming it `Enter` instead would submit the
        // half-typed line. Bracketed paste is the one channel that carries a
        // newline INTO a composer: paste_text wraps the block in ?2004 markers
        // exactly when the pane app asked for them, and an agent TUI inserts
        // the break instead of submitting.
        paste_text(target, command)?;
    } else {
        send_keys(target, command, true)?;
    }
    // A beat between the text and the Enter. An agent TUI that receives the
    // burst back-to-back can treat the Enter as part of a paste and leave the
    // line sitting in its composer unsubmitted — measured live on codex-cli
    // 0.148.0 (a delivered @mention parked in the input box; grok/kiro/claude
    // tolerate the burst). Team's nudge_pane learned the same lesson earlier
    // ("the TUI needs a beat to settle"). 200 ms is imperceptible next to a
    // model turn; shells do not care either way.
    std::thread::sleep(std::time::Duration::from_millis(200));
    // kiro-cli's terminal UI (2.18.1) opens a fuzzy FILE PICKER the moment an
    // `@` is typed — pasted or keyed, idle or mid-turn — and filters it with
    // the text that follows. Every delivery starts `@name`, so in a workspace
    // holding a file that fuzzy-matches the agent's name (kirocrew-agentcore:
    // `@kiro` → `runtime/kirocrew-artifact.json`) the picker is still open when
    // our Enter arrives, and Enter there means SELECT: the whole mention became
    // `@file:runtime/kirocrew-artifact.json` and kiro answered "Received
    // runtime/kirocrew-artifact.json…" (owner, 2026-09-03, board #10 there).
    // Escape closes the picker and leaves the typed line intact, and while the
    // picker is open it reaches ONLY the picker — measured on a 12-second
    // streaming turn, which ran to its end. With no picker open, Escape is
    // kiro's cancel-agent / clear-queue key, so it is sent only when the
    // picker's own footer is on screen. Nothing else is touched: an `@`-less
    // line skips the capture, and no other backend draws that footer.
    if command.contains('@') && file_picker_open(&capture_pane_plain(target, Some(8)).unwrap_or_default()) {
        send_keys(target, "Escape", false)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    send_keys(target, "Enter", false)?;
    Ok(())
}

/// kiro-cli's `@` file picker, recognised by its footer ("esc to cancel · ↵ to
/// select"). Both halves are required: a running turn shows "Thinking…
/// (esc to cancel)" with nothing to select, and Escape there would cancel it.
pub fn file_picker_open(screen: &str) -> bool {
    screen
        .lines()
        .any(|l| l.contains("esc to cancel") && l.contains("to select"))
}

/// Paste text into a pane the way a real terminal does.
///
/// send_keys treats every byte as keystrokes, so newlines in a multi-line
/// paste arrive as Enter presses and each line executes. Real terminals use
/// bracketed paste: when the pane app enabled mode ?2004, the block is
/// wrapped in \x1b[200~ … \x1b[201~ and treated as ONE paste. tmux tracks
/// that state per pane, and `paste-buffer -p` inserts the wrapper exactly
/// when the app asked for it — legacy apps get the raw text, unchanged.
/// The text goes through `load-buffer -` (stdin) into a dedicated named
/// buffer so user buffers are untouched; -d deletes it after pasting.
pub fn paste_text(target: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut cmd = Command::new(find_tmux());
    if let Some(socket) = get_socket() {
        cmd.args(["-S", &socket]);
    }
    let mut child = cmd
        .args(["load-buffer", "-b", "tmm-paste", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run tmux: {}", e))?;
    child
        .stdin
        .take()
        .ok_or("tmux stdin unavailable")?
        .write_all(text.as_bytes())
        .map_err(|e| format!("Failed to write paste buffer: {}", e))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to run tmux: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "tmux error: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    run_tmux(&["paste-buffer", "-p", "-d", "-b", "tmm-paste", "-t", target])?;
    Ok(())
}

pub fn home_dir() -> String {
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
    // Check if session name already exists to prevent grouped sessions. Exact:
    // with only `dev-2` alive, `dev` must be creatable.
    if session_exists(name) {
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
    // Exact, or with no `dev` alive this would kill `dev-2`.
    run_tmux(&["kill-session", "-t", &exact_session(name)])?;
    Ok(())
}

/// 创建新 window（继承当前 pane 的工作目录）
pub fn new_window(session: &str) -> Result<(), String> {
    let target = exact_session(session);
    // Get the active pane's working directory so new window starts there
    let cwd = run_tmux(&["display-message", "-t", &target, "-p", "#{pane_current_path}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let dir = if cwd.is_empty() { home_dir() } else { cwd };
    if dir.is_empty() {
        run_tmux(&["new-window", "-t", &target])?;
    } else {
        run_tmux(&["new-window", "-t", &target, "-c", &dir])?;
    }
    Ok(())
}

/// 关闭 window
pub fn kill_window(target: &str) -> Result<(), String> {
    run_tmux(&["kill-window", "-t", target])?;
    Ok(())
}

/// True if a tmux session with this exact name exists.
pub fn session_exists(session: &str) -> bool {
    run_tmux(&["has-session", "-t", &exact_session(session)]).is_ok()
}

/// Rename a live session. `-t` is the exact current name; tmux refuses when the
/// new name is already taken, which is why the caller picks a free one first.
pub fn rename_session(from: &str, to: &str) -> Result<(), String> {
    run_tmux(&["rename-session", "-t", &exact_session(from), to]).map(|_| ())
}

/// All tmux session names beginning with `prefix` (e.g. "tmm-team-"). Used on
/// startup to recover teams that survived a server restart.
pub fn list_team_sessions(prefix: &str) -> Vec<String> {
    match run_tmux(&["list-sessions", "-F", "#{session_name}"]) {
        Ok(out) => out
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with(prefix))
            .map(|l| l.to_string())
            .collect(),
        Err(_) => Vec::new(), // no server / no sessions
    }
}

/// Ensure `session` exists (detached), creating it at `cwd` if missing. Used by
/// the in-process team supervisor before it spawns agent windows.
pub fn ensure_session(session: &str, cwd: &str) -> Result<(), String> {
    if session_exists(session) {
        return Ok(());
    }
    let dir = if cwd.is_empty() { home_dir() } else { cwd.to_string() };
    if dir.is_empty() {
        run_tmux(&["new-session", "-d", "-s", session])?;
    } else {
        run_tmux(&["new-session", "-d", "-s", session, "-c", &dir])?;
    }
    // Deep scrollback so an agent's history survives in its window. set-option
    // rejects an `=` target; the exact session exists as of the line above, and
    // tmux tries the exact name before any prefix, so the bare name is safe here.
    let _ = run_tmux(&["set-option", "-t", session, "history-limit", "100000"]);
    Ok(())
}

/// Find the active pane id of a window named `name` in `session`, if one exists.
/// Used by the team supervisor to adopt an already-open agent window instead of
/// launching a duplicate (idempotent across server restarts).
pub fn find_window_by_name(session: &str, name: &str) -> Option<String> {
    let out = run_tmux(&[
        "list-windows", "-t", &exact_session(session), "-F", "#{window_name}<TMM_SEP>#{pane_id}",
    ])
    .ok()?;
    for line in out.lines() {
        let mut it = line.split("<TMM_SEP>");
        if let (Some(wname), Some(pane)) = (it.next(), it.next()) {
            if wname == name {
                return Some(pane.to_string());
            }
        }
    }
    None
}

/// All `(window_name, pane_id)` pairs in `session`. Used by team recovery to
/// nudge every agent window back online after a server restart.
pub fn list_named_windows(session: &str) -> Vec<(String, String)> {
    match run_tmux(&["list-windows", "-t", &exact_session(session), "-F", "#{window_name}<TMM_SEP>#{pane_id}"]) {
        Ok(out) => out
            .lines()
            .filter_map(|line| {
                let mut it = line.split("<TMM_SEP>");
                match (it.next(), it.next()) {
                    (Some(n), Some(p)) if !n.is_empty() && !p.is_empty() => {
                        Some((n.to_string(), p.to_string()))
                    }
                    _ => None,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Create a new window named `name` in `session` rooted at `cwd`, returning the
/// new pane id (`%NN`). The window name lets the Team tab map an agent to its
/// pane by `window_name`.
pub fn new_named_window(session: &str, name: &str, cwd: &str) -> Result<String, String> {
    let dir = if cwd.is_empty() { home_dir() } else { cwd.to_string() };
    let target = exact_session(session);
    let mut args = vec!["new-window", "-t", &target, "-n", name, "-P", "-F", "#{pane_id}"];
    if !dir.is_empty() {
        args.push("-c");
        args.push(&dir);
    }
    let out = run_tmux(&args)?;
    Ok(out.trim().to_string())
}

/// Rename the window at `target` (`session:index`, `session:^`, or a pane id).
/// Used when reconciling a project: a freshly created session already owns one
/// window named after the shell, and the first slot takes it over instead of
/// leaving a stray window behind.
pub fn rename_window(target: &str, name: &str) -> Result<(), String> {
    run_tmux(&["rename-window", "-t", target, name]).map(|_| ())
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
    // The hook body is a tmux COMMAND LINE that tmux re-parses when it fires,
    // so the session name inside it is quoted — bare, a name with a space or
    // `;` became extra commands. (set-hook's own `-t` is argv and needs no
    // quoting; it does not accept an `=` prefix.)
    let hook_cmd = format!(
        "resize-window -A ; set-hook -u -t {} client-session-changed",
        tmux_quote(session)
    );
    run_tmux(&["set-hook", "-t", session, "client-session-changed", &hook_cmd])?;
    Ok(())
}

/// 检查 tmux server 是否运行
pub fn is_server_running() -> bool {
    run_tmux(&["list-sessions"]).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_session_prefixes_once() {
        assert_eq!(exact_session("dev"), "=dev:");
        assert_eq!(exact_session("=dev:"), "=dev:", "an already-exact target is left alone");
        assert_eq!(exact_session("a b"), "=a b:");
    }

    #[test]
    fn tmux_quote_escapes_what_the_command_parser_expands() {
        assert_eq!(tmux_quote("dev"), r#""dev""#);
        assert_eq!(tmux_quote("a b"), r#""a b""#);
        assert_eq!(tmux_quote("it's"), r#""it's""#, "an apostrophe needs no escape inside double quotes");
        assert_eq!(tmux_quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(tmux_quote("$HOME"), r#""\$HOME""#, "tmux expands $VAR inside double quotes");
        assert_eq!(tmux_quote(r"a\b"), r#""a\\b""#);
        assert_eq!(tmux_quote("x ; kill-server"), r#""x ; kill-server""#, "a quoted `;` is not a command separator");
    }

    /// Live against the running tmux: a session NAME resolves to itself and
    /// never to a sibling that merely starts with it. The names are unique per
    /// process so parallel suites sharing this tmux cannot collide.
    #[test]
    fn session_targets_are_exact_never_prefix_matches() {
        let base = format!("tmm-exact-{}", std::process::id());
        let sibling = format!("{base}-2");
        let _ = run_tmux(&["kill-session", "-t", &exact_session(&base)]);
        let _ = run_tmux(&["kill-session", "-t", &exact_session(&sibling)]);

        run_tmux(&["new-session", "-d", "-s", &sibling]).expect("create sibling");
        // Only `<base>-2` is alive. Bare `-t <base>` would prefix-match it.
        assert!(!session_exists(&base), "prefix match leaked into session_exists");
        assert!(list_panes(&base).is_err(), "list_panes answered for the sibling");
        new_session(&base, None, None).expect("the exact name is free, creation must succeed");
        assert!(session_exists(&base));
        assert!(session_exists(&sibling));
        assert_eq!(list_panes(&base).unwrap().len(), 1);

        kill_session(&base).expect("kill base");
        assert!(!session_exists(&base));
        assert!(session_exists(&sibling), "kill_session took the sibling instead of the exact name");
        // A glob in the name is a name, not a pattern.
        assert!(!session_exists(&format!("{base}*")));

        // A name with a space survives the hook body's re-parse.
        let spaced = format!("{base} sp");
        run_tmux(&["new-session", "-d", "-s", &spaced]).expect("create spaced");
        set_resize_hook(&spaced).expect("set_resize_hook on a spaced name");
        let hooks = run_tmux(&["show-hooks", "-t", &spaced]).unwrap_or_default();
        assert!(hooks.contains(&format!("-t \"{spaced}\"")), "hook body keeps the name quoted: {hooks}");

        kill_session(&spaced).expect("kill spaced");
        kill_session(&sibling).expect("kill sibling");
    }

    /// kiro-cli 2.18.1 screens as captured live (2026-09-03): the picker is
    /// recognised by its own footer, never by the turn's "esc to cancel".
    #[test]
    fn kiro_file_picker_is_recognised_by_its_footer_only() {
        let picker = "[tmm chat] human: @kiro [board #10] hello there\n\
                      @runtime/kirocrew-artifact.json\n\
                      ────────────────\n\
                       esc to cancel · ↵ to select\n";
        assert!(file_picker_open(picker));
        let streaming = "⠹ Thinking... (esc to cancel)\n\
                         ● 8 MCP failures — see /mcp\n\
                          Kiro is working · Type to steer · Ctrl+S to queue\n";
        assert!(!file_picker_open(streaming), "a running turn is not a picker");
        assert!(!file_picker_open("[tmm chat] human: @zzqx hello there\n"), "no match, no picker");
        assert!(!file_picker_open(""));
    }

    fn table(entries: &[(u32, u32, &str)]) -> std::collections::HashMap<u32, (u32, String)> {
        entries.iter().map(|&(pid, ppid, args)| (pid, (ppid, args.to_string()))).collect()
    }

    #[test]
    fn descendant_cmd_finds_interpreter_launched_agent() {
        // zsh(100) → node codex.js(200): the codex case where
        // pane_current_command only says "node".
        let t = table(&[
            (100, 1, "-zsh"),
            (200, 100, "node /Users/x/node_modules/@openai/codex/bin/codex.js"),
        ]);
        let cmd = descendant_cmd(&t, 100);
        assert!(cmd.contains("codex"), "got: {}", cmd);
    }

    #[test]
    fn descendant_cmd_keeps_all_levels() {
        // zsh(100) → claude(200) → tool subprocess(300). The agent's own
        // subprocess must not REPLACE the agent argv in the result.
        let t = table(&[
            (100, 1, "-zsh"),
            (200, 100, "claude --dangerously-skip-permissions"),
            (300, 200, "git status"),
        ]);
        let cmd = descendant_cmd(&t, 100);
        assert!(cmd.contains("claude"), "got: {}", cmd);
        assert!(cmd.contains("git status"), "got: {}", cmd);
    }

    #[test]
    fn descendant_cmd_idle_shell_is_empty() {
        let t = table(&[(100, 1, "-zsh")]);
        assert_eq!(descendant_cmd(&t, 100), "");
    }

    #[test]
    fn descendant_cmd_caps_runaway_argv() {
        let long = format!("node {}", "x".repeat(2000));
        let t = table(&[(100, 1, "-zsh"), (200, 100, long.as_str())]);
        assert!(descendant_cmd(&t, 100).len() <= 200);
    }

    // ── join_unflagged_wraps ──────────────────────────────────────────

    #[test]
    fn full_width_box_border_is_not_joined() {
        // kimi-style TUI frame: the border lines fill the pane exactly.
        // They are complete rows — joining them sheared the whole screen.
        let width = 10;
        let content = "╭────────╮\n│ hello  │\n╰────────╯";
        assert_eq!(visible_width("╭────────╮"), width);
        assert_eq!(join_unflagged_wraps(content, width), content);
    }

    #[test]
    fn unflagged_cjk_wrap_is_joined() {
        // 4 CJK chars = 8 cells; width 9 leaves the last column empty and
        // tmux forgets the WRAPPED flag. Next line starts with the wide
        // char that didn't fit → join.
        let content = "中文宽字\n符";
        assert_eq!(visible_width("中文宽字"), 8);
        assert_eq!(join_unflagged_wraps(content, 9), "中文宽字符");
    }

    #[test]
    fn width_minus_one_text_line_is_not_joined() {
        // A 9-cell ASCII line at width 10 is just a complete short row;
        // the next line starting with ASCII must stay separate.
        let content = "abcdefghi\njklmnop";
        assert_eq!(join_unflagged_wraps(content, 10), content);
    }

    #[test]
    fn chained_unflagged_wraps_join_across_segments() {
        // Each captured segment is width-1 wide and the continuation
        // starts wide → the chain collapses into one logical line.
        let content = "中中中中\n文文文文\n字";
        assert_eq!(join_unflagged_wraps(content, 9), "中中中中文文文文字");
    }

    #[test]
    fn ansi_escapes_are_ignored_when_matching() {
        // SGR-wrapped border: escapes must not count toward width and the
        // continuation check must skip escapes to find the first char.
        let border = "\x1b[31m╭────────╮\x1b[0m";
        let content = format!("{}\n\x1b[31m│ x      │\x1b[0m", border);
        assert_eq!(join_unflagged_wraps(&content, 10), content);
        let wrap = "\x1b[31m中文宽字\x1b[0m\n\x1b[31m符\x1b[0m";
        assert_eq!(join_unflagged_wraps(wrap, 9), "\x1b[31m中文宽字\x1b[0m\x1b[31m符\x1b[0m");
    }
}
