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
