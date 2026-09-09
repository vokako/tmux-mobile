//! The grok backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// grok `/effort` doc: low|medium|high|xhigh (grok 1.0.5, measured).
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["low", "medium", "high", "xhigh"]
}

/// `grok models` (1.0.5) prints a plain list:
///   Available models:
///     - grok-4.6
///     * bedrock-grok46 (default)
/// The `*` marks the default; a custom model may carry a "(default)" or
/// description suffix — the id is the first token after the bullet.
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    let out = std::process::Command::new("grok").arg("models").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let models: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("- ").or_else(|| l.strip_prefix("* "))?;
            rest.split_whitespace().next().map(str::to_string)
        })
        .collect();
    (!models.is_empty()).then_some(models)
}

use crate::projects::vitals::Vitals;

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
pub(crate) fn grok_footer_model(line: &str) -> Option<String> {
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
pub(crate) fn grok_context_ratio(line: &str) -> Option<u8> {
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
pub(crate) fn grok_tokens(s: &str) -> Option<f64> {
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

