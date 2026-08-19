//! What the agent's own status line already says.
//!
//! An agent CLI paints a status line at the bottom of its pane — the model it is
//! on, how much of the context it has used, its reasoning effort, its cwd and
//! branch. We were showing none of it, while asking the owner to read it in the
//! terminal ("从输出的最后几行原始文本内容 加一下当前状态的嗅探，比如模型名 上下文
//! 长度 effort 之类的", 2026-08-19).
//!
//! There is no API for these: a CLI's live state lives in its own process. So we
//! read the LAST FEW LINES of the pane, which is the one place the CLI publishes
//! it. That makes this a sniffer, with a sniffer's contract: every field is
//! optional, an unreadable line yields `Vitals::default()` and NEVER an error,
//! and nothing downstream may assume a value is present. It is a nicety on top of
//! the hook-derived state, never a replacement — hooks are facts, this is a
//! reading of somebody else's screen.
//!
//! kiro's layout is documented by its own source: the status line is a fixed
//! order of segments joined by `·` —
//! `agent · autonomous · model · effort · context · tangent · codeIntel · goal`
//! on the left, `location · branch` on the right — and the context segment is
//! "Share of the context used" (a pie glyph plus `N%`, or `N% ctx` in lite mode).
//! We anchor on the AGENT NAME, which the caller already knows (a managed agent's
//! window name is its `--agent` name), and read the segments that follow it. When
//! the pane is too narrow the right-hand segments wrap onto their own lines, so
//! every line of the tail is parsed and the first reading of each field wins.

/// A reading of an agent's status line. Every field is a maybe: this is sniffed.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub struct Vitals {
    /// The model the CLI says it is using, e.g. `claude-opus-5`.
    pub model: Option<String>,
    /// Share of the context window USED, 0–100 (kiro's own wording).
    pub context_pct: Option<u8>,
    /// Reasoning effort, when the backend reports one: low|medium|high|xhigh|max.
    pub effort: Option<String>,
    /// Checked-out branch, as the status line shows it.
    pub branch: Option<String>,
}

impl Vitals {
    /// True when nothing at all could be read — the caller can omit the object.
    pub fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.context_pct.is_none()
            && self.effort.is_none()
            && self.branch.is_none()
    }
}

/// The effort words kiro accepts. A segment matching one of these IS the effort
/// segment, wherever it sits — matching by position alone would mistake a model
/// id for it whenever the effort segment is absent (it usually is).
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The pie glyphs kiro ramps through for context usage. They are the marker that
/// a bare `N%` is the context segment and not, say, a percentage in a diff.
const PIE: [char; 6] = ['○', '◔', '◑', '◕', '●', '◯'];

/// Read what the last lines of a pane say about the agent's current state.
///
/// `agent` is the name the CLI prints as its first segment — for a managed agent
/// that is its window name. It is used as an anchor, not as a filter: fields that
/// identify themselves by shape (context, effort, branch) are read even when the
/// anchor never appears, because a narrow pane wraps segments onto later lines.
pub fn sniff_kiro(pane: &str, agent: &str) -> Vitals {
    let mut v = Vitals::default();
    // Bottom-up: the newest paint of the status line is the last one.
    for line in pane.lines().rev().take(12) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let segs: Vec<&str> = line.split('·').map(str::trim).filter(|s| !s.is_empty()).collect();

        // The activity line ("Kiro is working · Type to queue · …") shares the
        // dot separator with the status line, and its words are not segments.
        if segs.iter().any(|s| s.starts_with("Kiro is ") || *s == "Type to queue") {
            continue;
        }

        for (i, seg) in segs.iter().enumerate() {
            if v.context_pct.is_none() {
                if let Some(pct) = context_pct(seg) {
                    v.context_pct = Some(pct);
                    continue;
                }
            }
            if v.effort.is_none() && EFFORTS.contains(&seg.to_ascii_lowercase().as_str()) {
                v.effort = Some(seg.to_ascii_lowercase());
                continue;
            }
            if v.branch.is_none() {
                if let Some(b) = branch(seg) {
                    v.branch = Some(b.to_string());
                    continue;
                }
            }
            // The model is positional — it is whatever follows the agent name
            // (and the optional `Autonomous` flag). Anchoring on the name the
            // caller gave us is what keeps a cwd or a tangent from being read as
            // a model id.
            if v.model.is_none() && i == 0 && *seg == agent {
                let next = segs
                    .iter()
                    .skip(1)
                    .find(|s| !s.eq_ignore_ascii_case("Autonomous"));
                if let Some(m) = next.filter(|m| looks_like_model(m)) {
                    v.model = Some((*m).to_string());
                }
            }
        }
    }
    v
}

/// `◕ 69%` (TUI) or `69% ctx` (lite) — a percentage that is about the context.
/// A bare `69%` is deliberately NOT accepted: percentages are everywhere in a
/// terminal, and a wrong reading here is worse than a missing one.
fn context_pct(seg: &str) -> Option<u8> {
    let s = seg.trim();
    let digits = if let Some(rest) = s.strip_prefix(|c| PIE.contains(&c)) {
        rest.trim_start()
    } else if let Some(rest) = s.strip_suffix("ctx") {
        rest.trim_end()
    } else {
        return None;
    };
    let n = digits.trim().strip_suffix('%')?;
    n.parse::<u16>().ok().filter(|n| *n <= 100).map(|n| n as u8)
}

