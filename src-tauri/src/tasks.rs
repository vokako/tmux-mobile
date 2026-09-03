//! `tmm task` — background tasks as tmux windows. See
//! `docs/design-docs/features/tmm-cli.md`.
//!
//! LOCAL ONLY: nothing here touches the network. `tmm task *` works with no
//! tmux-mobile server running, which is the whole point — the thing an agent
//! most often wants to background *is* the server.
//!
//! Design contract (load-bearing, all three verified on tmux 3.7b):
//! - `remain-on-exit on` is what makes a task observable after it ends: the
//!   pane goes `#{pane_dead}=1` with the exit code in `#{pane_dead_status}`,
//!   and the scrollback stays readable. So status and log retention come from
//!   ONE native mechanism — no pidfiles, no sentinel files, no log files.
//! - It MUST be set with `-w` (window scope). Session scope would turn it on
//!   for every window the user has open in that session, so their shells would
//!   stop closing on exit. That is not ours to change.
//! - The registry is the `@tmm_task` window option, not a file: one
//!   `list-windows -a` call enumerates every task in every session. An agent
//!   whose context was compacted can rediscover what it left running, which a
//!   remembered PID can never do. `@tmm_cmd` / `@tmm_started` ride along so
//!   `list` needs no second lookup.
//!
//! Task names are GLOBALLY unique (the name is the handle an agent holds), so
//! lookups scan all sessions rather than just the current one.

use crate::tmux;
use std::time::{SystemTime, UNIX_EPOCH};

/// Same separator the team helpers in `tmux.rs` use for `-F` output.
const SEP: &str = "<TMM_SEP>";
/// Where tasks land when the caller is not inside tmux and named no session.
pub const FALLBACK_SESSION: &str = "tmm-tasks";

const OPT_TASK: &str = "@tmm_task";
const OPT_CMD: &str = "@tmm_cmd";
const OPT_STARTED: &str = "@tmm_started";

/// How long `stop` waits for the C-c to land before escalating to signals.
const STOP_GRACE_MS: u64 = 2_000;
const SIGNAL_GRACE_MS: u64 = 1_000;
const POLL_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Running,
    /// The command exited; tmux kept the pane and its scrollback.
    Exited(i32),
    /// A signal ended it — tmux names it (`kill`, `int`, `term`). Kept distinct
    /// from `Exited` because reporting a signal death as an exit code is a lie
    /// an agent would then act on.
    Killed(String),
}

