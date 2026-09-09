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

use crate::projects::vitals::{branch, context_pct, looks_like_model, Vitals, EFFORTS};

/// Read what the last lines of a pane say about the agent's current state.
///
/// `agent` is normally the first status segment (the managed window name).
/// Resumed legacy conversations may retain the exact built-in `kiro_default`
/// identity, which is accepted as the one narrow fallback. The anchor is not a
/// filter: fields that identify themselves by shape (context and branch) are
/// read even when it never appears, because narrow panes wrap later segments.
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
                if let Some(m) = next.filter(|m| looks_like_model(m)) {
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
use std::path::Path;

use crate::projects::spawn::{effort_flag, patch_hooks, Rendered};
use crate::projects::store::RegAgent;

use super::shared;

/// The hook set for each backend, in ONE place. `render_*` writes it at spawn
/// and `refresh_hooks` rewrites it on every start, so a config on disk can
/// never be older than the app that reads its events. (It was: agents spawned
/// before `userPromptSubmit` existed kept a three-hook config, and since that
/// hook is the only reset of the same-turn dedup flag, their first `tmm send`
/// silently killed the stop-hook auto-post for the rest of the window's life.)
pub(crate) fn kiro_hooks(notify: &str) -> Value {
    json!({
        // The notify helper feeds notifications AND telemetry (tool events are
        // recognized by hook_event_name and routed to telemetry only).
        "preToolUse":  [ { "matcher": "*", "command": notify } ],
        "postToolUse": [ { "matcher": "*", "command": notify } ],
        // Turn start — the ONLY reset of the same-turn dedup flag, and the
        // event that carries the submitted prompt.
        "userPromptSubmit": [ { "command": notify } ],
        "stop": [ { "command": notify } ]
    })
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
    ]
}

/// Force the canonical CLI settings into a managed kiro home, leaving any
/// other keys alone. Creates the file when it is missing (pre-settings homes),
/// no-op write when everything already matches — the same contract as
/// `patch_hooks`, because the app owns these configs. Returns true on change.
pub(crate) fn ensure_kiro_settings(home: &Path) -> bool {
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
    changed && std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap()).is_ok()
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

pub(crate) fn render_kiro(
    def: &RegAgent, name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::projects::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    // Fail-loud at spawn (a home without settings/cli.json would re-enable the
    // trust-all confirmation, which nobody is there to answer); refresh_hooks
    // reuses the same canonical list fail-soft via ensure_kiro_settings.
    let settings: serde_json::Map<String, Value> =
        kiro_cli_settings().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
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

    Ok(Rendered {
        env: vec![("KIRO_HOME".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command kiro-cli chat --agent {} --trust-all-tools{}",
            shared::shell_quote(name),
            effort_flag(def),
        ),
        confirmation: None,
    })
}


/// The kiro half of `refresh_hooks` (board #128): hooks, launch-model
/// migration, canonical settings, and the pre-recipe backfill — a home whose
/// `agents/<name>.json` exists but whose recipe does not is an old spawn
/// whose restart identity is reconstructible from the isolated home itself.
pub(crate) fn refresh(home: &Path, window_name: &str, notify: &str) -> bool {
    let mut changed = false;
    let config = home.join("agents").join(format!("{window_name}.json"));
    if config.is_file() {
        changed |= patch_hooks(&config, kiro_hooks(notify));
        changed |= migrate_launch_model(home, &config);
        // Settings drift is config drift: agents spawned before queue-mode
        // (or before settings existed at all) get the canonical file on their
        // next start, same as hooks.
        changed |= ensure_kiro_settings(home);
        if !home.join("launch.json").exists() {
            // Best effort: a backfill that cannot be written changes nothing,
            // and the restart then takes the generic launch path.
            changed |= crate::projects::spawn::write_launch_recipe(
                home,
                "kiro",
                &[("KIRO_HOME".to_string(), home.to_string_lossy().to_string())],
                &format!(
                    "command kiro-cli chat --agent {} --trust-all-tools kick",
                    shared::shell_quote(window_name),
                ),
                // A backfilled recipe cannot know who spawned the agent — the
                // provenance does not exist for pre-recipe spawns; refresh
                // falls back to the window name.
                "", None, "", "")
            .is_ok();
        }
    }
    changed
}

/// kiro resume dialect: `--resume-id <id>` exact, `--resume` recent (the
/// isolated KIRO_HOME scopes it to this one agent).
pub(crate) fn resume_command(cmd: &str, id: Option<&str>) -> String {
    match id {
        Some(id) => format!("{cmd} --resume-id {}", shared::shell_quote(id)),
        None => format!("{cmd} --resume"),
    }
}
