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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::projects::vitals::{branch, context_pct, looks_like_model, Vitals, EFFORTS};

/// Read what the last lines of a pane say about the agent's current state.
///
/// `agent` is normally the first status segment (the managed window name).
/// Resumed legacy conversations may retain the exact built-in `kiro_default`
/// identity, which is accepted as the one narrow fallback. The anchor is not a
/// filter: fields that identify themselves by shape (context and branch) are
/// read even when it never appears, because narrow panes wrap later segments.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

        // Is this the status line proper? Usually kiro's left side starts with
        // the managed window/registry name. A resumed conversation can retain
        // Kiro's exact built-in identity `kiro_default` even though the app
        // window was later named `chat`; that line is still the TUI's own
        // status and therefore the runtime-model authority. Keep the exception
        // exact — arbitrary first segments remain ordinary output.
        // Effort has no identifying glyph, so it is read ONLY on this anchored
        // line and only as the segment immediately before the context segment.
        // Matching bare low/medium/high words elsewhere made ordinary output
        // look like a runtime setting.
        let anchored = segs.first().is_some_and(|s| *s == agent || *s == "kiro_default");
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
                // v2 prints the slug (`claude-sonnet-5`), the v3 engine the
                // DISPLAY name (`Claude Sonnet 5`, board #207) — both are the
                // model, and only here, right after the anchor, is a name
                // with spaces one.
                if let Some(m) = next.filter(|m| looks_like_model(m) || looks_like_model_name(m)) {
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


use serde_json::{json, Value};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::path::Path;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::projects::spawn::{effort_flag, Rendered};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::projects::store::RegAgent;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use super::shared;

/// The hook set for each backend, in ONE place. `render_*` writes it at spawn
/// and `refresh_hooks` rewrites it on every start, so a config on disk can
/// never be older than the app that reads its events. (It was: agents spawned
/// before `userPromptSubmit` existed kept a three-hook config, and since that
/// hook is the only reset of the same-turn dedup flag, their first `tmm send`
/// silently killed the stop-hook auto-post for the rest of the window's life.)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
/// A model DISPLAY name as kiro-cli --v3's status line prints it: words of
/// letters/digits/dots/dashes separated by single spaces, at least one letter,
/// ≤ 48 chars ("Claude Sonnet 5", "GPT 5.1 Codex"). Read only right after the
/// anchor — the position is what makes a name with spaces a model and not a
/// path, a branch, a "Midway: 19h 19m" segment (the colon fails) or a "◔ 4%".
pub(crate) fn looks_like_model_name(s: &str) -> bool {
    let word_ok = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    !s.is_empty()
        && s.len() <= 48
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && !s.contains("  ")
        && s.split(' ').all(word_ok)
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

pub(crate) fn kiro_hooks(notify: &str) -> Value {
    // The 3.0 profile shape (kiro-cli 2.22.1, board #207): an ARRAY of named
    // hooks. The v2 engine accepts it and fires all four (measured,
    // hooks-v2b.log); v3 fires prompt/stop and gates the tool hooks behind
    // its featureFlags.v2Hooks. Writing this shape means the CLI's
    // "agent configs are still in the 2.0 format" upgrade never runs on our
    // homes — no startup modal, no `.bak`, no rewrite churn against refresh.
    let hook = |name: &str, trigger: &str, matcher: Option<&str>| {
        let mut h = json!({
            "name": name,
            "trigger": trigger,
            "action": { "type": "command", "command": notify },
            "timeout": 10,
        });
        if let Some(m) = matcher {
            h["matcher"] = json!(m);
        }
        h
    };
    json!([
        // The notify helper feeds notifications AND telemetry (tool events are
        // recognized by hook_event_name and routed to telemetry only).
        hook("tmm-pre-tool", "preToolUse", Some("*")),
        hook("tmm-post-tool", "postToolUse", Some("*")),
        // Turn start — the ONLY reset of the same-turn dedup flag, and the
        // event that carries the submitted prompt.
        hook("tmm-prompt", "userPromptSubmit", None),
        hook("tmm-stop", "stop", None),
    ])
}

/// The 3.0 profile's permission block, the equivalent of `--trust-all-tools`
/// spelled in the config (the CLI's own upgrade writes exactly this).
pub(crate) fn kiro_permissions() -> Value {
    json!({ "rules": [ { "capability": "all", "effect": "allow" } ] })
}

/// Bring a managed profile to the 3.0 shape without churn: hooks replaced
/// only when they differ (either old shape), `permissions` added only when
/// absent (a block the user or the CLI wrote is theirs). Returns true on
/// change.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn patch_profile(path: &Path, hooks: Value) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Ok(mut root) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(obj) = root.as_object_mut() else { return false };
    let mut changed = false;
    if obj.get("hooks") != Some(&hooks) {
        obj.insert("hooks".into(), hooks);
        changed = true;
    }
    if !obj.contains_key("permissions") {
        obj.insert("permissions".into(), kiro_permissions());
        changed = true;
    }
    changed && std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or(text)).is_ok()
}

