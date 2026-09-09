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
