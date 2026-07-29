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
        backend: "openclaw",
        needle: "openclaw",
        launch: "openclaw",
        resume_recent: None,
        resume_id: None,
    },
];

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
        // codex --last is machine-wide, so no id means a clean start rather
        // than somebody else's conversation.
        assert_eq!(launch_line("codex", None).as_deref(), Some("codex"));
        assert_eq!(launch_line("kimi", None).as_deref(), Some("kimi"));
        assert_eq!(launch_line("kimi", Some("x")).as_deref(), Some("kimi"), "no resume flags known");
        assert_eq!(launch_line("", Some("x")), None);
    }

    #[test]
    fn an_empty_id_is_not_a_conversation() {
        assert_eq!(launch_line("kiro", Some("")).as_deref(), Some("kiro-cli chat --resume"));
    }
}