/// The CLI settings every managed kiro agent runs with (`<home>/settings/
/// cli.json`, read because the pane launches with `KIRO_HOME=<home>`).
///
/// `chat.defaultInterruptBehavior = "queue"` is an owner decision, 2026-08-20
/// ("所有 Agent 在 kiro 里边发送指令的模式 默认给我设计成 Queue 队列模式吧 不要
/// steer 模式"): a line typed at a BUSY agent waits for the turn to end instead
/// of steering the turn mid-flight — the agent reads it whole, as its own
/// prompt. That is also the contract the delivery pipeline already assumes:
/// `delivery_overdue` pauses the ack clock while a turn is open precisely
/// because kiro "Type to queue"s what we send.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn kiro_cli_settings() -> Vec<(&'static str, Value)> {
    vec![
        ("chat.disableTrustAllConfirmation", json!(true)),
        ("chat.defaultInterruptBehavior", json!("queue")),
        // MCP tool schemas are DEFERRED into a compact list and loaded on
        // demand via kiro's own tool_search (owner, 2026-08-28: "给 kiro 的
        // mcp 工具开启 toolsearch"). Thresholds 0/0 = defer whenever any MCP
        // tools are present, which is the progressive behavior asked for.
        ("toolSearch.enabled", json!(true)),
        ("toolSearch.minPct", json!(0)),
        ("toolSearch.minTokens", json!(0)),
        // kiro-cli 2.22.1 --v3 (board #207): without this the CLI parks an
        // unattended pane on "Your agent configs are still in the 2.0 format.
        // Upgrade?". Our profiles are already 3.0-shaped, so with it set the
        // upgrade is a no-op — no `.bak`, no rewrite.
        ("chat.enableAutoAgentUpgrade", json!(true)),
    ]
}

/// The one per-agent setting: v3 IGNORES the profile's `model` and reads
/// `chat.defaultModel` from the home settings (board #207). The profile keeps
/// `model` for v2; both come from the same definition, so they cannot
/// disagree. An empty model means the backend default: the key is OMITTED,
/// and a stale key is DELETED when the model is cleared — otherwise an old
/// pin outlives the config.
pub(crate) const DEFAULT_MODEL_KEY: &str = "chat.defaultModel";

/// Force the canonical CLI settings into a managed kiro home, leaving any
/// other keys alone. Creates the file when it is missing (pre-settings homes),
/// no-op write when everything already matches — the same contract as
/// `patch_hooks`, because the app owns these configs. Returns true on change.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn ensure_kiro_settings(home: &Path, model: &str) -> bool {
    let dir = home.join("settings");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let path = dir.join("cli.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let obj = root.as_object_mut().expect("filtered to object above");
    let mut changed = !path.is_file();
    for (key, value) in kiro_cli_settings() {
        if obj.get(key) != Some(&value) {
            obj.insert(key.to_string(), value);
            changed = true;
        }
    }
    let model = model.trim();
    if model.is_empty() {
        changed |= obj.remove(DEFAULT_MODEL_KEY).is_some();
    } else if obj.get(DEFAULT_MODEL_KEY).and_then(Value::as_str) != Some(model) {
        obj.insert(DEFAULT_MODEL_KEY.into(), json!(model));
        changed = true;
    }
    changed && std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap()).is_ok()
}

/// The model a managed profile pins (its `model` key), or "" — what refresh
/// mirrors into `chat.defaultModel` so the v3 engine reads the same id v2 does.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn profile_model(config: &Path) -> String {
    std::fs::read_to_string(config)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// ` --agent-engine v3` when the app-wide engine door is open, else nothing
/// — the v2 launch line is byte-for-byte what it was, so flipping the door
/// back leaves no trace (board #207). The door is `kiro_engine` in
/// config.toml (`KIRO_ENGINE` overrides), read at spawn and at refresh: an
/// agent takes the new engine at its next restart, so one scratch agent can be
/// flipped alone before the owner restarts the real ones.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn engine_flag() -> String {
    match crate::config::kiro_engine().as_str() {
        "v3" => " --agent-engine v3".into(),
        _ => String::new(),
    }
}

