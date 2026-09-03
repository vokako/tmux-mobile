//! Which model ids a backend actually accepts.
//!
//! Reason this exists: a registry def's `model` used to be free text that was
//! pasted straight onto the launch line, and kiro-cli's TUI treats an unknown
//! `--model` as a warning it prints above the splash screen before starting on
//! the DEFAULT model. So `claude-sonnet-4-5` (dashes, one character off the
//! real `claude-sonnet-4.5`) produced an agent that ran happily on the wrong
//! model with nothing in the UI to say so — the owner's report, 2026-08-19.
//!
//! The list is asked of the CLI itself (`kiro-cli chat --list-models`), never
//! hardcoded: model ids come and go weekly, and a stale table in this repo
//! would reject a model the backend has just shipped. Everything degrades
//! soft — no CLI, no auth, unparsable output all mean "we cannot know", and an
//! unknown-to-us model is then accepted rather than blocking a save.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a fetched list is reused. `--list-models` is a network round trip
/// (~3s measured), which is fine once per app run and much too slow per save.
const TTL: Duration = Duration::from_secs(600);

type Cache = Mutex<Vec<(String, Instant, Vec<String>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

/// The model ids `backend` accepts, or `None` when we cannot find out.
///
/// The cache lock is held for the lookup and for the insert, never across
/// `fetch`: that is a ~3 s CLI round trip, and every registry save validates
/// through here, so one hung `--list-models` used to serialise every save
/// behind it. Two concurrent misses may both fetch — a rare 3 s, not a bug.
pub fn list(backend: &str) -> Option<Vec<String>> {
    if let Ok(c) = cache().lock() {
        if let Some((_, at, models)) = c.iter().find(|(b, ..)| b == backend) {
            if at.elapsed() < TTL {
                return Some(models.clone());
            }
        }
    }
    let fetched = fetch(backend)?;
    if let Ok(mut c) = cache().lock() {
        c.retain(|(b, ..)| b != backend);
        c.push((backend.to_string(), Instant::now(), fetched.clone()));
    }
    Some(fetched)
}

/// Reject a model the backend would silently ignore. An empty model means
/// "backend default" and is always fine.
/// The reasoning-effort levels each backend's CLI accepts — a FIXED enum per
/// backend, measured (2026-08-22), never guessed:
/// * kiro:   `kiro-cli chat --effort` — "e.g. low, medium, high, xhigh, max"
/// * claude: its own warning text names them: "Valid values: low, medium,
///           high, xhigh, max" (claude 2.1.239, measured)
/// * grok:   `/effort` doc: low|medium|high|xhigh (grok 1.0.5)
/// * codex:  `model_reasoning_effort` (ReasoningEffort enum): minimal..xhigh
pub fn effort_values(backend: &str) -> &'static [&'static str] {
    match backend {
        "kiro" | "claude" => &["low", "medium", "high", "xhigh", "max"],
        "grok" => &["low", "medium", "high", "xhigh"],
        "codex" => &["minimal", "low", "medium", "high", "xhigh"],
        _ => &[],
    }
}

/// Effort validation mirrors model validation: empty = the backend default and
/// always passes; a non-empty value must be one the CLI accepts, because a
/// wrong one is a warning above the splash and a silent fallback (claude), or
/// a config error nobody reads (codex).
pub fn validate_effort(backend: &str, effort: &str) -> Result<(), String> {
    let effort = effort.trim();
    if effort.is_empty() {
        return Ok(());
    }
    let known = effort_values(backend);
    if known.contains(&effort) {
        return Ok(());
    }
    Err(format!(
        "'{effort}' is not a {backend} effort level. Available: {}",
        known.join(", ")
    ))
}

pub fn validate(backend: &str, model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(());
    }
    let Some(known) = list(backend) else { return Ok(()) };
    if known.is_empty() || known.iter().any(|m| m == model) {
        return Ok(());
    }
    Err(format!(
        "'{model}' is not a {backend} model. Available: {}",
        known.join(", ")
    ))
}

/// Ask the backend's own CLI. Kiro and grok can enumerate their models; claude
/// and codex take aliases we have no authoritative list for, so they return
/// `None` (= no validation) instead of a guess.
fn fetch(backend: &str) -> Option<Vec<String>> {
    match backend {
        "kiro" => {
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
        // `grok models` (1.0.5) prints a plain list:
        //   Available models:
        //     - grok-4.6
        //     * bedrock-grok46 (default)
        // The `*` marks the default; a custom model may carry a "(default)"
        // or description suffix — the id is the first token after the bullet.
        "grok" => {
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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything about this must degrade soft: a machine with no kiro-cli, an
    /// expired login, or a future output format must not make saving an agent
    /// impossible.
    #[test]
    fn an_unknowable_list_validates_everything() {
        assert!(list("claude").is_none(), "no authoritative list for claude");
        assert!(list("codex").is_none());
        assert!(validate("claude", "sonnet").is_ok());
        assert!(validate("codex", "gpt-5.6-terra").is_ok());
        // An empty model is "backend default" for every backend.
        assert!(validate("kiro", "").is_ok());
        assert!(validate("kiro", "   ").is_ok());
    }

    /// When the CLI IS there, the exact typo class this module exists for must
    /// be rejected — and the real id it was a typo OF must be accepted.
    #[test]
    fn a_one_character_typo_is_rejected_when_the_cli_can_tell_us() {
        let Some(models) = list("kiro") else {
            eprintln!("kiro-cli unavailable — validation degrades to accept-all, covered above");
            return;
        };
        assert!(models.iter().any(|m| m == "auto"), "got {models:?}");
        let real = models
            .iter()
            .find(|m| m.contains("sonnet"))
            .expect("a sonnet model exists")
            .clone();
        assert!(validate("kiro", &real).is_ok());
        let typo = real.replace('.', "-");
        if typo != real {
            let err = validate("kiro", &typo).expect_err("dashed version must not pass");
            assert!(err.contains(&real), "the error must name the real ids: {err}");
        }
        // The second call comes from the cache, so a save never pays for a
        // second round trip.
        let before = Instant::now();
        assert!(validate("kiro", &real).is_ok());
        assert!(before.elapsed() < Duration::from_millis(200), "cached");
    }
}
