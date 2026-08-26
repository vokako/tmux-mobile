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
    /// True when the FULL status line was seen this capture (kiro: the anchored
    /// line with its context segment), so an absent effort is a VERDICT — kiro
    /// omits the effort segment entirely when it is the backend default (owner,
    /// 2026-08-26: "effort 这个参数不是百分之百都会显示的"), and without this
    /// flag a once-misread effort could never clear: backfill re-inserted it
    /// with a fresh timestamp on every poll, a permanent ghost.
    #[serde(skip)]
    pub effort_definitive: bool,
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
/// far better than a card that blinks empty. An hour, not five minutes: readings
/// are now REFRESHED by events (hooks, deliveries — `sniff_window_soon`), so age
/// means "nothing has happened", and showing the last known state through a long
/// quiet spell is exactly what the owner asked of the record (2026-08-25:
/// "服务端记录这个状态，客户端随时能获取到"). Past this the reading is dropped
/// rather than aged.
const VITALS_TTL_SECS: u64 = 3600;

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
        // An effort verdict stands: when the full status line was read and
        // carried no effort segment, the effort IS "backend default" — filling
        // it from memory would resurrect a stale (or once-misread) value that
        // kiro will never repaint to contradict.
        if self.effort.is_none() && !self.effort_definitive {
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
        "codex" => sniff_codex(pane),
        // claude has NO sniffer yet: the CLI is not installed on this machine,
        // so its status furniture cannot be measured, and reading its pane
        // with kiro's grammar (the old `_` fallback) risks a confident wrong
        // reading — e.g. any `on branch …` in ordinary output becoming the
        // card's branch. No reading beats a wrong one (owner, 2026-08-22 对齐).
        "claude" => Vitals::default(),
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

/// When each (session, window) was last SCHEDULED for an event sniff, for the
/// throttle below. Separate from the reading cache: a throttled request is not
/// a reading.
fn sniff_times() -> &'static std::sync::Mutex<std::collections::HashMap<(String, usize), u64>> {
    static TIMES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(String, usize), u64>>,
    > = std::sync::OnceLock::new();
    TIMES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Sniff one managed window shortly AFTER an event that makes its pane fresh —
/// a hook fired (a turn opened or closed, a tool ran) or a chat line was just
/// typed into it. `hub_agents` only sniffed when the client happened to poll,
/// so a fresh page often showed nothing until a later poll caught the status
/// line (owner, 2026-08-25: "经常看到没有信息，过了一会儿才出来"); sniffing at
/// the moments the CLI repaints its footer keeps the memory warm, and the poll
/// answers from the memory. Runs on a throwaway thread after ~1.2 s (the TUI
/// needs a beat to repaint; telemetry may never block what it observes), and
/// is throttled per window so a burst of tool hooks costs one capture, not
/// thirty. Fail-soft everywhere: an unresolvable window is a no-op.
pub fn sniff_window_soon(session: &str, window: usize) {
    if cfg!(test) {
        // Tests must not reach for a real tmux or spawn sniffer threads.
        return;
    }
    let now = now_secs();
    {
        let mut times = sniff_times().lock().unwrap();
        let key = (session.to_string(), window);
        if let Some(at) = times.get(&key) {
            if now.saturating_sub(*at) < 3 {
                return;
            }
        }
        times.insert(key, now);
        times.retain(|_, at| now.saturating_sub(*at) < 600);
    }
    let session = session.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        sniff_window_now(&session, window);
    });
}