/// Bring a launch recipe's engine segment in line with the door: add
/// ` --agent-engine v3` after `--trust-all-tools` when the door is open and
/// the line lacks it, drop any ` --agent-engine <x>` when it is closed. Exact
/// token surgery on the recorded line, like `migrate_launch_model`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn reconcile_recipe_engine(home: &Path) -> bool {
    let recipe_path = home.join("launch.json");
    let Ok(text) = std::fs::read_to_string(&recipe_path) else { return false };
    let Ok(mut recipe) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(cmd) = recipe.get("cmd").and_then(Value::as_str).map(str::to_string) else { return false };
    let Some(next) = reconcile_engine_in(&cmd, &engine_flag()) else { return false };
    recipe["cmd"] = json!(next);
    std::fs::write(&recipe_path, serde_json::to_string_pretty(&recipe).unwrap_or(text)).is_ok()
}

/// The pure half: `Some(new line)` when the engine segment has to change.
pub(crate) fn reconcile_engine_in(cmd: &str, flag: &str) -> Option<String> {
    let mut tokens: Vec<&str> = cmd.split_whitespace().collect();
    let has = tokens.iter().position(|t| *t == "--agent-engine");
    let want: Vec<&str> = flag.split_whitespace().collect();
    match (has, want.is_empty()) {
        (Some(at), true) => {
            tokens.drain(at..(at + 2).min(tokens.len()));
            Some(tokens.join(" "))
        }
        (Some(at), false) => {
            if tokens.get(at + 1) == want.get(1) {
                return None;
            }
            if at + 1 < tokens.len() {
                tokens[at + 1] = want[1];
            } else {
                tokens.push(want[1]);
            }
            Some(tokens.join(" "))
        }
        (None, true) => None,
        (None, false) => Some(format!("{cmd}{flag}")),
    }
}

/// The v3 engine's Stop payload carries no reply text; the final response of
/// the turn is the last `assistant` entry of the GLOBAL session file
/// `~/.kiro/sessions/<cwd-hash>/sess_<id>/messages.jsonl` (board #207; the
/// hook's `session_id` names the directory). Each line is
/// `{id, timestamp, payload: {type, content, …}}`; a turn is `turn_start` …
/// `turn_end` and may hold several `assistant` entries (text between tool
/// calls, then the final text) — the reply is the LAST assistant entry after
/// the LAST `turn_start`. Bad lines are skipped, never fatal.
pub(crate) fn kiro_reply_from_session(jsonl: &str) -> Option<String> {
    let mut reply: Option<String> = None;
    for line in jsonl.lines() {
        let Ok(row) = serde_json::from_str::<Value>(line) else { continue };
        let payload = row.get("payload").unwrap_or(&row);
        match payload.get("type").and_then(Value::as_str) {
            Some("turn_start") => reply = None,
            Some("assistant") => {
                if let Some(text) = payload.get("content").and_then(Value::as_str) {
                    let text = text.trim();
                    if !text.is_empty() {
                        reply = Some(text.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    reply
}

/// Locate and read the session's reply — `sessions_root` is `~/.kiro/sessions`
/// in production and a temp dir in tests. `None` when nothing is found.
pub(crate) fn session_reply(sessions_root: &Path, session_id: &str) -> Option<String> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.starts_with('.') {
        return None;
    }
    let dirs = std::fs::read_dir(sessions_root).ok()?;
    for entry in dirs.flatten() {
        let file = entry.path().join(session_id).join("messages.jsonl");
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Some(reply) = kiro_reply_from_session(&text) {
                return Some(reply);
            }
        }
    }
    None
}

/// The v3 fallback wired into the notify helper: the kiro Stop payload has no
/// text, the session file has.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn reply_fallback(payload: &serde_json::Map<String, Value>) -> Option<String> {
    let id = crate::agent_notifications::string_field(payload, &["session_id", "sessionId"])?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    session_reply(&home.join(".kiro").join("sessions"), &id)
}

/// Move a `--model <id>` an older build put on the launch line into the agent
/// config, where kiro actually honours it, and drop it from the recipe so the
/// two cannot disagree. Exact information, so nothing is guessed: the id is
/// read off the line that was really used.
///
/// Why it matters beyond tidiness: `refresh_hooks` also BACKFILLS recipes for
/// pre-recipe agents, and that backfilled line has no `--model` at all — so an
/// agent restarted through that path silently lost its model. Once the id is in
/// the config it survives every start (`up`, restart, resume) because they all
/// pass `--agent`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn migrate_launch_model(home: &Path, config: &Path) -> bool {
    let recipe_path = home.join("launch.json");
    let Ok(text) = std::fs::read_to_string(&recipe_path) else { return false };
    let Ok(mut recipe) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(cmd) = recipe.get("cmd").and_then(Value::as_str).map(str::to_string) else {
        return false;
    };
    // Model ids never contain whitespace, so token splitting is exact here.
    let mut tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(at) = tokens.iter().position(|t| *t == "--model") else { return false };
    let model = tokens
        .get(at + 1)
        .map(|m| m.trim_matches('\'').trim_matches('"').to_string())
        .filter(|m| !m.is_empty() && !m.starts_with('-'));
    tokens.drain(at..(at + 2).min(tokens.len()));
    recipe["cmd"] = json!(tokens.join(" "));
    let recipe_written = std::fs::write(
        &recipe_path,
        serde_json::to_string_pretty(&recipe).unwrap_or(text),
    )
    .is_ok();
    // Only fill a config that has no model of its own: a value already there
    // came from a newer spawn (or the user) and outranks the old launch line.
    let Some(model) = model else { return recipe_written };
    // And only if the backend actually accepts it. An id it rejects was never
    // the agent's model — kiro fell back to its default and said so above the
    // splash — so carrying the typo into the config would turn a working agent
    // into a mute one on its next restart. Dropping it preserves what was
    // really running.
    if let Err(e) = crate::projects::models::validate("kiro", &model) {
        eprintln!("projects: dropping the launch line's model for a managed agent — {e}");
        return recipe_written;
    }
    let config_written = std::fs::read_to_string(config)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|mut root| {
            let obj = root.as_object_mut()?;
            if obj.contains_key("model") {
                return None;
            }
            obj.insert("model".into(), json!(model));
            Some(std::fs::write(config, serde_json::to_string_pretty(&root).unwrap()).is_ok())
        })
        .unwrap_or(false);
    recipe_written || config_written
}

/// The environment a managed kiro pane launches with — ONE definition for
/// the spawn recipe and the launch.json backfill.
///
/// `KIRO_HOME` is the isolated home (the agent's identity). `KIRO_SKIP_MIDWAY_CHECK`
/// (board #183, owner 2026-09-12: "跳过这个提示，避免启动被阻塞住"): a managed
/// pane is unattended, and kiro-cli's launch-time "Your Midway session has
/// expired or is missing. Refresh it now with mwinit? [y/N]" parks the agent
/// until a human types. kiro-cli 2.21.4 reads this switch right beside that
/// prompt (`crates/chat-cli/src/launch/midway.rs`); there is no flag or
/// setting for it. Skipping the CHECK changes nothing about kiro's own login —
/// Midway only gates internal endpoints, and an expired session fails those
/// calls instead of freezing the launch.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn kiro_launch_env(home: &Path) -> Vec<(String, String)> {
    vec![
        ("KIRO_HOME".into(), home.to_string_lossy().to_string()),
        ("KIRO_SKIP_MIDWAY_CHECK".into(), "1".into()),
    ]
}

