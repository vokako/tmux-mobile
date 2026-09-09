//! The omp backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// omp 18.0.6 `--thinking` (its own --help): "off, minimal, low, medium,
/// high, xhigh, max, auto".
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["off", "minimal", "low", "medium", "high", "xhigh", "max", "auto"]
}

/// omp models ride its own catalog files; no authoritative list to ask.
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    None
}