/// The synchronous half: resolve the window, apply the same managed-only gate
/// as `hub_agents` (we only know OUR agents' status-line shapes), capture, and
/// remember. The reading lands in the same cache `hub_agents` reads.
fn sniff_window_now(session: &str, window: usize) {
    let ws = crate::projects::project_for_session(session)
        .ok()
        .flatten()
        .map(|p| p.path);
    let Ok(panes) = crate::tmux::list_panes(session) else { return };
    let Some(p) = panes.iter().find(|p| p.window == window && p.active) else { return };
    if !crate::projects::is_managed_in(ws.as_deref(), &p.window_name) {
        return;
    }
    let hay = format!("{} {} {}", p.current_command, p.pane_title, p.window_name);
    let Some(agent) = crate::projects::agents::detect_managed(ws.as_deref(), &p.window_name, &hay)
    else {
        return;
    };
    let Ok(text) = crate::tmux::capture_pane_plain(&format!("{session}:{window}"), Some(0)) else {
        return;
    };
    sniff_remembered(session, window, &text, &p.window_name, agent.backend);
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
        // A WIDE pane keeps `location · branch` right-aligned on the SAME line
        // as the left segments, joined not by `·` but by the padding run of
        // spaces — so the last left segment arrives glued to the location
        // (`◔ 5%       /local/home/cfu/temp`) and its parser refuses it
        // (owner, 2026-08-26: context missing on the chat project). A run of
        // two or more spaces is that gap and never occurs INSIDE a segment
        // (`◔ 5%` is single-spaced), so it is a segment boundary too. Narrow
        // panes wrap the right side onto its own line and are unaffected.
        let segs: Vec<&str> = line
            .split('·')
            .flat_map(|s| s.split("  "))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        // The activity line ("Kiro is working · Type to queue · …") shares the
        // dot separator with the status line, and its words are not segments.
        if segs.iter().any(|s| s.starts_with("Kiro is ") || *s == "Type to queue") {
            continue;
        }

        // Is this the status line proper? kiro's left side starts with the
        // agent's own name, so seg 0 == the anchor. Effort is ONLY read here,
        // and only by the line's own PATTERN — `… · model · effort · ◔ N%`,
        // i.e. the segment immediately BEFORE the context segment (owner,
        // 2026-08-26: "gpt-5.6-sol · medium · ◔ 5% 所以 effort 显示 你要按照
        // 这样的模式去匹配 不要直接全文匹配"): unlike context (pie glyph) and
        // branch (parentheses) it is a bare word with no shape of its own, and
        // reading `high`/`max`/`medium` wherever they sat turned ordinary
        // output (a table cell, a priority column) into a confident reading.
        let anchored = segs.first().is_some_and(|s| *s == agent);
        if anchored && v.effort.is_none() && !v.effort_definitive {
            if let Some(ci) = segs.iter().position(|s| context_pct(s).is_some()) {
                let word = segs[ci.saturating_sub(1)].to_ascii_lowercase();
                if ci > 0 && EFFORTS.contains(&word.as_str()) {
                    v.effort = Some(word);
                }
            }
        }

        for (i, seg) in segs.iter().enumerate() {
            if v.context_pct.is_none() {
                if let Some(pct) = context_pct(seg) {
                    v.context_pct = Some(pct);
                    continue;
                }
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
            if v.model.is_none() && i == 0 && anchored {
                let next = segs
                    .iter()
                    .skip(1)
                    .find(|s| !s.eq_ignore_ascii_case("Autonomous"));
                if let Some(m) = next.filter(|m| looks_like_model(m)) {
                    v.model = Some((*m).to_string());
                }
            }
        }
        // The anchored line carrying its context segment is the FULL left side
        // (`agent · [autonomous] · model · [effort] · context`): whatever it
        // says about effort — including "nothing" — is the verdict. kiro omits
        // the segment when the effort is the backend default, so absence here
        // is a reading, not a miss, and backfill must not overwrite it.
        if anchored && v.context_pct.is_some() {
            v.effort_definitive = true;
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

/// codex's status furniture, measured on codex-cli 0.148.0 (2026-08-22).
///
/// The persistent footer is a `·`-joined line whose FIRST segment is
/// `<model> [<effort>]` and whose SECOND is the cwd:
/// `xai.grok-4.6 default · /local/home/cfu/work/projects/tmux-mobile`.
/// Context is spelled `NN% context left` (the binary's own footer format
/// string; "100% context left" is its zero-use rendering) or, in the /status
/// card, `NN% left (21.5K used / 258K)` — both say LEFT where kiro says USED,
/// so the reading is `100 - NN`.
pub fn sniff_codex(pane: &str) -> Vitals {
    let mut v = Vitals::default();
    for line in pane.lines().rev() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if v.model.is_none() {
            if let Some((m, e)) = codex_footer_model(line) {
                v.model = Some(m);
                v.effort = e;
            }
        }
        if v.context_pct.is_none() {
            if let Some(pct) = codex_context_left(line) {
                v.context_pct = Some(pct);
            }
        }
        if v.model.is_some() && v.context_pct.is_some() {
            break;
        }
    }
    v
}

/// `<model> [<effort>] · <cwd> [· …]` → (model, effort). The anchors: the
/// second `·`-segment must be an absolute path (`/` or `~` — the footer's cwd,
/// measured), and the model token must contain a digit (`xai.grok-4.6`,
/// `gpt-5.2-codex` — every model id does), which keeps prose with a
/// mid-sentence `·` from becoming a reading.
fn codex_footer_model(line: &str) -> Option<(String, Option<String>)> {
    let mut segs = line.trim().split('·').map(str::trim);
    let first = segs.next()?;
    let second = segs.next()?;
    if !(second.starts_with('/') || second.starts_with('~')) {
        return None;
    }
    let mut toks = first.split_whitespace();
    let model = toks.next()?;
    if !model.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let effort = toks.next().map(str::to_string);
    // More than two tokens is not the footer's shape.
    if toks.next().is_some() {
        return None;
    }
    Some((model.to_string(), effort))
}

/// `NN% context left` (footer) or `NN% left (… used / …)` (/status card) →
/// share of the context USED (`100 - NN`), matching kiro's own wording for
/// `Vitals::context_pct`. The trailing words are the anchor: a bare `NN%` is
/// never accepted (same rule as kiro's pie-glyph requirement).
fn codex_context_left(line: &str) -> Option<u8> {
    let s = line.trim().trim_matches('│').trim();
    let idx = s.find("% context left").or_else(|| {
        let i = s.find("% left (")?;
        // The /status shape must really be the context card, not prose.
        s.contains("used /").then_some(i)
    })?;
    let digits: String = s[..idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let left = digits.parse::<u16>().ok().filter(|n| *n <= 100)?;
    Some((100 - left) as u8)
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

    /// A real capture of a WIDE managed kiro pane (chat:2, 2026-08-26). With
    /// room to spare, kiro right-aligns `location · branch` on the SAME line as
    /// the left segments, joined by a padding run of spaces — so the context
    /// segment arrived glued to the cwd and the percent was never read (owner:
    /// "上下文长度也没嗅探出来").
    #[test]
    fn a_wide_pane_glues_the_right_side_onto_the_status_line() {
        let real = "─────────────────────────────────────────────\n\
            chat · gpt-5.6-sol · medium · ◔ 5%       /local/home/cfu/temp\n\
            \x20ask a question or describe a task ↵\n\
            \x20                                 /copy to clipboard\n";
        let v = sniff_kiro(real, "chat");
        assert_eq!(v.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(v.effort.as_deref(), Some("medium"));
        assert_eq!(v.context_pct, Some(5), "the glued cwd must not eat the percent");
        assert_eq!(v.branch, None, "no branch painted in a non-repo cwd");
        // And with a branch, the right side is `location · branch`:
        let with_branch =
            "bot · claude-opus-5 · ◕ 71%       /local/home/cfu/work · (feat/x)\n";
        let b = sniff_kiro(with_branch, "bot");
        assert_eq!(b.context_pct, Some(71));
        assert_eq!(b.branch.as_deref(), Some("feat/x"));
        assert_eq!(b.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn the_activity_line_is_not_a_status_line() {
        // It uses the same `·` separator, so it has to be skipped by name or
        // "working" and "queue" become segments to interpret.
        let v = sniff_kiro(" Kiro is working · Type to queue · Ctrl+S to steer\n", "builder-2");
        assert_eq!(v, Vitals::default());
    }

    #[test]
    fn effort_is_matched_by_the_status_line_pattern() {
        // `… · model · effort · ◔ N%` — the effort is the segment immediately
        // BEFORE the context segment on the anchored line (owner, 2026-08-26:
        // "你要按照这样的模式去匹配 不要直接全文匹配").
        let v = sniff_kiro("worker · gpt-5.1 · high · ◑ 42%\n", "worker");
        assert_eq!(v.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(v.effort.as_deref(), Some("high"));
        assert_eq!(v.context_pct, Some(42));
        // Position alone would have read `high` as the model.
        let no_effort = sniff_kiro("worker · gpt-5.1 · ◑ 42%\n", "worker");
        assert_eq!(no_effort.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(no_effort.effort, None);
        // A bare effort word ANYWHERE else is not a reading — not in ordinary
        // output, and not even elsewhere on the anchored line (a tangent or
        // goal segment could be named `high`).
        for text in [
            "priority  high  assigned to worker\n",
            "medium\n",
            "risk: low · impact: high\n",
            "worker · gpt-5.1 · ◑ 42% · high\n",
        ] {
            assert_eq!(sniff_kiro(text, "worker").effort, None, "{text:?}");
        }
    }

    #[test]
    fn an_absent_effort_on_the_full_status_line_is_a_verdict() {
        // kiro omits the effort segment when it is the backend default, so the
        // anchored line WITH its context segment and WITHOUT an effort says
        // "no effort" — and backfill must not resurrect a remembered one, or a
        // stale/misread value becomes a permanent ghost (re-inserted with a
        // fresh timestamp on every poll).
        let mut fresh = sniff_kiro("worker · gpt-5.1 · ◑ 42%\n", "worker");
        assert!(fresh.effort_definitive);
        let prev = Vitals { effort: Some("high".into()), ..Default::default() };
        fresh.backfill(&prev);
        assert_eq!(fresh.effort, None, "verdict beats memory");
        // Without the verdict (status line not on screen), memory still fills
        // the gap — that is the whole point of remembering.
        let mut miss = sniff_kiro("$ ls\n", "worker");
        assert!(!miss.effort_definitive);
        miss.backfill(&prev);
        assert_eq!(miss.effort.as_deref(), Some("high"));
        // And an OLDER status line in scrollback cannot re-fill it either: the
        // newest paint's verdict wins over anything above it.
        let two = "worker · gpt-5.1 · high · ◑ 40%\nworker · gpt-5.1 · ◑ 42%\n";
        assert_eq!(sniff_kiro(two, "worker").effort, None);
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

    /// codex-cli 0.148.0, real pane capture (2026-08-22): the persistent
    /// footer under the composer.
    #[test]
    fn codex_footer_reads_model_effort_and_nothing_else() {
        let pane = "\u{2022} ok\n\u{203a} Ask Codex to do anything\n  xai.grok-4.6 default \u{b7} /local/home/cfu/work/projects/tmux-mobile\n";
        let v = sniff_codex(pane);
        assert_eq!(v.model.as_deref(), Some("xai.grok-4.6"));
        assert_eq!(v.effort.as_deref(), Some("default"));
        assert_eq!(v.context_pct, None, "no context painted in this capture");
        assert_eq!(v.branch, None);
    }

    /// The context spellings: the binary's own footer format string renders
    /// `100% context left` at zero use, and the /status card (captured live)
    /// says `96% left (21.5K used / 258K)`. Both are LEFT; the vital is USED.
    #[test]
    fn codex_context_left_becomes_used() {
        assert_eq!(codex_context_left("  97% context left"), Some(3));
        assert_eq!(codex_context_left("100% context left"), Some(0));
        assert_eq!(
            codex_context_left("\u{2502}  Context window:       96% left (21.5K used / 258K)  \u{2502}"),
            Some(4)
        );
        // A bare percentage, or "left" prose without the context anchors,
        // is never a reading.
        assert_eq!(codex_context_left("97%"), None);
        assert_eq!(codex_context_left("3 tries left (2 used / x)"), None);
        assert_eq!(codex_context_left("101% context left"), None);
    }

    /// The footer anchors: second segment must be a path, model token must
    /// carry a digit, exactly one optional effort token.
    #[test]
    fn codex_footer_rejects_prose_with_middots() {
        assert_eq!(codex_footer_model("word \u{b7} another word"), None, "no path");
        assert_eq!(codex_footer_model("plainmodel \u{b7} /tmp"), None, "no digit");
        assert_eq!(codex_footer_model("a b c 4 \u{b7} /tmp"), None, "too many tokens");
        assert_eq!(
            codex_footer_model("gpt-5.2-codex medium \u{b7} ~/my-project"),
            Some(("gpt-5.2-codex".into(), Some("medium".into())))
        );
    }

    /// claude gets NO reading until its furniture is measured — kiro's grammar
    /// must not be applied to another CLI's screen.
    #[test]
    fn claude_panes_are_not_read_with_kiro_grammar() {
        let session = format!("vitals-claude-{}", std::process::id());
        let pane = "bot \u{b7} claude-opus-5 \u{b7} \u{25d1} 44%\n(main)\n";
        let v = sniff_remembered(&session, 3, pane, "bot", "claude");
        assert!(v.is_empty(), "claude pane must yield the empty reading, got {v:?}");
    }
}