/// Failures are typed rather than stringly, because the CLI maps them onto
/// tmm's tiered exit codes and sniffing message text for that would rot.
#[derive(Debug)]
pub enum Error {
    /// The request cannot work as asked: bad name, no command, or a name a
    /// live task already holds.
    Invalid(String),
    /// No task by that name.
    NotFound(String),
    /// tmux itself failed — not running, target gone, no permission.
    Tmux(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(m) | Error::NotFound(m) => write!(f, "{m}"),
            Error::Tmux(m) => write!(f, "{m}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Task {
    pub name: String,
    pub session: String,
    pub window: String,
    /// Pane id (`%NN`). Used as the tmux target because it is stable across
    /// window renumbering, unlike `session:index`.
    pub pane: String,
    pub cmd: String,
    pub state: State,
    pub pid: String,
    /// Unix seconds, 0 when unknown.
    pub started: u64,
}

impl Task {
    pub fn target(&self) -> String {
        format!("{}:{}", self.session, self.window)
    }

    pub fn state_str(&self) -> String {
        match &self.state {
            State::Running => "running".into(),
            State::Exited(code) => format!("exited:{code}"),
            State::Killed(sig) => format!("killed:{sig}"),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Seconds since start, or `None` when the start time is unknown — which is
    /// not the same thing as "just started".
    pub fn age(&self, now: u64) -> Option<u64> {
        if self.started == 0 {
            None
        } else {
            Some(now.saturating_sub(self.started))
        }
    }
}

/// Every task in every session. Never fails: no tmux server means no tasks.
pub fn list() -> Vec<Task> {
    match tmux::run_tmux(&["list-windows", "-a", "-F", &list_format()]) {
        Ok(out) => out.lines().filter_map(parse_line).collect(),
        Err(_) => Vec::new(),
    }
}

/// Field order here is the field order `parse_line` reads. `@tmm_cmd` goes last
/// because it is the only field that may contain anything at all.
fn list_format() -> String {
    [
        format!("#{{{OPT_TASK}}}"),
        "#{session_name}".to_string(),
        "#{window_index}".to_string(),
        "#{pane_id}".to_string(),
        "#{pane_dead}".to_string(),
        "#{pane_dead_status}".to_string(),
        "#{pane_dead_signal}".to_string(),
        "#{pane_pid}".to_string(),
        format!("#{{{OPT_STARTED}}}"),
        format!("#{{{OPT_CMD}}}"),
    ]
    .join(SEP)
}

/// The task called `name`, wherever it lives.
pub fn find(name: &str) -> Option<Task> {
    list().into_iter().find(|t| t.name == name)
}

/// Start `argv` detached in its own tmux window called `name`.
///
/// The window is created empty and then respawned with the command, because the
/// `remain-on-exit` option has to be in place BEFORE the command runs — a
/// command that exits in milliseconds would otherwise take its window (and its
/// output) down with it.
pub fn start(
    name: &str,
    argv: &[String],
    session: Option<&str>,
    replace: bool,
) -> Result<Task> {
    validate_name(name)?;
    if argv.is_empty() {
        return Err(Error::Invalid(
            "no command — usage: tmm task start <name> -- <cmd...>".into(),
        ));
    }
    let cmd = join_cmd(argv);
    let session = match session {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => current_session().unwrap_or_else(|| FALLBACK_SESSION.to_string()),
    };
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let pane = match find(name) {
        Some(t) if t.is_running() && !replace => {
            return Err(Error::Invalid(format!(
                "task '{name}' is already running in {} (pid {}) — stop it, or pass --replace",
                t.target(),
                t.pid
            )));
        }
        // Dead, or --replace: reuse the window when it is already in the target
        // session, otherwise drop it so the task does not exist twice.
        Some(t) if t.session == session => t.pane,
        Some(t) => {
            // If the old window will not go, starting a second one would make
            // the task exist twice — the one thing the registry promises not to.
            tmux::kill_window(&t.pane).map_err(Error::Tmux)?;
            create_window(&session, name, &cwd)?
        }
        None => create_window(&session, name, &cwd)?,
    };

    let now = unix_now();
    set_opt(&pane, "remain-on-exit", "on")?;
    set_opt(&pane, OPT_TASK, name)?;
    set_opt(&pane, OPT_CMD, &cmd)?;
    set_opt(&pane, OPT_STARTED, &now.to_string())?;
    tmux::run_tmux(&["respawn-window", "-k", "-t", &pane, &cmd]).map_err(Error::Tmux)?;

    find(name).ok_or_else(|| Error::Tmux(format!("started '{name}' but it vanished from tmux")))
}

/// Last `limit` lines of the task's output, optionally only lines containing
/// `grep` (case-insensitive substring). The scan covers the whole scrollback;
/// only the returned slice is bounded, because the caller is usually an agent
/// paying for every line in its context.
pub fn logs(name: &str, limit: usize, grep: Option<&str>) -> Result<String> {
    let task = need(name)?;
    let out = tmux::run_tmux(&["capture-pane", "-p", "-J", "-S", "-", "-t", &task.pane])
        .map_err(Error::Tmux)?;
    let out = if task.is_running() {
        out
    } else {
        strip_dead_marker(&out)
    };
    Ok(tail_lines(out.trim_end(), limit, grep))
}

/// Ask the task to stop, escalating only as far as it has to: C-c first (a real
/// TTY, so the whole foreground process group gets it — this is what a
/// `nohup`-ed process cannot be given), then TERM, then KILL on the pane's
/// process. The window is left in place either way so the log survives.
pub fn stop(name: &str) -> Result<Task> {
    let task = need(name)?;
    if !task.is_running() {
        return Ok(task);
    }
    tmux::send_keys(&task.pane, "C-c", false).map_err(Error::Tmux)?;
    if let Some(t) = wait_dead(name, STOP_GRACE_MS) {
        return Ok(t);
    }
    for sig in ["-TERM", "-KILL"] {
        if !task.pid.is_empty() {
            let _ = std::process::Command::new("kill")
                .args([sig, &task.pid])
                .output();
        }
        if let Some(t) = wait_dead(name, SIGNAL_GRACE_MS) {
            return Ok(t);
        }
    }
    Err(Error::Tmux(format!(
        "task '{name}' ignored C-c, TERM and KILL — inspect it with `tmux attach -t {}`",
        task.target()
    )))
}

/// Forget a finished task, closing its window. Refuses while it runs: removing
/// a live task would look like it stopped, and its log would be gone.
pub fn remove(name: &str) -> Result<Task> {
    let task = need(name)?;
    if task.is_running() {
        return Err(Error::Invalid(format!(
            "task '{name}' is still running — `tmm task stop {name}` first"
        )));
    }
    tmux::kill_window(&task.pane).map_err(Error::Tmux)?;
    Ok(task)
}

fn need(name: &str) -> Result<Task> {
    find(name).ok_or_else(|| Error::NotFound(format!("no task '{name}' — try `tmm task list`")))
}

/// Poll for up to `budget_ms` waiting for the task's pane to go dead.
fn wait_dead(name: &str, budget_ms: u64) -> Option<Task> {
    let mut waited = 0;
    while waited < budget_ms {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        waited += POLL_MS;
        match find(name) {
            Some(t) if !t.is_running() => return Some(t),
            Some(_) => {}
            // The window went away entirely (remain-on-exit lost): nothing to
            // report, but it is certainly not running.
            None => return None,
        }
    }
    None
}

/// A detached window, so starting a task never steals the user's focus.
/// (`tmux::new_named_window` deliberately does take focus — the Team tab wants
/// that when it opens an agent.)
fn create_window(session: &str, name: &str, cwd: &str) -> Result<String> {
    tmux::ensure_session(session, cwd).map_err(Error::Tmux)?;
    let mut args = vec![
        "new-window", "-d", "-t", session, "-n", name, "-P", "-F", "#{pane_id}",
    ];
    if !cwd.is_empty() {
        args.push("-c");
        args.push(cwd);
    }
    Ok(tmux::run_tmux(&args).map_err(Error::Tmux)?.trim().to_string())
}

fn set_opt(pane: &str, name: &str, value: &str) -> Result<()> {
    tmux::run_tmux(&["set-option", "-w", "-t", pane, name, value])
        .map(|_| ())
        .map_err(Error::Tmux)
}

/// The session the caller sits in, or `None` when it is not inside tmux.
/// Gated on `$TMUX`: without it `display-message` would answer for whichever
/// session tmux considers current, and the task would land somewhere random.
fn current_session() -> Option<String> {
    if std::env::var("TMUX").ok().filter(|v| !v.is_empty()).is_none() {
        return None;
    }
    let pane = std::env::var("TMUX_PANE").unwrap_or_default();
    let out = if pane.is_empty() {
        tmux::run_tmux(&["display-message", "-p", "#{session_name}"])
    } else {
        tmux::run_tmux(&["display-message", "-p", "-t", &pane, "#{session_name}"])
    };
    out.ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Unix seconds. Shared with the CLI so ages are computed against one clock.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- pure helpers

/// A task name has to survive being pasted into a tmux target (`session:window`
/// with `.pane`), and has to not look like a flag.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Invalid("task name is empty".into()));
    }
    if name.starts_with('-') {
        return Err(Error::Invalid(format!(
            "task name '{name}' cannot start with '-'"
        )));
    }
    if let Some(bad) = name.chars().find(|c| matches!(c, ':' | '.') || c.is_whitespace()) {
        return Err(Error::Invalid(format!(
            "task name '{name}' cannot contain '{bad}' — tmux reads ':' and '.' as target separators"
        )));
    }
    Ok(())
}

/// Quote one argv element for `/bin/sh`, which is what tmux runs the command
/// with. Building the string ourselves is the point: an agent passes argv, so
/// nothing it contains can turn into shell syntax.
fn sh_quote(arg: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./=:,@+".contains(c);
    if !arg.is_empty() && arg.chars().all(safe) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

fn join_cmd(argv: &[String]) -> String {
    argv.iter()
        .map(|a| sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One `list-windows -F` row → a task, or `None` for windows that are not tasks
/// (no `@tmm_task`) or rows tmux truncated.
fn parse_line(line: &str) -> Option<Task> {
    let f: Vec<&str> = line.split(SEP).collect();
    if f.len() < 10 || f[0].is_empty() {
        return None;
    }
    let state = if f[4] != "1" {
        State::Running
    } else if !f[6].is_empty() {
        State::Killed(f[6].to_string())
    } else {
        // Dead with neither a status nor a signal: tmux told us nothing, so say
        // -1 rather than inventing a success.
        State::Exited(f[5].parse().unwrap_or(-1))
    };
    Some(Task {
        name: f[0].to_string(),
        session: f[1].to_string(),
        window: f[2].to_string(),
        pane: f[3].to_string(),
        state,
        pid: f[7].to_string(),
        started: f[8].parse().unwrap_or(0),
        // The command can contain the separator only if a user put it there;
        // rejoin so it survives round-tripping regardless.
        cmd: f[9..].join(SEP),
    })
}

/// tmux writes `Pane is dead (…)` into the pane itself, on the bottom row, and
/// pads the gap above it with blank rows. That is tmux UI text, not the task's
/// output, and leaving it in would spend a bounded tail entirely on padding —
/// the real last lines would fall out of view. So `logs` returns output only
/// and `status` stays the one place that says how the task ended. Applied only
/// to dead tasks, so a running task's output is never second-guessed.
fn strip_dead_marker(text: &str) -> String {
    text.lines()
        .filter(|l| !(l.starts_with("Pane is dead (") && l.ends_with(')')))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Last `limit` lines, optionally filtered first.
fn tail_lines(text: &str, limit: usize, grep: Option<&str>) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    if let Some(pat) = grep {
        let pat = pat.to_lowercase();
        lines.retain(|l| l.to_lowercase().contains(&pat));
    }
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

/// Compact age for `list`, so an agent can tell a stuck task from a new one
/// without doing arithmetic on timestamps. `None` (unknown start) reads as "-".
pub fn fmt_age(secs: Option<u64>) -> String {
    match secs {
        None => "-".into(),
        Some(s) if s < 60 => format!("{s}s"),
        Some(s) if s < 3600 => format!("{}m", s / 60),
        Some(s) if s < 86400 => format!("{}h", s / 3600),
        Some(s) => format!("{}d", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_what_needs_it() {
        assert_eq!(sh_quote("npm"), "npm");
        assert_eq!(sh_quote("tauri:dev:release"), "tauri:dev:release");
        assert_eq!(sh_quote("--release"), "--release");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        // The reason this function exists: shell metacharacters in an agent's
        // argv must not become shell syntax.
        assert_eq!(sh_quote("; rm -rf /"), "'; rm -rf /'");
        assert_eq!(sh_quote("$(whoami)"), "'$(whoami)'");
    }

    #[test]
    fn joins_argv_into_one_sh_command() {
        let argv: Vec<String> = ["npm", "run", "tauri:dev:release"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(join_cmd(&argv), "npm run tauri:dev:release");

        let argv: Vec<String> = ["sh", "-c", "echo hi; exit 3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(join_cmd(&argv), "sh -c 'echo hi; exit 3'");
    }

    #[test]
    fn rejects_names_tmux_would_misread() {
        assert!(validate_name("build").is_ok());
        assert!(validate_name("build-web_2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("--replace").is_err());
        assert!(validate_name("a:b").is_err());
        assert!(validate_name("a.b").is_err());
        assert!(validate_name("a b").is_err());
    }

    fn row(fields: &[&str]) -> String {
        fields.join(SEP)
    }

    /// Field order: task, session, window, pane, dead, status, signal, pid,
    /// started, cmd.
    #[test]
    fn parses_a_running_task() {
        let t = parse_line(&row(&[
            "dev", "tmux", "3", "%518", "0", "", "", "4242", "1700000000", "npm run dev",
        ]))
        .expect("row is a task");
        assert_eq!(t.name, "dev");
        assert_eq!(t.state, State::Running);
        assert_eq!(t.state_str(), "running");
        assert_eq!(t.target(), "tmux:3");
        assert_eq!(t.pane, "%518");
        assert_eq!(t.pid, "4242");
        assert_eq!(t.cmd, "npm run dev");
        assert!(t.is_running());
        assert_eq!(t.age(1700000030), Some(30));
        // A task started "now" is 0s old, not unknown.
        assert_eq!(fmt_age(t.age(1700000000)), "0s");
    }

    #[test]
    fn parses_exit_code_of_a_finished_task() {
        let t = parse_line(&row(&[
            "build", "tmm-tasks", "1", "%9", "1", "7", "", "0", "0", "cargo build",
        ]))
        .expect("row is a task");
        assert_eq!(t.state, State::Exited(7));
        assert_eq!(t.state_str(), "exited:7");
        assert!(!t.is_running());
        // Unknown start time must not become a bogus age.
        assert_eq!(t.age(1700000000), None);
    }

    #[test]
    fn a_signal_death_is_not_an_exit_code() {
        let t = parse_line(&row(&[
            "x", "s", "1", "%1", "1", "", "kill", "0", "0", "sleep 30",
        ]))
        .unwrap();
        assert_eq!(t.state, State::Killed("kill".into()));
        assert_eq!(t.state_str(), "killed:kill");
        assert!(!t.is_running());
    }

    #[test]
    fn ignores_windows_that_are_not_tasks() {
        // A plain window: @tmm_task is empty.
        assert!(parse_line(&row(&["", "tmux", "1", "%1", "0", "", "", "1", "0", ""])).is_none());
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn dead_pane_without_a_status_is_not_reported_as_success() {
        let t = parse_line(&row(&["x", "s", "1", "%1", "1", "", "", "0", "0", "c"])).unwrap();
        assert_eq!(t.state, State::Exited(-1));
    }

    #[test]
    fn command_containing_the_separator_round_trips() {
        let t = parse_line(&row(&[
            "x", "s", "1", "%1", "0", "", "", "1", "0", "echo <TMM_SEP> hi",
        ]))
        .unwrap();
        assert_eq!(t.cmd, "echo <TMM_SEP> hi");
    }

    #[test]
    fn dead_marker_and_its_padding_leave_the_log() {
        // The exact shape capture-pane returns for a finished task: output,
        // blank rows to the bottom of the pane, then tmux's own annotation.
        let captured = "starting\nwork done\n\n\n\nPane is dead (signal int, Wed Aug  5 07:30:08 2026)";
        let cleaned = strip_dead_marker(captured);
        assert_eq!(tail_lines(cleaned.trim_end(), 2, None), "starting\nwork done");
        // Stripped wherever it sits, leaving no gap behind.
        assert_eq!(
            strip_dead_marker("Pane is dead (status 0, x)\nreal output"),
            "real output"
        );
    }

    #[test]
    fn tail_is_bounded_and_keeps_the_end() {
        let text = "a\nb\nc\nd";
        assert_eq!(tail_lines(text, 2, None), "c\nd");
        assert_eq!(tail_lines(text, 99, None), "a\nb\nc\nd");
        assert_eq!(tail_lines("", 5, None), "");
    }

    #[test]
    fn grep_is_case_insensitive_and_still_bounded() {
        let text = "ok\nERROR one\nfine\nerror two\nlast";
        assert_eq!(tail_lines(text, 10, Some("error")), "ERROR one\nerror two");
        // Bounded to the LAST matches, which is what a tail means.
        assert_eq!(tail_lines(text, 1, Some("error")), "error two");
        assert_eq!(tail_lines(text, 10, Some("nope")), "");
    }

    #[test]
    fn ages_read_at_a_glance() {
        assert_eq!(fmt_age(None), "-");
        assert_eq!(fmt_age(Some(0)), "0s");
        assert_eq!(fmt_age(Some(5)), "5s");
        assert_eq!(fmt_age(Some(59)), "59s");
        assert_eq!(fmt_age(Some(60)), "1m");
        assert_eq!(fmt_age(Some(3599)), "59m");
        assert_eq!(fmt_age(Some(3600)), "1h");
        assert_eq!(fmt_age(Some(86399)), "23h");
        assert_eq!(fmt_age(Some(86400)), "1d");
    }
}