/// The v3 engine's door to an agent's identity is the WORKSPACE, not
/// `KIRO_HOME` (board #207, measured on kiro-cli 2.22.1 with a private home:
/// the same profile is "not found" under any other name or from any other
/// cwd, and found the moment it sits in `<ws>/.kiro/agents/`). So, only while
/// the engine door is open, the workspace carries ONE entry per managed agent:
/// `<ws>/.kiro/agents/<name>.json` → symlink → the isolated home's profile.
/// The truth does not fork — v3 reads THROUGH the link, the CLI's own
/// upgrade writes through it — and v2 sees no change at all.
///
/// Rules: idempotent on every start; an existing file that is NOT our symlink
/// is never clobbered (fail loud, naming it); the path goes into
/// `.git/info/exclude` when the workspace is a git checkout, so an agent's
/// `git add -A` cannot commit it; removed with the agent.
pub(crate) const WS_AGENTS_DIR: &str = ".kiro/agents";
const GIT_EXCLUDE_MARK: &str = "# tmux-mobile managed kiro entries (board #207)";

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn workspace_entry_path(workspace: &Path, name: &str) -> std::path::PathBuf {
    workspace.join(WS_AGENTS_DIR).join(format!("{name}.json"))
}

/// Ensure (engine v3) or retire (engine v2) the workspace entry. Returns true
/// when the workspace changed; an error names the file that stands in the way.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn ensure_workspace_entry(workspace: &Path, home: &Path, name: &str) -> Result<bool, String> {
    let open = crate::config::kiro_engine() == "v3";
    let link = workspace_entry_path(workspace, name);
    let target = home.join("agents").join(format!("{name}.json"));
    let meta = std::fs::symlink_metadata(&link).ok();
    if !open {
        // Door closed: our link goes; anything else there is not ours.
        return Ok(meta.is_some_and(|m| m.file_type().is_symlink()
            && std::fs::read_link(&link).ok().as_deref() == Some(target.as_path()))
            && std::fs::remove_file(&link).is_ok());
    }
    if let Some(m) = meta {
        if m.file_type().is_symlink() {
            if std::fs::read_link(&link).ok().as_deref() == Some(target.as_path()) {
                return Ok(false); // already ours, pointing right
            }
            std::fs::remove_file(&link).map_err(|e| format!("replace stale link {}: {e}", link.display()))?;
        } else {
            return Err(format!(
                "{} exists and is not a tmux-mobile link — the v3 engine would read it as '{name}'; move it aside or rename the agent",
                link.display()
            ));
        }
    }
    std::fs::create_dir_all(link.parent().unwrap()).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(&target, &link).map_err(|e| format!("link {}: {e}", link.display()))?;
    exclude_from_git(workspace);
    Ok(true)
}

