//! The claude backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// Claude's own warning text names its effort levels: "Valid values: low,
/// medium, high, xhigh, max" (claude 2.1.239, measured 2026-08-22).
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["low", "medium", "high", "xhigh", "max"]
}

/// Claude takes model aliases nobody can enumerate — no authoritative list,
/// so no validation (soft degradation, models.rs module doc).
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    None
}

use crate::projects::vitals::{context_pct, Vitals, EFFORTS};

/// Format Claude Code's official statusLine JSON into one compact, stable row.
/// This is invoked by the local `tmm claude-statusline` command configured in
/// Claude settings. The `[CC]` anchor is intentionally unique: `sniff_claude`
/// can read the pane without guessing from ordinary conversation text.
pub fn claude_status_line(input: &str) -> Option<String> {
    let data: serde_json::Value = serde_json::from_str(input).ok()?;
    let model = data
        .pointer("/model/display_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| data.pointer("/model/id").and_then(serde_json::Value::as_str))?
        .trim();
    let model = if model.ends_with("context)") {
        model.rfind(" (").map(|i| &model[..i]).unwrap_or(model)
    } else {
        model
    };
    let context = data.get("context_window").and_then(serde_json::Value::as_object);
    let pct = context
        .and_then(|c| c.get("used_percentage"))
        .and_then(serde_json::Value::as_f64)
        .map(|pct| pct.round().clamp(0.0, 100.0) as u8);
    let used = context
        .and_then(|c| c.get("total_input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let size = context
        .and_then(|c| c.get("context_window_size"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let mut parts = vec!["[CC]".to_string(), model.to_string()];
    if let Some(pct) = pct {
        // Percentage comes BEFORE counts so a narrow pane still preserves the
        // field the card needs; Claude may truncate the right side of the row.
        parts.push(format!("{pct}% ctx"));
        if size > 0 {
            parts.push(format!("{}/{}", short_tokens(used), short_tokens(size)));
        }
    }
    if let Some(effort) = data
        .pointer("/effort/level")
        .and_then(serde_json::Value::as_str)
        .filter(|e| EFFORTS.contains(e))
    {
        parts.push(format!("effort {effort}"));
    }
    Some(parts.join(" · "))
}

fn short_tokens(tokens: u64) -> String {
    fn scaled(tokens: u64, unit: u64, suffix: char) -> String {
        if tokens % unit == 0 {
            format!("{}{suffix}", tokens / unit)
        } else {
            let value = tokens as f64 / unit as f64;
            format!("{value:.1}{suffix}")
        }
    }
    if tokens >= 1_000_000 {
        scaled(tokens, 1_000_000, 'M')
    } else if tokens >= 1_000 {
        scaled(tokens, 1_000, 'K')
    } else {
        tokens.to_string()
    }
}

/// Read the canonical row produced by `tmm claude-statusline`.
///
/// Claude's built-in footer is not a stable machine format; statusLine is its
/// official extension point and hands us exact model/context data. Parsing only
/// our `[CC]` row makes ordinary output (including pasted examples) inert.
pub fn sniff_claude(pane: &str) -> Vitals {
    let mut v = Vitals::default();
    for line in pane.lines().rev().take(24) {
        let segs: Vec<&str> = line
            .trim()
            .split('·')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if segs.first() != Some(&"[CC]") {
            continue;
        }
        if let Some(model) = segs.get(1).filter(|s| !s.is_empty()) {
            v.model = Some((*model).to_string());
        }
        for seg in &segs[2..] {
            if v.context_pct.is_none() {
                v.context_pct = context_pct(seg);
            }
            if v.effort.is_none() {
                v.effort = seg
                    .strip_prefix("effort ")
                    .filter(|e| EFFORTS.contains(e))
                    .map(str::to_string);
            }
        }
        v.effort_definitive = v.context_pct.is_some();
        break;
    }
    v
}

