//! The kiro backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// Reasoning-effort levels kiro-cli accepts (`kiro-cli chat --effort`,
/// measured 2026-08-22: "e.g. low, medium, high, xhigh, max").
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["low", "medium", "high", "xhigh", "max"]
}

/// `kiro-cli chat --list-models -f json` enumerates real model ids.
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    let out = std::process::Command::new("kiro-cli")
        .args(["chat", "--list-models", "-f", "json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let models: Vec<String> = parsed
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|m| m.get("model_id")?.as_str().map(str::to_string))
        .collect();
    (!models.is_empty()).then_some(models)
}