/// Remove the entry an agent left in the workspace (agent removal). Only a
/// symlink into `.tmm/agents/` is ours to remove.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn remove_workspace_entry(workspace: &Path, name: &str) -> bool {
    let link = workspace_entry_path(workspace, name);
    let ours = std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::read_link(&link).is_ok_and(|t| t.to_string_lossy().contains("/.tmm/agents/"));
    ours && std::fs::remove_file(&link).is_ok()
}

/// `.git/info/exclude` — local, untracked, idempotent: the workspace's own
/// `.gitignore` is the project's, not ours to edit.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn exclude_from_git(workspace: &Path) {
    let git = workspace.join(".git");
    if !git.is_dir() {
        return;
    }
    let path = git.join("info").join("exclude");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let line = format!("/{WS_AGENTS_DIR}/");
    if current.lines().any(|l| l.trim() == line) {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    let _ = std::fs::write(&path, format!("{current}{sep}{GIT_EXCLUDE_MARK}\n{line}\n"));
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn render_kiro(
    def: &RegAgent, name: &str, home: &Path, workspace: &Path, system_prompt: &str,
    skills: &[crate::projects::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    // Fail-loud at spawn (a home without settings/cli.json would re-enable the
    // trust-all confirmation, which nobody is there to answer); refresh_hooks
    // reuses the same canonical list fail-soft via ensure_kiro_settings.
    let mut settings: serde_json::Map<String, Value> =
        kiro_cli_settings().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    if !def.model.trim().is_empty() {
        settings.insert(DEFAULT_MODEL_KEY.into(), json!(def.model.trim()));
    }
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&Value::Object(settings)).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("kiro");

    let resources: Vec<String> = skills
        .iter()
        .map(|sk| format!("skill://{}/SKILL.md", sk.dir.to_string_lossy()))
        .collect();
    let mut mcp_servers = json!({});
    for m in &crate::projects::spawn::mcp_defs(def) {
        if !m.name.is_empty() {
            mcp_servers.as_object_mut().unwrap().insert(m.name.clone(), shared::kiro_mcp_value(m));
        }
    }
    let mut conf = json!({
        "name": name,
        "description": format!("{} (registry agent)", def.name),
        "prompt": system_prompt,
        "tools": ["*"],
        "allowedTools": ["*"],
        "resources": resources,
        // Native MCP again (owner, 2026-08-28: "mcp 工具还是用原生的方式调用
        // 吧") — the context cost is handled by toolSearch instead
        // (kiro_cli_settings enables it): schemas are DEFERRED into a compact
        // list and loaded on demand via kiro's own tool_search. The `tmm mcp`
        // CLI stays available as a SKILL, never taught in the prompt.
        "mcpServers": mcp_servers,
        "hooks": kiro_hooks(&notify),
        "permissions": kiro_permissions(),
    });
    // The model belongs to the agent's IDENTITY, not to one launch of it. It
    // used to ride on `--model`, which had two costs: it was invisible in the
    // config the owner reads (`.tmm/agents/<name>/agents/<name>.json`), and
    // kiro-cli's TUI answers an unknown id with a warning above the splash and
    // then runs its DEFAULT model — so a typo'd id was a silent downgrade. In
    // the config, kiro reports it as a real error on the first turn instead,
    // and every later start (resume, restart, `up`) reads the same field.
    // `registry_save` rejects unknown ids up front.
    //
    // An empty model means what the editor's placeholder says — the BACKEND's
    // default — so the key is omitted rather than set to a hardcoded id (the
    // old launch line pinned `claude-sonnet-4.6`, which silently contradicted
    // the UI and would have outlived that model).
    let model = def.model.trim();
    if !model.is_empty() {
        conf["model"] = json!(model);
    }
    std::fs::write(home.join("agents").join(format!("{name}.json")), serde_json::to_string_pretty(&conf).unwrap())
        .map_err(|e| e.to_string())?;
    // v3 finds the identity through the workspace (fail loud: a foreign file
    // there would be read as this agent).
    ensure_workspace_entry(workspace, home, name)?;

    Ok(Rendered {
        env: kiro_launch_env(home),
        cmd: format!(
            "command kiro-cli chat --agent {} --trust-all-tools{}{}",
            crate::shell::quote(name),
            effort_flag(def),
            engine_flag(),
        ),
        confirmation: None,
    })
}


/// The kiro half of `refresh_hooks` (board #128): hooks, launch-model
/// migration, canonical settings, and the pre-recipe backfill — a home whose
/// `agents/<name>.json` exists but whose recipe does not is an old spawn
/// whose restart identity is reconstructible from the isolated home itself.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn refresh(home: &Path, window_name: &str, workspace: &Path, notify: &str) -> bool {
    let mut changed = false;
    let config = home.join("agents").join(format!("{window_name}.json"));
    if config.is_file() {
        // The workspace entry follows the engine door at every start (#207);
        // refresh is fail-soft, so a foreign file only logs.
        match ensure_workspace_entry(workspace, home, window_name) {
            Ok(c) => changed |= c,
            Err(e) => eprintln!("projects: kiro v3 workspace entry for '{window_name}': {e}"),
        }
        changed |= patch_profile(&config, kiro_hooks(notify));
        changed |= migrate_launch_model(home, &config);
        // Settings drift is config drift: agents spawned before queue-mode
        // (or before settings existed at all) get the canonical file on their
        // next start, same as hooks; the v3 model mirror follows the profile.
        changed |= ensure_kiro_settings(home, &profile_model(&config));
        // The engine door is read at every start (board #207).
        changed |= reconcile_recipe_engine(home);
        if !home.join("launch.json").exists() {
            // Best effort: a backfill that cannot be written changes nothing,
            // and the restart then takes the generic launch path.
            changed |= crate::projects::spawn::LaunchRecipe {
                backend: "kiro",
                env: &kiro_launch_env(home),
                cmd: &format!(
                    "command kiro-cli chat --agent {} --trust-all-tools{} kick",
                    crate::shell::quote(window_name),
                    engine_flag(),
                ),
                // A backfilled recipe cannot know who spawned the agent — the
                // provenance does not exist for pre-recipe spawns; refresh
                // falls back to the window name.
                spawned_by: "",
                team: None,
                agent_def: "",
                member: "",
            }
            .write(home)
            .is_ok();
        }
    }
    changed
}

