//! Which agent CLI is running in a pane, and how to start it again.
//!
//! One table, two uses: detection during capture and the launch line `up`
//! replays. Keeping them together is the point — a detector that recognises
//! "codex" but relaunches something else would silently rebuild the wrong
//! workspace. P2 replaces this table with the real agent definitions on disk
//! (see `docs/exec-plans/projects-and-tasks.md` §5); until then it stays
//! deliberately small.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownAgent {
    /// Stable name we store in the slot.
    pub backend: &'static str,
    /// Lowercase substring searched in the pane's command text.
    pub needle: &'static str,
    /// What `up` runs when there is no conversation to go back to.
    pub launch: &'static str,
    /// Resume the newest conversation **of this directory**. `None` where the
    /// CLI has no directory-scoped resume — see `codex` below.
    pub resume_recent: Option<&'static str>,
    /// Resume one exact conversation; `{id}` is substituted.
    pub resume_id: Option<&'static str>,
}

use crate::tmux::TmuxPane;

/// Kimi must precede Kiro: a Kimi pane's process chain contains its
/// `kiro-web-search` helper, so an array-order rule would paint it as Kiro.
/// Earliest match in the text wins regardless (see `detect`), but keeping the
/// order meaningful documents the trap.
///
/// Resume flags are taken from the installed CLIs' own `--help`, not guessed:
///
/// * `kiro-cli chat -r/--resume` — "Resume the most recent conversation from
///   this directory"; `--resume-id <SESSION_ID>` for an exact one.
/// * `claude -c/--continue` — most recent conversation in this directory;
///   `--resume <id>` for an exact one.
/// * `codex resume <SESSION_ID>` — exact only. `codex resume --last` is
///   deliberately NOT used: it continues the most recent recorded session
///   machine-wide, so restoring project A could reopen project B's
///   conversation. Without a recorded id, codex starts fresh.
/// * kimi / openclaw — no resume wired up because their flags are unverified
///   here; they relaunch clean rather than guess.
const KNOWN: &[KnownAgent] = &[
    KnownAgent {
        backend: "kimi",
        needle: "kimi",
        launch: "kimi",
        resume_recent: None,
        resume_id: None,
    },
    KnownAgent {
        backend: "kiro",
        needle: "kiro",
        launch: "kiro-cli chat",
        resume_recent: Some("kiro-cli chat --resume"),
        resume_id: Some("kiro-cli chat --resume-id {id}"),
    },
    KnownAgent {
        backend: "claude",
        needle: "claude",
        launch: "claude",
        resume_recent: Some("claude --continue"),
        resume_id: Some("claude --resume {id}"),
    },
    KnownAgent {
        backend: "codex",
        needle: "codex",
        launch: "codex",
        resume_recent: None,
        resume_id: Some("codex resume {id}"),
    },
    KnownAgent {
        // grok 1.0.5, flags from its own --help: `-c/--continue` — "Continue
        // the most recent session for the current working directory" (cwd-
        // scoped, so safe, unlike codex's machine-wide --last); `--resume <id>`
        // for an exact session (UUID-shaped values always mean ids).
        backend: "grok",
        needle: "grok",
        launch: "grok",
        resume_recent: Some("grok --continue"),
        resume_id: Some("grok --resume {id}"),
    },
    KnownAgent {
        backend: "openclaw",
        needle: "openclaw",
        launch: "openclaw",
        resume_recent: None,
        resume_id: None,
    },
];

/// The longest agent name we accept — a tmux window name and a directory
/// component; anything longer is a mistake, not an identity.
pub const MAX_NAME_LEN: usize = 64;

