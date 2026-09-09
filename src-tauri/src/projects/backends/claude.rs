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