/// kiro resume dialect: `--resume-id <id>` exact, `--resume` recent (the
/// isolated KIRO_HOME scopes it to this one agent).
pub(crate) fn resume_command(cmd: &str, id: Option<&str>) -> String {
    match id {
        Some(id) => format!("{cmd} --resume-id {}", crate::shell::quote(id)),
        None => format!("{cmd} --resume"),
    }
}

/// This backend's detection/relaunch row (board #129). Resume flags from
/// `kiro-cli chat --help`: `-r/--resume` — "Resume the most recent
/// conversation from this directory"; `--resume-id <SESSION_ID>` exact.
/// The recipe-based resume dialect lives in `resume_command` above — same
/// knowledge, the template form serves adopted/hand-started windows.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn known() -> crate::projects::agents::KnownAgent {
    crate::projects::agents::KnownAgent {
        backend: "kiro",
        needle: "kiro",
        launch: "kiro-cli chat",
        resume_recent: Some("kiro-cli chat --resume"),
        resume_id: Some("kiro-cli chat --resume-id {id}"),
    }
}

/// kiro's hook payload dialect (board #129): snake_case `hook_event_name`,
/// a turn's end is `stop`/`Stop`.
pub(crate) fn normalize_kind(
    payload: &serde_json::Map<String, Value>,
) -> Result<&'static str, String> {
    let event = crate::agent_notifications::string_field(payload, &["hook_event_name"]);
    if !matches!(event.as_deref(), Some("stop" | "Stop")) {
        return Err("unsupported Kiro event".into());
    }
    Ok("completed")
}