/// Is `name` something an agent may be called?
///
/// The name is reused in FOUR places with four parsers, and the rule is the
/// intersection of what all of them treat as a plain word:
///
/// * a directory component under `<ws>/.tmm/agents/` — so no `/`, `\`, NUL,
///   and never the components `.` or `..` (`agent_remove("../..")` resolved to
///   the workspace itself, which `is_dir()`, and would have deleted it);
/// * a tmux window name that also appears inside targets (`session:name.pane`)
///   — so no `:` `.` `=` (tmux's target separators and exact-match prefix) and
///   no whitespace or glob characters (`*` `?` `[`), which tmux matches as a
///   pattern;
/// * the first argv element after a flag on the CLI (`tmm agent remove <name>`)
///   — so it cannot start with `-`;
/// * an `@name` address in chat — so no whitespace or `@`.
///
/// A whitelist is the only shape of that rule that stays true when a fifth
/// parser arrives: letters and digits (any script — a Chinese agent name is
/// an ordinary name), `-` and `_`, starting with a letter or digit, at most
/// `MAX_NAME_LEN` characters.
pub fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("agent name must not be empty".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("agent name must be at most {MAX_NAME_LEN} characters"));
    }
    if !name.chars().next().is_some_and(char::is_alphanumeric) {
        return Err(format!(
            "agent name '{name}' must start with a letter or digit — it is a window name, a directory and an @address"
        ));
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_'))) {
        return Err(format!(
            "agent name '{name}' cannot contain '{bad}' — only letters, digits, '-' and '_' survive tmux targets, paths and @addresses unchanged"
        ));
    }
    Ok(())
}

/// The isolated home of `name` under `workspace`, or `None` for a name that
/// must never become a path (see `valid_name`). Every `<ws>/.tmm/agents/<x>`
/// path in the projects module is built here, so no caller can skip the check.
pub fn home_dir(workspace: &str, name: &str) -> Option<std::path::PathBuf> {
    valid_name(name).ok()?;
    Some(std::path::Path::new(workspace).join(".tmm").join("agents").join(name))
}

/// The agent running in a pane, or `None` for an ordinary shell.
///
/// `text` must be ordered shallow → deep (`pane_current_command`, then the
/// title, then the foreground child's argv) because the EARLIEST match wins:
/// a late match is a subprocess the agent spawned, not what the user launched.
/// Claude Code needs no special case even though its process name is a bare
/// version number — its argv path contains `.../claude/versions/<v>`.
pub fn detect(text: &str) -> Option<&'static KnownAgent> {
    let lower = text.to_lowercase();
    let mut best: Option<(usize, &'static KnownAgent)> = None;
    for agent in KNOWN {
        if let Some(idx) = lower.find(agent.needle) {
            if best.is_none_or(|(prev, _)| idx < prev) {
                best = Some((idx, agent));
            }
        }
    }
    best.map(|(_, a)| a)
}

/// The agent in a MANAGED window: the launch recipe's recorded backend first,
/// the pane sniff (`detect`) as the fallback for windows we did not create.
///
/// The sniff is inherently wrong for some backends we ourselves spawned: the
/// npm-installed codex runs as `node` (`bin/codex.js` shim), its pane title is
/// the project name, and the window name is the agent's name — nothing says
/// "codex", so `detect` returned None and the window fell out of delivery,
/// the roster, vitals and recovery (found live 2026-08-22: a spawned cx-probe
/// never received its @mention). We WROTE the backend into `launch.json` at
/// spawn — for our own windows the record beats the sniff.
pub fn detect_managed(
    workspace: Option<&str>,
    window_name: &str,
    pane_text: &str,
) -> Option<&'static KnownAgent> {
    if let Some(recipe) = workspace.and_then(|ws| home_dir(ws, window_name)).map(|h| h.join("launch.json")) {
        if let Some(backend) = std::fs::read_to_string(recipe)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("backend").and_then(|b| b.as_str().map(str::to_owned)))
        {
            if let Some(agent) = KNOWN.iter().find(|a| a.backend == backend) {
                return Some(agent);
            }
        }
    }
    detect(pane_text)
}

/// Every clue a pane offers about the agent in it, shallow → deep, in the
/// order `detect` wants: `pane_current_command` (the process tmux started),
/// the pane title and the window name (labels the launcher set — for a
/// spawned window the name IS the agent's identity), then the foreground
/// child's argv (the deepest process, where an interpreter-launched CLI such
/// as the npm codex finally says its own name).
///
/// ONE function because seven callers used to build this string by hand and
/// one of them differently: the capturer looked at `child_cmd` and not the
/// window name, the roster/delivery/vitals/recovery looked at the window name
/// and not `child_cmd` — so the declaration and the roster could disagree
/// about whether the same window was an agent.
pub fn pane_text(pane: &TmuxPane) -> String {
    format!(
        "{} {} {} {}",
        pane.current_command, pane.pane_title, pane.window_name, pane.child_cmd
    )
}

