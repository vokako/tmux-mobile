//! The codex backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// codex `model_reasoning_effort` (its ReasoningEffort enum): minimal..xhigh
/// (measured 2026-08-22).
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["minimal", "low", "medium", "high", "xhigh"]
}

/// codex takes aliases with no authoritative list — no validation.
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    None
}

use crate::projects::vitals::{looks_like_model, Vitals};

/// codex's status furniture, measured on codex-cli 0.148.0 (2026-08-22,
/// re-measured 2026-09-03).
///
/// The persistent footer under the composer is the configurable
/// `tui.status_line`: `·`-joined items in the order the config lists them.
/// The inherited `~/.codex/config.toml` sets `["model", "context-used",
/// "current-dir"]`, which paints
/// `openai.gpt-5.6-sol · Context 5% used · /tmp/x` — and, at 44 columns,
/// `openai.gpt-5.6-sol · Context 5% used · /t…` (codex TRUNCATES the line with
/// `…`, it never wraps). The default footer (no `[tui]` section) is
/// `<model> [<effort>] · <cwd>`, and `context-remaining` spells
/// `Context 99% left`, so every item is found BY SHAPE, not by position:
/// a `Context NN% used|left` segment is the context, a segment starting
/// `/`/`~` is the cwd, and a 1–2-token segment whose first token carries a
/// digit is `<model> [<effort>]`. A line counts as the footer only when it
/// has a model segment AND (a cwd or a context segment) — prose with one
/// mid-sentence `·` has neither anchor.
///
/// Context is also spelled `NN% context left` (codex's right-footer format
/// string; `100% context left` is its zero-use rendering) and, in the
/// `/status` card, `NN% left (21.5K used / 258K)` — both say LEFT where kiro
/// says USED, so those readings are `100 - NN`.
pub fn sniff_codex(pane: &str) -> Vitals {
    let mut v = Vitals::default();
    for line in pane.lines().rev() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(f) = codex_footer(line) {
            if v.model.is_none() {
                v.model = Some(f.model);
                v.effort = f.effort;
            }
            if v.context_pct.is_none() {
                v.context_pct = f.context_pct;
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

pub(crate) struct CodexFooter {
    pub(crate) model: String,
    pub(crate) effort: Option<String>,
    pub(crate) context_pct: Option<u8>,
}

/// One `tui.status_line` paint → its readings, or `None` when the line does
/// not have the footer's anchors (see `sniff_codex`).
pub(crate) fn codex_footer(line: &str) -> Option<CodexFooter> {
    let mut model: Option<(String, Option<String>)> = None;
    let mut context_pct = None;
    let mut cwd = false;
    for seg in line.trim().split('\u{b7}').map(str::trim) {
        if seg.starts_with('/') || seg.starts_with('~') {
            cwd = true;
        } else if let Some(pct) = codex_context_item(seg) {
            context_pct = Some(pct);
        } else if model.is_none() {
            model = codex_model_item(seg);
        }
    }
    let (model, effort) = model?;
    (cwd || context_pct.is_some()).then_some(CodexFooter { model, effort, context_pct })
}

/// `<model> [<effort>]` — the model token must contain a digit
/// (`xai.grok-4.6`, `gpt-5.2-codex` — every model id does) and the effort,
/// when present, is one plain lowercase word; a `…`-truncated word
/// (`defa…`) is not an effort.
pub(crate) fn codex_model_item(seg: &str) -> Option<(String, Option<String>)> {
    let mut toks = seg.split_whitespace();
    let model = toks.next()?;
    if !looks_like_model(model) || !model.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let effort = toks.next();
    if toks.next().is_some() {
        return None;
    }
    let effort = match effort {
        None => None,
        Some(e) if !e.is_empty() && e.chars().all(|c| c.is_ascii_lowercase()) => Some(e.to_string()),
        // A truncated word (`defa…`) is not an effort — the model still is.
        Some(_) => None,
    };
    Some((model.to_string(), effort))
}

/// `Context 5% used` (`context-used`) → 5; `Context 99% left`
/// (`context-remaining`) → 1. The literal `Context` word is the anchor: a
/// bare `5%` is never accepted.
pub(crate) fn codex_context_item(seg: &str) -> Option<u8> {
    let rest = seg.strip_prefix("Context")?.trim_start();
    let (num, tail) = rest.split_once('%')?;
    let n = num.trim().parse::<u16>().ok().filter(|n| *n <= 100)?;
    match tail.trim() {
        "used" => Some(n as u8),
        "left" => Some((100 - n) as u8),
        _ => None,
    }
}

/// `NN% context left` (right footer) or `NN% left (… used / …)` (/status
/// card) → share of the context USED (`100 - NN`), matching kiro's own
/// wording for `Vitals::context_pct`. The trailing words are the anchor: a
/// bare `NN%` is never accepted (same rule as kiro's pie-glyph requirement).
pub(crate) fn codex_context_left(line: &str) -> Option<u8> {
    let s = line.trim().trim_matches('\u{2502}').trim();
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