/// kiro marks a new user turn with `hook_event_name: userPromptSubmit`
/// (case-insensitive) — resets the auto-post dedup flag.
pub(crate) fn is_user_prompt_submit(payload: &Value) -> bool {
    payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .is_some_and(|e| e.eq_ignore_ascii_case("userpromptsubmit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Board #207: the engine door edits the recorded launch line exactly.
    #[test]
    fn engine_segment_is_added_replaced_or_removed_exactly() {
        let base = "command kiro-cli chat --agent dev --trust-all-tools";
        assert_eq!(reconcile_engine_in(base, ""), None, "closed door, no segment: nothing to do");
        assert_eq!(reconcile_engine_in(base, " --agent-engine v3").as_deref(), Some("command kiro-cli chat --agent dev --trust-all-tools --agent-engine v3"));
        let v3 = "command kiro-cli chat --agent dev --trust-all-tools --agent-engine v3";
        assert_eq!(reconcile_engine_in(v3, " --agent-engine v3"), None, "already open: idempotent");
        assert_eq!(reconcile_engine_in(v3, "").as_deref(), Some(base), "door closed: the segment goes, the line is what it was");
        let kick = "command kiro-cli chat --agent dev --trust-all-tools --agent-engine v3 kick";
        assert_eq!(reconcile_engine_in(kick, "").as_deref(), Some("command kiro-cli chat --agent dev --trust-all-tools kick"));
        assert_eq!(reconcile_engine_in("x --agent-engine v2", " --agent-engine v3").as_deref(), Some("x --agent-engine v3"));
    }

    /// Board #207: v3 keeps the reply in the global session file. Real row
    /// shape from ~/.kiro/sessions/…/messages.jsonl (kiro-cli 2.22.1):
    /// {id, timestamp, payload:{type, content, …}}.
    #[test]
    fn v3_reply_is_the_last_assistant_entry_of_the_last_turn() {
        let row = |t: &str, extra: &str| format!(r#"{{"id":"x","timestamp":"2026-09-20T02:00:00Z","payload":{{"type":"{t}"{extra}}}}}"#);
        let jsonl = [
            row("user", r#","content":"first question""#),
            row("turn_start", r#","executionId":"e1""#),
            row("assistant", r#","content":"old answer""#),
            row("turn_end", r#","stopReason":"end_turn""#),
            row("user", r#","content":"Run the shell command: echo debug-probe""#),
            row("turn_start", r#","executionId":"e2""#),
            row("tool_call", r#","toolName":"execute_bash""#),
            row("assistant", r#","content":"Running it.""#),
            row("tool_result", r#","content":"Output:\ndebug-probe""#),
            "this line is not json".to_string(),
            row("assistant", r#","content":"```\ndebug-probe\n```\nExit code: 0""#),
            row("turn_end", r#","stopReason":"end_turn""#),
            row("session_start", r#","content":"You are a probe.""#),
        ].join("\n");
        assert_eq!(kiro_reply_from_session(&jsonl).as_deref(), Some("```\ndebug-probe\n```\nExit code: 0"));
        assert_eq!(kiro_reply_from_session(&row("user", r#","content":"q""#)), None, "no assistant yet");
        assert_eq!(kiro_reply_from_session(""), None);
        // Bare rows (no payload wrapper) are read too.
        assert_eq!(kiro_reply_from_session(r#"{"type":"assistant","content":"bare"}"#).as_deref(), Some("bare"));
    }

    #[test]
    fn session_reply_finds_the_session_under_any_cwd_hash() {
        let root = std::env::temp_dir().join(format!("tmm-kiro-sessions-{}", std::process::id()));
        let sess = root.join("1f1f982434702399").join("sess_abc");
        std::fs::create_dir_all(&sess).unwrap();
        std::fs::write(sess.join("messages.jsonl"), "{\"payload\":{\"type\":\"turn_start\"}}\n{\"payload\":{\"type\":\"assistant\",\"content\":\"done and verified\"}}\n{\"payload\":{\"type\":\"turn_end\"}}").unwrap();
        assert_eq!(session_reply(&root, "sess_abc").as_deref(), Some("done and verified"));
        assert_eq!(session_reply(&root, "sess_nope"), None);
        assert_eq!(session_reply(&root, "../etc"), None, "a path is not a session id");
        assert_eq!(session_reply(&root, ""), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Board #207: the v3 status line prints the model's DISPLAY name.
    #[test]
    fn display_name_models_are_read_only_after_the_anchor() {
        assert!(looks_like_model_name("Claude Sonnet 5"));
        assert!(looks_like_model_name("GPT 5.1 Codex"));
        assert!(!looks_like_model_name("Midway: 19h 19m"), "a colon is not a model");
        assert!(!looks_like_model_name("◔ 4%"));
        assert!(!looks_like_model_name("/local/home/cfu/temp/kiro-v3-test/ws"));
        assert!(!looks_like_model_name("(probe-branch)"));
        let v3 = "probe · Claude Sonnet 5 · high · ◔ 4% · Midway: 19h 19m   /local/home/cfu/temp/kiro-v3-test/ws · (probe-branch)\n";
        let v = sniff_kiro(v3, "probe");
        assert_eq!(v.model.as_deref(), Some("Claude Sonnet 5"));
        assert_eq!(v.effort.as_deref(), Some("high"));
        assert_eq!(v.context_pct, Some(4));
        assert_eq!(v.branch.as_deref(), Some("probe-branch"));
        let v2 = "probe · claude-sonnet-5 · high · ◔ 1% · Midway: 19h 19m   /local/home/cfu/temp/kiro-v3-test/ws · (probe-branch)\n";
        let v = sniff_kiro(v2, "probe");
        assert_eq!(v.model.as_deref(), Some("claude-sonnet-5"));
        // Negative control: the same words on an UNANCHORED line are prose.
        let prose = "I switched to Claude Sonnet 5 · high · ◔ 4%\n";
        assert_eq!(sniff_kiro(prose, "probe").model, None);
    }

    /// Board #207 option A: the v3 engine finds the agent through the
    /// WORKSPACE, so the door being open puts a symlink there and closing it
    /// takes the link away; a foreign file is never clobbered.
    #[test]
    fn workspace_entry_follows_the_engine_door_and_never_clobbers() {
        let root = std::env::temp_dir().join(format!("tmm-kiro-wsentry-{}", std::process::id()));
        let ws = root.join("ws");
        let home = ws.join(".tmm").join("agents").join("dev");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::create_dir_all(ws.join(".git").join("info")).unwrap();
        std::fs::write(home.join("agents").join("dev.json"), "{}").unwrap();
        let link = workspace_entry_path(&ws, "dev");

        // Door closed (default): zero change to the workspace.
        std::env::remove_var("KIRO_ENGINE");
        assert!(!ensure_workspace_entry(&ws, &home, "dev").unwrap());
        assert!(!link.exists() && !ws.join(".kiro").exists(), "v2 leaves no trace");

        // Door open: the link, idempotent, and the git exclude.
        std::env::set_var("KIRO_ENGINE", "v3");
        assert!(ensure_workspace_entry(&ws, &home, "dev").unwrap());
        assert_eq!(std::fs::read_link(&link).unwrap(), home.join("agents").join("dev.json"));
        assert!(!ensure_workspace_entry(&ws, &home, "dev").unwrap(), "second start: nothing to do");
        let exclude = std::fs::read_to_string(ws.join(".git/info/exclude")).unwrap();
        assert!(exclude.lines().any(|l| l == "/.kiro/agents/"), "{exclude}");
        ensure_workspace_entry(&ws, &home, "dev").unwrap();
        assert_eq!(std::fs::read_to_string(ws.join(".git/info/exclude")).unwrap(), exclude, "exclude is written once");
        // Writing THROUGH the link lands in the home (what the CLI's upgrade does).
        std::fs::write(&link, "{\"name\":\"dev\"}").unwrap();
        assert_eq!(std::fs::read_to_string(home.join("agents/dev.json")).unwrap(), "{\"name\":\"dev\"}");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "still a link, no copy");

        // A foreign file in the way: fail loud, naming it, leave it alone.
        let other = workspace_entry_path(&ws, "ops");
        std::fs::write(&other, "{\"name\":\"ops\",\"mine\":true}").unwrap();
        let err = ensure_workspace_entry(&ws, &home, "ops").unwrap_err();
        assert!(err.contains("ops.json") && err.contains("not a tmux-mobile link"), "{err}");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "{\"name\":\"ops\",\"mine\":true}");
        assert!(!remove_workspace_entry(&ws, "ops"), "not ours to remove");

        // Door closed again: our link goes, the foreign file stays.
        std::env::set_var("KIRO_ENGINE", "v2");
        assert!(ensure_workspace_entry(&ws, &home, "dev").unwrap());
        assert!(!link.exists() && other.exists());
        // Removal helper: ours only.
        std::env::set_var("KIRO_ENGINE", "v3");
        ensure_workspace_entry(&ws, &home, "dev").unwrap();
        assert!(remove_workspace_entry(&ws, "dev") && !link.exists());
        std::env::remove_var("KIRO_ENGINE");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Board #207: the 3.0 profile is written once and never churned — a home
    /// the CLI already upgraded (array + permissions) is left byte-identical.
    #[test]
    fn patch_profile_is_idempotent_on_both_shapes() {
        let dir = std::env::temp_dir().join(format!("tmm-kiro-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.json");
        std::fs::write(&path, serde_json::to_string_pretty(&json!({
            "name": "a", "prompt": "p", "hooks": { "stop": [ { "command": "old" } ] }
        })).unwrap()).unwrap();
        assert!(patch_profile(&path, kiro_hooks("n")), "2.0 object → 3.0 array + permissions");
        let once = std::fs::read_to_string(&path).unwrap();
        assert!(!patch_profile(&path, kiro_hooks("n")), "already 3.0: no write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
        let v: Value = serde_json::from_str(&once).unwrap();
        assert!(v["hooks"].is_array());
        assert_eq!(v["permissions"]["rules"][0]["capability"], "all");
        // A permissions block someone else wrote is theirs.
        std::fs::write(&path, serde_json::to_string_pretty(&json!({
            "name": "a", "hooks": kiro_hooks("n"), "permissions": { "rules": [ { "capability": "shell", "effect": "ask" } ] }
        })).unwrap()).unwrap();
        assert!(!patch_profile(&path, kiro_hooks("n")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