/// The agent in `pane`, by its recipe first and every pane clue second — the
/// one way to ask about a pane (`detect_managed` over `pane_text`).
pub fn detect_pane(workspace: Option<&str>, pane: &TmuxPane) -> Option<&'static KnownAgent> {
    detect_managed(workspace, &pane.window_name, &pane_text(pane))
}

/// The launch line for a backend name we stored earlier.
///
/// Restoring a workspace should put you back in the conversation, not in a
/// blank prompt, so the exact conversation id wins when we have one, a
/// directory-scoped resume is the next best thing, and a clean start is the
/// last resort.
pub fn launch_line(backend: &str, session_id: Option<&str>) -> Option<String> {
    let agent = KNOWN.iter().find(|a| a.backend == backend)?;
    if let (Some(id), Some(template)) = (session_id.filter(|s| !s.is_empty()), agent.resume_id) {
        return Some(template.replace("{id}", id));
    }
    Some(agent.resume_recent.unwrap_or(agent.launch).to_string())
}

/// The plain launch line, ignoring any conversation history.
pub fn launch_for(backend: &str) -> Option<&'static str> {
    KNOWN.iter().find(|a| a.backend == backend).map(|a| a.launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_shallowest_agent_not_the_first_listed() {
        // codex spawning a kiro-web-search MCP tool: "kiro" sits deeper.
        let a = detect("node codex /Users/me/.codex/bin kiro-web-search").unwrap();
        assert_eq!(a.backend, "codex");
    }

    /// The rule that keeps a name from becoming a path or a tmux pattern.
    /// `../..` is the case that mattered: `agent_remove` joined it under
    /// `<ws>/.tmm/agents/`, landed on the workspace itself, and
    /// `remove_dir_all` would have taken the whole project.
    #[test]
    fn a_name_is_a_plain_word_or_nothing() {
        for ok in ["lead", "builder-2", "cx_probe", "经理", "a", "Z9"] {
            assert!(valid_name(ok).is_ok(), "{ok} is an ordinary name");
        }
        for bad in [
            "", ".", "..", "../..", "a/b", "a\\b", "-flag", "_lead", "a b", "a.b", "a:b", "a=b",
            "a*", "a?", "a[1]", "@lead", "a\0b", "a\nb",
        ] {
            assert!(valid_name(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(valid_name(&"x".repeat(MAX_NAME_LEN)).is_ok());
        assert!(valid_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    fn pane(cmd: &str, title: &str, window_name: &str, child: &str) -> TmuxPane {
        TmuxPane {
            session: "s".into(),
            window: 1,
            pane: 0,
            width: 80,
            height: 24,
            current_command: cmd.into(),
            window_name: window_name.into(),
            pane_title: title.into(),
            current_path: "/w".into(),
            active: true,
            child_cmd: child.into(),
        }
    }

    /// The two clues the old call sites disagreed on both count now: the
    /// window name (a spawned window is named after its agent) and the
    /// foreground child's argv (where an interpreter-launched CLI says its
    /// name). And the order stays shallow → deep, so a shell whose child is
    /// an agent is still the agent, while a label never outranks the process.
    #[test]
    fn a_pane_is_read_by_every_clue_in_one_order() {
        // The npm codex: `node` process, project title, agent-named window —
        // the window name is what says codex (roster's view).
        assert_eq!(detect_pane(None, &pane("node", "myproj", "codex-2", "")).map(|a| a.backend), Some("codex"));
        // Same process, anonymous window, but argv names it (capturer's view).
        assert_eq!(
            detect_pane(None, &pane("node", "myproj", "win3", "node /x/@openai/codex/bin/codex.js")).map(|a| a.backend),
            Some("codex")
        );
        // Shallow beats deep: the process tmux started wins over a subprocess.
        assert_eq!(
            detect_pane(None, &pane("kiro-cli", "chat", "w", "node kiro-web-search")).map(|a| a.backend),
            Some("kiro")
        );
        // A bare shell with an ordinary name is not an agent.
        assert!(detect_pane(None, &pane("zsh", "~", "shell", "")).is_none());
        assert_eq!(pane_text(&pane("a", "b", "c", "d")), "a b c d");
    }

    #[test]
    fn an_invalid_name_has_no_home() {
        assert!(home_dir("/tmp/ws", "../..").is_none());
        assert!(home_dir("/tmp/ws", "").is_none());
        assert_eq!(
            home_dir("/tmp/ws", "lead").as_deref(),
            Some(std::path::Path::new("/tmp/ws/.tmm/agents/lead"))
        );
    }

    #[test]
    fn kimi_wins_over_its_kiro_helper() {
        let a = detect("kimi-code kiro-web-search").unwrap();
        assert_eq!(a.backend, "kimi");
    }

    #[test]
    fn claude_is_found_through_its_version_named_binary_path() {
        let a = detect("2.1.141  /Users/me/.local/share/claude/versions/2.1.141").unwrap();
        assert_eq!(a.backend, "claude");
        assert_eq!(a.launch, "claude");
    }

    #[test]
    fn a_plain_shell_is_not_an_agent() {
        assert!(detect("zsh").is_none());
        assert!(detect("npm run dev").is_none());
        assert!(detect("").is_none());
    }

    #[test]
    fn launch_lines_round_trip_by_backend_name() {
        assert_eq!(launch_for("kiro"), Some("kiro-cli chat"));
        assert_eq!(launch_for("nope"), None);
    }

    #[test]
    fn a_recorded_conversation_id_beats_a_directory_resume() {
        assert_eq!(
            launch_line("kiro", Some("abc-123")).as_deref(),
            Some("kiro-cli chat --resume-id abc-123")
        );
        assert_eq!(
            launch_line("claude", Some("abc-123")).as_deref(),
            Some("claude --resume abc-123")
        );
        assert_eq!(
            launch_line("codex", Some("abc-123")).as_deref(),
            Some("codex resume abc-123")
        );
    }

    #[test]
    fn without_an_id_we_resume_this_directory_where_the_cli_can() {
        assert_eq!(launch_line("kiro", None).as_deref(), Some("kiro-cli chat --resume"));
        assert_eq!(launch_line("claude", None).as_deref(), Some("claude --continue"));
        // The generic path shares ~/.codex: --last could cross projects, so
        // no id means a clean start. Managed recipes use isolated CODEX_HOME
        // and may safely use cwd-filtered `resume --last` (spawn.rs).
        assert_eq!(launch_line("codex", None).as_deref(), Some("codex"));
        assert_eq!(launch_line("kimi", None).as_deref(), Some("kimi"));
        assert_eq!(launch_line("kimi", Some("x")).as_deref(), Some("kimi"), "no resume flags known");
        assert_eq!(launch_line("", Some("x")), None);
    }

    #[test]
    fn an_empty_id_is_not_a_conversation() {
        assert_eq!(launch_line("kiro", Some("")).as_deref(), Some("kiro-cli chat --resume"));
    }

    /// The exact live failure (2026-08-22): a spawned codex runs under the
    /// npm `node` shim — pane shows `cmd=node`, title = project name, window
    /// name = agent name; nothing says "codex", so the sniff misses and the
    /// window fell out of delivery/roster/vitals/recovery. The recipe we
    /// wrote at spawn is the record, so it wins for managed windows.
    #[test]
    fn a_managed_window_is_detected_by_its_recipe_not_its_process_name() {
        let ws = std::env::temp_dir().join(format!("tmm-detect-{}", std::process::id()));
        let home = ws.join(".tmm").join("agents").join("cx-probe");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("launch.json"), r#"{"backend":"codex","cmd":"command codex"}"#).unwrap();
        let pane_text = "node bedrock-e2e cx-probe"; // measured: cmd/title/window_name
        assert!(detect(pane_text).is_none(), "the sniff alone must miss — that is the bug");
        let hit = detect_managed(Some(ws.to_str().unwrap()), "cx-probe", pane_text);
        assert_eq!(hit.map(|a| a.backend), Some("codex"), "the recipe is the record");
        // No workspace (a hand-started window) still sniffs.
        assert!(detect_managed(None, "cx-probe", pane_text).is_none());
        assert_eq!(
            detect_managed(None, "w", "kiro-cli chat").map(|a| a.backend),
            Some("kiro")
        );
        // A recipe naming an unknown backend falls back to the sniff.
        std::fs::write(home.join("launch.json"), r#"{"backend":"martian"}"#).unwrap();
        assert!(detect_managed(Some(ws.to_str().unwrap()), "cx-probe", pane_text).is_none());
        std::fs::remove_dir_all(&ws).ok();
    }
}
