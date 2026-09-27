//! Per-backend knowledge for the agents-v2 spawn path (projects/spawn.rs).
//!
//! `shared` holds the backend-neutral launch helpers that used to live in
//! `team::backends_shared` (board #100): per-backend MCP rendering, the
//! launch-script pattern (the 2KB tty lesson) and startup-prompt confirmation.
//! Moved here so the spawn path owns its dependencies and the desktop Team
//! system can be deleted whole (docs/todo.md §A).

pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod grok;
pub(crate) mod kimi;
pub(crate) mod kiro;
pub(crate) mod omp;
pub(crate) mod shared;

/// The closed set of agent CLIs this app can spawn (board #101/#127). An enum
/// rather than a trait object on purpose: the set is closed, and every
/// per-concern method below is an EXHAUSTIVE match with one-line arms into
/// the backend's own file — so adding a backend is adding a variant, and the
/// compiler's error list is exactly the file the new backend must fill
/// (2026-09-07: adding omp missed registry_save; this makes that class of
/// omission a compile error). The `backend` field stays a STRING at rest
/// (state.db, recipes, the wire); `parse` runs once at each boundary
/// (tenet 8: validate at the door).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Kiro,
    Claude,
    Codex,
    Grok,
    Omp,
    Kimi,
}

/// Where a spawn's first prompt goes — see `Backend::first_prompt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstPrompt {
    /// Appended to the launch command as one quoted positional argument.
    LaunchLine,
    /// Typed into the pane by the startup waiter once the CLI's ready
    /// markers are on screen; the launch line carries no prompt.
    Typed,
}

impl Backend {
    /// Every backend, in the one canonical order (seeds, pickers, caps).
    pub const ALL: [Backend; 6] =
        [Backend::Kiro, Backend::Claude, Backend::Codex, Backend::Grok, Backend::Omp, Backend::Kimi];