/// `(feat/projects-and-tasks)` — kiro wraps the branch in parentheses, which is
/// what tells it apart from the cwd segment next to it.
fn branch(seg: &str) -> Option<&str> {
    let inner = seg.strip_prefix('(')?.strip_suffix(')')?.trim();
    // A branch name, not a sentence: no spaces, and something in it.
    if inner.is_empty() || inner.contains(' ') {
        return None;
    }
    Some(inner)
}

/// A model id is a lowercase slug: letters, digits, `-` and `.`, with at least
/// one separator (`claude-opus-5`, `gpt-5.1`, `auto` is the exception we allow by
/// name). Rejecting anything else is what keeps a path or a stray word out.
fn looks_like_model(s: &str) -> bool {
    if s == "auto" {
        return true;
    }
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.';
    !s.is_empty()
        && s.len() <= 48
        && s.chars().all(ok)
        && s.contains('-')
        && s.starts_with(|c: char| c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real capture of a managed kiro pane (tmux capture-pane -p, this repo,
    /// 2026-08-19). Everything below is measured, not imagined.
    const REAL: &str = "  the same blink sourced from the\n\
        \x20 text instead of from a clip.\n\
        \x20/quit to exit\n\
        ────────────────────────────────────────────────────\n\
        builder-2 · claude-opus-5 · ◔ 9%\n\
        /local/home/cfu/work/projects/tmux-mobile ·\n\
        (feat/projects-and-tasks)\n\
        \x20Kiro is working · Type to queue · Ctrl+S to steer\n";

    #[test]
    fn reads_a_real_kiro_status_line() {
        let v = sniff_kiro(REAL, "builder-2");
        assert_eq!(v.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(v.context_pct, Some(9));
        assert_eq!(v.branch.as_deref(), Some("feat/projects-and-tasks"));
        assert_eq!(v.effort, None, "kiro only shows effort when the backend reports one");
        assert!(!v.is_empty());
    }

    #[test]
    fn the_activity_line_is_not_a_status_line() {
        // It uses the same `·` separator, so it has to be skipped by name or
        // "working" and "queue" become segments to interpret.
        let v = sniff_kiro(" Kiro is working · Type to queue · Ctrl+S to steer\n", "builder-2");
        assert_eq!(v, Vitals::default());
    }

    #[test]
    fn effort_is_read_wherever_it_sits() {
        let v = sniff_kiro("worker · gpt-5.1 · high · ◑ 42%\n", "worker");
        assert_eq!(v.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(v.effort.as_deref(), Some("high"));
        assert_eq!(v.context_pct, Some(42));
        // Position alone would have read `high` as the model.
        let no_effort = sniff_kiro("worker · gpt-5.1 · ◑ 42%\n", "worker");
        assert_eq!(no_effort.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(no_effort.effort, None);
    }

    #[test]
    fn the_autonomous_flag_does_not_hide_the_model() {
        let v = sniff_kiro("bot · Autonomous · claude-sonnet-4.5 · ● 88%\n", "bot");
        assert_eq!(v.model.as_deref(), Some("claude-sonnet-4.5"));
        assert_eq!(v.context_pct, Some(88));
    }

    #[test]
    fn lite_mode_writes_the_percent_the_other_way_round() {
        let v = sniff_kiro("bot · auto · 7% ctx\n", "bot");
        assert_eq!(v.model.as_deref(), Some("auto"));
        assert_eq!(v.context_pct, Some(7));
    }

    #[test]
    fn a_bare_percentage_is_never_the_context() {
        // A terminal is full of percentages; reading one as the context usage
        // would put a confident wrong number on the card.
        for text in ["bot · claude-opus-5 · 69%\n", "Compacting… 42%\n", "[####  ] 60%\n"] {
            assert_eq!(sniff_kiro(text, "bot").context_pct, None, "{text}");
        }
    }

    #[test]
    fn a_model_is_only_read_next_to_the_agents_own_name() {
        // The anchor is what keeps a path, a tangent name or a stray word from
        // being reported as the model the agent is running.
        let v = sniff_kiro("somebody-else · claude-opus-5 · ◔ 9%\n", "builder-2");
        assert_eq!(v.model, None, "not our line");
        assert_eq!(v.context_pct, Some(9), "shape-identified fields still read");
        // A cwd is not a model even in the model's position.
        let path = sniff_kiro("bot · /local/home/cfu/work · ◔ 9%\n", "bot");
        assert_eq!(path.model, None);
    }

    #[test]
    fn the_newest_paint_wins() {
        // A pane holds every earlier status line in its scrollback; the last one
        // is the current state.
        let two = "bot · claude-opus-5 · ◔ 9%\nbot · gpt-5.1 · ● 90%\n";
        let v = sniff_kiro(two, "bot");
        assert_eq!(v.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(v.context_pct, Some(90));
    }

    #[test]
    fn nothing_readable_is_not_an_error() {
        for text in ["", "\n\n", "$ ls -la\ntotal 48\n", "no dots here at all"] {
            let v = sniff_kiro(text, "bot");
            assert!(v.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_branch_is_parenthesised_and_a_sentence_is_not() {
        assert_eq!(branch("(main)"), Some("main"));
        assert_eq!(branch("(feat/x-y)"), Some("feat/x-y"));
        assert_eq!(branch("()"), None);
        assert_eq!(branch("(no branch here)"), None);
        assert_eq!(branch("main"), None);
    }

    #[test]
    fn percentages_out_of_range_are_refused() {
        assert_eq!(context_pct("◔ 101%"), None);
        assert_eq!(context_pct("◔ 100%"), Some(100));
        assert_eq!(context_pct("◔ 0%"), Some(0));
        assert_eq!(context_pct("◔ abc%"), None);
    }
}
