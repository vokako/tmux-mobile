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

/// How long a remembered reading may stand in for a fresh one. A model or a branch
/// does not change between polls, and a context percentage that is a minute old is
/// far better than a card that blinks empty. Past this the agent has probably been
/// doing something else entirely, so the reading is dropped rather than aged.
const VITALS_TTL_SECS: u64 = 300;

/// Last good reading per (session, window). Sniffing reads somebody else's screen
/// at an arbitrary instant, so a miss is normal: the pane may be mid-repaint, a
/// tool's output may have pushed the status line up, a panel may be open. Treating
/// each miss as "no information" is what made the card flicker (owner, 2026-08-19:
/// "context window 和模型状态信息，有时候会闪没了 … 可以多维持缓存一会儿").
fn cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<(String, usize), (Vitals, u64)>,
> {
    static C: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(String, usize), (Vitals, u64)>>,
    > = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Vitals {
    /// Fill the fields this reading did not get from a previous one. FIELD BY
    /// FIELD, not all-or-nothing: a pane commonly shows the wrapped branch line
    /// while the status line itself has scrolled off, and half a reading is still
    /// half a reading.
    pub fn backfill(&mut self, prev: &Vitals) {
        if self.model.is_none() {
            self.model = prev.model.clone();
        }
        if self.context_pct.is_none() {
            self.context_pct = prev.context_pct;
        }
        if self.effort.is_none() {
            self.effort = prev.effort.clone();
        }
        if self.branch.is_none() {
            self.branch = prev.branch.clone();
        }
    }
}

/// Sniff a pane and remember what was read, filling gaps from the last reading.
///
/// This is what `hub_agents` calls. The sniffers stay pure (and tested) — the
/// memory lives here, keyed by session and window, expiring after
/// `VITALS_TTL_SECS` so a long-dead reading cannot follow an agent around.
/// `backend` picks the dialect: every CLI paints its own status furniture
/// (kiro a `·`-joined status line, grok a header ratio + a boxed footer), and
/// reading one CLI's screen with another CLI's grammar yields confident
/// nonsense, which is worse than nothing.
pub fn sniff_remembered(
    session: &str,
    window: usize,
    pane: &str,
    agent: &str,
    backend: &str,
) -> Vitals {
    let mut v = match backend {
        "grok" => sniff_grok(pane),
        _ => sniff_kiro(pane, agent),
    };
    let now = now_secs();
    let key = (session.to_string(), window);
    let mut map = cache().lock().unwrap();
    if let Some((prev, at)) = map.get(&key) {
        if now.saturating_sub(*at) <= VITALS_TTL_SECS {
            v.backfill(prev);
        }
    }
    if v.is_empty() {
        // Nothing known at all: do not store an empty reading over a good one.
        map.remove(&key);
        return v;
    }
    map.insert(key, (v.clone(), now));
    v
}

/// Forget readings for windows that no longer exist — the same housekeeping
/// telemetry does, called from the same place.
pub fn retain_windows(session: &str, live: &[usize]) {
    cache().lock().unwrap().retain(|(s, w), _| s != session || live.contains(w));
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

/// Read what grok's screen says about its current state. grok 1.0.5 paints two
/// fixtures (measured live, 2026-08-21/22):
///
/// - a header line, cwd left + context ratio right: `/w/reports   47K / 500K`
///   ("上下文长度在右上角" — the owner's words for where to look). The ratio is
///   used / total tokens, so the percentage is computed, not read.
/// - the input box's bottom border carries the model (and the approval mode):
///   `╰──────── Grok 4.6 (Bedrock) · always-approve ─╯`.
///
/// No agent-name anchor exists in either fixture, so both fields identify
/// themselves BY SHAPE: the ratio must be `N[K|M] / N[K|M]` at the end of a
/// line, the model must sit in a `╰…╯` border. Bottom-up, newest paint wins —
/// the footer is redrawn at the bottom, and stale headers scroll upward.
pub fn sniff_grok(pane: &str) -> Vitals {
    let mut v = Vitals::default();
    for line in pane.lines().rev() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if v.model.is_none() {
            if let Some(m) = grok_footer_model(line) {
                v.model = Some(m);
            }
        }
        if v.context_pct.is_none() {
            if let Some(pct) = grok_context_ratio(line) {
                v.context_pct = Some(pct);
            }
        }
        if v.model.is_some() && v.context_pct.is_some() {
            break;
        }
    }
    v
}

/// The model out of grok's input-box bottom border: `╰─── <model> [· mode] ─╯`.
/// The border glyphs are the marker — ordinary output does not draw box
/// corners — and the FIRST `·`-segment inside is the model; what follows is
/// the approval mode (`always-approve`), which changes per keypress and is not
/// a vital.
fn grok_footer_model(line: &str) -> Option<String> {
    let s = line.trim();
    if !(s.starts_with('╰') && s.ends_with('╯')) {
        return None;
    }
    let inner = s.trim_matches(|c| matches!(c, '╰' | '╯' | '─')).trim();
    let model = inner.split('·').next()?.trim();
    // An empty border (`╰────╯`, no label) is the box with nothing to say.
    if model.is_empty() || model.chars().all(|c| c == '─' || c.is_whitespace()) {
        return None;
    }
    Some(model.to_string())
}

/// `47K / 500K` at the END of a line → percentage of the context used. Both
/// sides must parse as token counts and the ratio must make sense (used ≤
/// total); a `3 / 5` in ordinary output fails the K/M requirement on the
/// total, which is what keeps arithmetic in a diff from becoming a reading.
fn grok_context_ratio(line: &str) -> Option<u8> {
    let s = line.trim_end();
    let (head, total_txt) = s.rsplit_once('/')?;
    let total_txt = total_txt.trim();
    let used_txt = head.trim_end().rsplit(char::is_whitespace).next()?;
    // The total is a model's context budget: it always carries a magnitude
    // suffix (500K, 2M). Requiring it filters out fractions in ordinary text.
    if !total_txt.ends_with(['K', 'M']) {
        return None;
    }
    let used = grok_tokens(used_txt)?;
    let total = grok_tokens(total_txt)?;
    if total == 0.0 || used > total {
        return None;
    }
    Some((used * 100.0 / total).round().clamp(0.0, 100.0) as u8)
}

/// `47K` → 47_000, `1.2M` → 1_200_000, `800` → 800.
fn grok_tokens(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, mult) = match s.strip_suffix('M') {
        Some(n) => (n, 1_000_000.0),
        None => match s.strip_suffix('K') {
            Some(n) => (n, 1_000.0),
            None => (s, 1.0),
        },
    };
    let n: f64 = num.trim().parse().ok()?;
    (n >= 0.0).then_some(n * mult)
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

    /// A real capture of a managed grok pane (tmux capture-pane -p, grok 1.0.5,
    /// 2026-08-21). Header carries cwd + context ratio, the input box's bottom
    /// border carries the model and the approval mode.
    const GROK: &str = "\n\
        \x20 /local/home/cfu/work/reports          47K / 500K\n\
        \n\
        \x20    Worked for 2m8s            stop  [hooks: 1]\n\
        \n\
        \x20 ╭──────────────────────────────────────────────╮\n\
        \x20 │ ❯                                            │\n\
        \x20 ╰──────── Grok 4.6 (Bedrock) · always-approve ─╯\n\
        \n\
        \x20 Shift+Tab:mode  │  Ctrl+x:shortcuts\n";

    #[test]
    fn reads_a_real_grok_pane() {
        let v = sniff_grok(GROK);
        assert_eq!(v.model.as_deref(), Some("Grok 4.6 (Bedrock)"));
        assert_eq!(v.context_pct, Some(9), "47K of 500K rounds to 9%");
        assert_eq!(v.effort, None);
        assert_eq!(v.branch, None, "grok paints no branch");
    }

    #[test]
    fn a_grok_footer_without_the_mode_still_names_the_model() {
        // Wide pane, no `· always-approve` segment (measured, test-grok:2).
        let v = sniff_grok(
            "  /local/home/cfu                        13K / 500K\n\
             \x20 ╰───────────────────────── Grok 4.6 (Bedrock) ─╯\n",
        );
        assert_eq!(v.model.as_deref(), Some("Grok 4.6 (Bedrock)"));
        assert_eq!(v.context_pct, Some(3));
    }

    #[test]
    fn grok_ratios_in_ordinary_output_are_not_context() {
        // Fractions and paths are everywhere; only `N[K|M] / N[K|M]` with a
        // suffixed total at the end of a line is the header ratio.
        for text in [
            "passed 3 / 5 tests\n",
            "progress: 47 / 500\n",
            "  a/b\n",
            "download 900K / 1G\n", // G is not a context magnitude grok paints
        ] {
            assert_eq!(sniff_grok(text).context_pct, None, "{text:?}");
        }
        // An empty box border is not a model.
        assert_eq!(sniff_grok("  ╰────────╯\n").model, None);
        // Used above total is a misread, not a reading.
        assert_eq!(sniff_grok("  /w  600K / 500K\n").context_pct, None);
    }

    #[test]
    fn grok_token_shapes_parse() {
        assert_eq!(grok_tokens("47K"), Some(47_000.0));
        assert_eq!(grok_tokens("1.2M"), Some(1_200_000.0));
        assert_eq!(grok_tokens("800"), Some(800.0));
        assert_eq!(grok_tokens("abcK"), None);
        // Full window reads 100, empty reads 0.
        assert_eq!(grok_context_ratio("  /w  500K / 500K"), Some(100));
        assert_eq!(grok_context_ratio("  /w  0K / 500K"), Some(0));
    }

    #[test]
    fn a_reading_fills_its_gaps_from_the_last_one() {
        // The flicker: this capture caught the pane between paints, so only the
        // wrapped branch line was there.
        let mut half = sniff_kiro("(feat/x)\n", "bot");
        assert_eq!(half.model, None);
        assert_eq!(half.context_pct, None);

        let good = sniff_kiro("bot · claude-opus-5 · high · ◔ 12%\n(feat/x)\n", "bot");
        assert_eq!(good.model.as_deref(), Some("claude-opus-5"));

        half.backfill(&good);
        assert_eq!(half.model.as_deref(), Some("claude-opus-5"), "gap filled");
        assert_eq!(half.context_pct, Some(12));
        assert_eq!(half.effort.as_deref(), Some("high"));
        assert_eq!(half.branch.as_deref(), Some("feat/x"), "its own value, unchanged");
    }

    #[test]
    fn a_fresh_value_always_wins_over_a_remembered_one() {
        // Backfill must never overwrite: a `/model` swap has to show up at once.
        let mut fresh = sniff_kiro("bot · gpt-5.1 · ● 90%\n", "bot");
        let old = sniff_kiro("bot · claude-opus-5 · ○ 3%\n(main)\n", "bot");
        fresh.backfill(&old);
        assert_eq!(fresh.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(fresh.context_pct, Some(90));
        // The field it never had is the only one that comes from memory.
        assert_eq!(fresh.branch.as_deref(), Some("main"));
    }

    #[test]
    fn backfilling_from_nothing_changes_nothing() {
        let mut v = sniff_kiro("bot · gpt-5.1 · ● 90%\n", "bot");
        let before = v.clone();
        v.backfill(&Vitals::default());
        assert_eq!(v, before);
    }

    /// The flicker itself, end to end: a good capture, then one that caught the
    /// pane between paints. The second must still report what we know.
    #[test]
    fn a_missed_capture_keeps_the_last_reading() {
        let session = format!("vitals-test-{}", std::process::id());
        let good = "bot · claude-opus-5 · ◔ 12%\n/w/x ·\n(main)\n";
        let first = sniff_remembered(&session, 7, good, "bot", "kiro");
        assert_eq!(first.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(first.context_pct, Some(12));

        // A capture with nothing in it — a repaint, a tool's output, a panel.
        let blind = sniff_remembered(&session, 7, "\n\n$ ls\n", "bot", "kiro");
        assert_eq!(blind.model.as_deref(), Some("claude-opus-5"), "remembered, not blank");
        assert_eq!(blind.context_pct, Some(12));
        assert_eq!(blind.branch.as_deref(), Some("main"));

        // A fresh number replaces the remembered one immediately.
        let moved = sniff_remembered(&session, 7, "bot · claude-opus-5 · ◑ 44%\n", "bot", "kiro");
        assert_eq!(moved.context_pct, Some(44));

        // Another window's reading is its own.
        let other = sniff_remembered(&session, 9, "\n", "bot", "kiro");
        assert!(other.is_empty(), "window 9 was never read");

        // A window that goes away is forgotten, so a new agent in the same index
        // cannot inherit its numbers.
        retain_windows(&session, &[9]);
        let after = sniff_remembered(&session, 7, "\n", "bot", "kiro");
        assert!(after.is_empty());
    }
}