    /// `ALL` as the string names — kept literally beside it so a `const` can
    /// borrow it (`SPAWNABLE_BACKENDS`); the roundtrip test pins the two in
    /// sync.
    pub const NAMES: [&'static str; 6] = ["kiro", "claude", "codex", "grok", "omp", "kimi"];

    /// The documented fallback where an absent backend means kiro (the
    /// registry dispatch's old inline `unwrap_or("kiro")`).
    pub const DEFAULT: Backend = Backend::Kiro;

    pub fn parse(name: &str) -> Option<Backend> {
        match name.trim() {
            "kiro" => Some(Backend::Kiro),
            "claude" => Some(Backend::Claude),
            "codex" => Some(Backend::Codex),
            "grok" => Some(Backend::Grok),
            "omp" => Some(Backend::Omp),
            "kimi" => Some(Backend::Kimi),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Kiro => "kiro",
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Grok => "grok",
            Backend::Omp => "omp",
            Backend::Kimi => "kimi",
        }
    }

    /// One backend as the CLIENT sees it (`backends_list`, board #130): the
    /// name, its avatar and colour-token RESOURCE NAMES, and the effort
    /// levels its editor offers. The two resource names are DERIVED from the
    /// name by convention (`/assets/<name>.svg`, `--backend-<name>`), never
    /// declared per backend — the test below pins that both files exist for
    /// every variant, so adding a backend without its avatar or token is a
    /// failing test, not a lettered fallback on the phone. Efforts ride along
    /// because the client kept a hand mirror of them and it had already
    /// drifted (omp's list existed here and nowhere on the client).
    pub fn describe(self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name(),
            "icon": format!("/assets/{}.svg", self.name()),
            "color": format!("--backend-{}", self.name()),
            "efforts": self.effort_values(),
        })
    }

    /// `describe` over ALL, in the canonical order — the first entry is the
    /// client's default (`DEFAULT` is `ALL[0]`, pinned below).
    pub fn list_json() -> serde_json::Value {
        serde_json::Value::Array(Backend::ALL.iter().map(|b| b.describe()).collect())
    }

    /// The reasoning-effort levels this backend's CLI accepts — a FIXED enum
    /// per backend, measured per CLI, never guessed (each list's provenance
    /// is on the backend file).
    pub fn effort_values(self) -> &'static [&'static str] {
        match self {
            Backend::Kiro => kiro::effort_values(),
            Backend::Claude => claude::effort_values(),
            Backend::Codex => codex::effort_values(),
            Backend::Grok => grok::effort_values(),
            Backend::Omp => omp::effort_values(),
            Backend::Kimi => kimi::effort_values(),
        }
    }

    /// Where a backend's Stop payload carries no reply text, the backend may
    /// know another source: kiro --v3 reads the GLOBAL session store (board
    /// #207); kimi reads the session wire inside `home`, the agent's managed
    /// home resolved by name at the call site (board #224) — `None` for a
    /// window this app did not spawn.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn reply_fallback(
        self,
        payload: &serde_json::Map<String, serde_json::Value>,
        home: Option<&std::path::Path>,
    ) -> Option<String> {
        match self {
            Backend::Kiro => kiro::reply_fallback(payload),
            Backend::Kimi => kimi::reply_fallback(payload, home?),
            _ => None,
        }
    }

    /// The kind ("completed"/"failed"/"permission_required"/"input_required")
    /// carried by one hook payload, read with this backend's own key spelling
    /// and screening rules — each arm lives on the backend's file (board
    /// #129). Compiled on every target: the mobile shell consumes the same
    /// inbox envelopes.
    pub fn normalize_kind(
        self,
        payload: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<&'static str, String> {
        match self {
            Backend::Kiro => kiro::normalize_kind(payload),
            Backend::Claude => claude::normalize_kind(payload),
            Backend::Codex => codex::normalize_kind(payload),
            Backend::Grok => grok::normalize_kind(payload),
            Backend::Omp => omp::normalize_kind(payload),
            Backend::Kimi => kimi::normalize_kind(payload),
        }
    }

    /// True when the payload marks the start of a new user turn (resets the
    /// auto-post dedup flag), in this backend's spelling.
    pub fn is_user_prompt_submit(self, payload: &serde_json::Value) -> bool {
        match self {
            Backend::Kiro => kiro::is_user_prompt_submit(payload),
            Backend::Claude => claude::is_user_prompt_submit(payload),
            Backend::Codex => codex::is_user_prompt_submit(payload),
            Backend::Grok => grok::is_user_prompt_submit(payload),
            Backend::Omp => omp::is_user_prompt_submit(payload),
            Backend::Kimi => kimi::is_user_prompt_submit(payload),
        }
    }

    /// True when a tool event is the CLI's own HOUSEKEEPING rather than the
    /// agent's work — kiro v3's post-turn `memory` auto-capture (board #227).
    /// The consumer records such a call only inside an open turn; outside one
    /// it must not reopen the turn. Only kiro has one today.
    pub fn is_housekeeping_tool(self, payload: &serde_json::Value) -> bool {
        match self {
            Backend::Kiro => kiro::is_housekeeping_tool(payload),
            _ => false,
        }
    }

    /// The child thread id when a hook payload comes from a SUB-AGENT thread
    /// of this window's agent rather than the agent itself (board #169). Only
    /// codex spawns in-process sub-agents whose hooks fire on the parent's
    /// pane; the other backends never report one.
    pub fn subagent_thread<'a>(self, payload: &'a serde_json::Value) -> Option<&'a str> {
        match self {
            Backend::Codex => codex::subagent_thread(payload),
            _ => None,
        }
    }

    /// True for claude's idle reminder — an event that must never read as an
    /// ask (board #75). Only claude has one.
    pub fn is_idle_nudge(self, payload: &serde_json::Value) -> bool {
        match self {
            Backend::Claude => claude::is_idle_nudge(payload),
            _ => false,
        }
    }

    /// The backend's detection/relaunch row for the KNOWN table
    /// (projects/agents.rs assembles it from ALL, board #129).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn known(self) -> crate::projects::agents::KnownAgent {
        match self {
            Backend::Kiro => kiro::known(),
            Backend::Claude => claude::known(),
            Backend::Codex => codex::known(),
            Backend::Grok => grok::known(),
            Backend::Omp => omp::known(),
            Backend::Kimi => kimi::known(),
        }
    }

    /// Render an agent's isolated home from its definition — prompt, backend
    /// config, hooks, launch command (board #128). Each arm is the backend's
    /// own file; `workspace` is read by claude and codex (trust pre-seeding).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        self,
        def: &crate::projects::store::RegAgent,
        window_name: &str,
        home: &std::path::Path,
        workspace: &std::path::Path,
        system_prompt: &str,
        skills: &[crate::projects::skills::ResolvedSkill],
    ) -> Result<crate::projects::spawn::Rendered, String> {
        match self {
            Backend::Kiro => kiro::render_kiro(def, window_name, home, workspace, system_prompt, skills),
            Backend::Claude => claude::render_claude(def, window_name, home, workspace, system_prompt, skills),
            Backend::Codex => codex::render_codex(def, window_name, home, workspace, system_prompt, skills),
            Backend::Grok => grok::render_grok(def, window_name, home, system_prompt, skills),
            Backend::Omp => omp::render_omp(def, window_name, home, system_prompt, skills),
            Backend::Kimi => kimi::render_kimi(def, window_name, home, workspace, system_prompt, skills),
        }
    }

    /// How a spawn's first prompt (the stamped `--brief`) reaches the CLI.
    /// Five CLIs take it as a trailing positional argument on the launch line;
    /// Kimi Code 2.0.2 has no positional prompt (`kimi <text>` is "unknown
    /// command …" and the pane falls back to the shell — measured live by
    /// claude, board #224) and its `-p` is one-shot non-interactive, so the
    /// brief is TYPED into the pane once the composer is up — the same
    /// delivery primitive every later message uses (tenet 5).
    pub fn first_prompt(self) -> FirstPrompt {
        match self {
            Backend::Kimi => FirstPrompt::Typed,
            _ => FirstPrompt::LaunchLine,
        }
    }

    /// The backend's resume dialect on one persisted identity command.
    pub fn resume_command(self, cmd: &str, id: Option<&str>) -> String {
        match self {
            Backend::Kiro => kiro::resume_command(cmd, id),
            Backend::Claude => claude::resume_command(cmd, id),
            Backend::Codex => codex::resume_command(cmd, id),
            Backend::Grok => grok::resume_command(cmd, id),
            Backend::Omp => omp::resume_command(cmd, id),
            Backend::Kimi => kimi::resume_command(cmd, id),
        }
    }

    /// Repair this backend's on-disk surface in a managed home (hooks,
    /// settings, channel keys, recipe backfill). Probes for its own files and
    /// no-ops when the home is not this backend's.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn refresh(
        self,
        home: &std::path::Path,
        window_name: &str,
        workspace: &std::path::Path,
        notifications: &crate::agent_notifications::AgentNotificationHub,
    ) -> bool {
        match self {
            Backend::Kiro => kiro::refresh(home, window_name, workspace, &notifications.helper_command("kiro")),
            Backend::Claude => claude::refresh(home, workspace, &notifications.helper_command("claude")),
            Backend::Codex => codex::refresh(home, &notifications.helper_command("codex")),
            Backend::Grok => grok::refresh(home, &notifications.helper_command("grok")),
            Backend::Omp => omp::refresh(home, &notifications.helper_command("omp")),
            Backend::Kimi => kimi::refresh(home, &notifications.helper_command("kimi")),
        }
    }

    /// Read the CLI's own status furniture from a pane capture — each
    /// backend paints its own dialect, and reading one CLI's screen with
    /// another's grammar yields confident nonsense (agent-status.md).
    /// `agent` anchors kiro's positional status line; the other dialects
    /// ignore it.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn sniff(self, pane: &str, agent: &str) -> crate::projects::vitals::Vitals {
        match self {
            Backend::Kiro => kiro::sniff_kiro(pane, agent),
            Backend::Claude => claude::sniff_claude(pane),
            Backend::Codex => codex::sniff_codex(pane),
            Backend::Grok => grok::sniff_grok(pane),
            Backend::Omp => omp::sniff_omp(pane),
            Backend::Kimi => kimi::sniff_kimi(pane),
        }
    }

    /// Ask the backend's own CLI for its model ids. `None` = no authoritative
    /// list exists (claude/codex/omp aliases) or the CLI is unavailable —
    /// validation then degrades to accept-all (models.rs module doc).
    pub fn models_fetch(self) -> Option<Vec<String>> {
        match self {
            Backend::Kiro => kiro::models_fetch(),
            Backend::Claude => claude::models_fetch(),
            Backend::Codex => codex::models_fetch(),
            Backend::Grok => grok::models_fetch(),
            Backend::Omp => omp::models_fetch(),
            Backend::Kimi => kimi::models_fetch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, FirstPrompt};

    #[test]
    fn parse_and_name_roundtrip_and_names_mirror_all() {
        for (i, b) in Backend::ALL.into_iter().enumerate() {
            assert_eq!(Backend::parse(b.name()), Some(b));
            assert_eq!(Backend::NAMES[i], b.name(), "NAMES stays in ALL's order");
        }
        assert_eq!(Backend::parse(" kiro "), Some(Backend::Kiro), "boundary input is trimmed");
        // kimi is the one CLI without a positional prompt (board #224).
        for b in Backend::ALL {
            let expected = if b == Backend::Kimi { FirstPrompt::Typed } else { FirstPrompt::LaunchLine };
            assert_eq!(b.first_prompt(), expected, "{}", b.name());
        }
        assert_eq!(Backend::parse("openclaw"), None, "detection-only, never spawnable");
        assert_eq!(Backend::DEFAULT.name(), "kiro");
    }

    /// Backend names are spelled in `src/backends/` and nowhere else (board
    /// #101/#131; tenet 2: adding a backend touches one file). A quoted
    /// literal outside it is knowledge that has leaked — the class of drift
    /// that let omp miss `registry_save` (2026-09-07). The scan reads every
    /// `src/**/*.rs`, ignores comments, and skips exactly three things:
    /// tests (a file's column-0 `#[cfg(test)]` module and any `*test*` file),
    /// the store.rs seed region fenced by `// backend-seeds:begin/end` (a
    /// seed is registry data), and a statement whose preceding comment block
    /// carries `// backend-quirk(measured): …` — the marker for a measured,
    /// screen-triggered adaptation that generic code has to keep (tmux.rs's
    /// codex beat and kiro picker; the owner accepted those, 2026-09-09).
    /// Marker comments, never line numbers: the exemption moves with the
    /// code it explains.
    #[test]
    fn backend_literals_live_only_in_backends() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        walk(&root, &mut files);
        files.sort();
        assert!(files.len() > 20, "the walk found only {} files", files.len());

        let literals: Vec<String> = Backend::NAMES.iter().map(|n| format!("\"{n}\"")).collect();
        let mut leaks: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        for path in &files {
            let rel = path.strip_prefix(&root).unwrap().display().to_string();
            if rel.starts_with("backends/") || rel.contains("test") {
                continue;
            }
            scanned += 1;
            let src = std::fs::read_to_string(path).unwrap();
            let lines: Vec<&str> = src.lines().collect();
            let end = lines.iter().position(|l| l.starts_with("#[cfg(test)]")).unwrap_or(lines.len());
            let mut in_seeds = false;
            let mut quirk = false;
            for line in &lines[..end] {
                let t = line.trim_start();
                if t.starts_with("//") {
                    if t.contains("backend-seeds:begin") {
                        in_seeds = true;
                    } else if t.contains("backend-seeds:end") {
                        in_seeds = false;
                    } else if t.contains("backend-quirk(measured):") {
                        quirk = true;
                    }
                    continue;
                }
                // Code line: strip a trailing comment, then look for a literal.
                let code = match t.find("//") {
                    Some(k) if !t[..k].contains('"') => &t[..k],
                    _ => t,
                };
                let hit = literals.iter().any(|l| code.contains(l.as_str()));
                if hit && !in_seeds && !quirk {
                    leaks.push(format!("{rel}: {}", t.trim_end()));
                }
                // A quirk marker covers the statement it introduces; the first
                // code line after the comment block ends its reach.
                if !t.is_empty() {
                    quirk = false;
                }
            }
        }
        assert!(scanned >= 20, "scanned only {scanned} files");
        assert!(
            leaks.is_empty(),
            "backend names spelled outside src/backends/ (move the knowledge, or fence a seed / mark a measured quirk): {leaks:#?}"
        );
        // The fences must still be there for the exemption to mean anything —
        // in whichever store file holds the seeds (store/ split, board #147).
        let fenced = files.iter().any(|p| {
            let s = std::fs::read_to_string(p).unwrap();
            s.contains("backend-seeds:begin") && s.contains("backend-seeds:end")
        });
        assert!(fenced, "no file carries the backend-seeds fence");
        let tmux = std::fs::read_to_string(root.join("tmux.rs")).unwrap();
        assert!(tmux.matches("backend-quirk(measured):").count() >= 2, "tmux.rs lost its quirk markers");
    }

    /// `describe` derives resource names by convention; this is where the
    /// convention is enforced (board #130): every spawnable backend ships
    /// its avatar under public/assets and its colour token in app.css, and
    /// the list's first entry is the documented default.
    #[test]
    fn every_backend_has_its_avatar_and_colour_token() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let css = std::fs::read_to_string(root.join("src/app.css")).unwrap();
        for b in Backend::ALL {
            let icon = root.join("public/assets").join(format!("{}.svg", b.name()));
            assert!(icon.is_file(), "{} avatar missing: {}", b.name(), icon.display());
            assert!(css.contains(&format!("--backend-{}:", b.name())), "{} colour token missing from app.css", b.name());
        }
        let list = Backend::list_json();
        assert_eq!(list[0]["name"], Backend::DEFAULT.name());
        assert_eq!(list.as_array().unwrap().len(), Backend::ALL.len());
        assert_eq!(list[4]["efforts"].as_array().unwrap().len(), Backend::Omp.effort_values().len());
        assert_eq!(list[5]["efforts"].as_array().unwrap().len(), Backend::Kimi.effort_values().len());
    }
}
