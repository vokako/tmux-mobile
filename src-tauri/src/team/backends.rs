//! Per-backend launch preparation (Kiro / Claude Code / Codex): MCP config
//! rendering, per-agent HOME seeding, hook wiring, and CLI arg assembly.
//! Split from team.rs 2026-07-22 — content unchanged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::skills::{ResolvedSkill, skills_index_text};
use super::workspace::Paths;
use super::workspace::prepare_kiro_home;
use super::launch::{
    build_agent_prompt, Prepared, StartupConfirmation,
    CLAUDE_FOLDER_TRUST_MARKERS, CODEX_FOLDER_TRUST_MARKERS,
};
use super::{TeamConfig, TEAM_MCP_TOOL_TIMEOUT_MS};

// ---- Kiro ----
#[allow(clippy::too_many_arguments)] // agent config genuinely needs all of these
/// An extra MCP server attached to an agent (from the team.yaml `mcp:` list).
/// Either a remote HTTP server (`url` [+ `headers`]) or a local stdio server
/// (`command` [+ `args`/`env`]).
#[derive(serde::Deserialize, Default, Clone)]
pub(crate) struct McpDef {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) command: Option<String>,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) env: std::collections::BTreeMap<String, String>,
}

/// A skill resolved to a concrete local directory (containing SKILL.md), ready
/// to wire into a backend.
/// Per-agent extras threaded from the spec into each backend's launcher.
#[derive(Default)]
pub(super) struct Extras {
    pub(super) env: Vec<(String, String)>,
    pub(super) mcp: Vec<McpDef>,
    pub(super) skills: Vec<ResolvedSkill>,
}

fn env_reference(value: &str) -> Option<&str> {
    let name = value
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        Some(name)
    } else {
        None
    }
}

fn header_env_reference(value: &str) -> Option<(&str, bool)> {
    if let Some(name) = value.strip_prefix("Bearer ").and_then(env_reference) {
        Some((name, true))
    } else {
        env_reference(value).map(|name| (name, false))
    }
}

fn interpolated_headers(headers: &std::collections::BTreeMap<String, String>) -> Value {
    let values: std::collections::BTreeMap<String, String> = headers
        .iter()
        .map(|(key, value)| {
            let value = match header_env_reference(value) {
                Some((name, true)) => format!("Bearer ${{{name}}}"),
                Some((name, false)) => format!("${{{name}}}"),
                None => value.clone(),
            };
            (key.clone(), value)
        })
        .collect();
    serde_json::to_value(values).unwrap_or(Value::Null)
}

/// kiro mcpServers entry: remote = `{url,headers}`, local = `{command,args,env}`.
pub(crate) fn kiro_mcp_value(m: &McpDef) -> Value {
    if let Some(url) = &m.url {
        let mut o = serde_json::json!({ "url": url });
        if !m.headers.is_empty() {
            o["headers"] = interpolated_headers(&m.headers);
        }
        o
    } else if let Some(cmd) = &m.command {
        let mut o = serde_json::json!({ "command": cmd, "args": m.args });
        if !m.env.is_empty() {
            o["env"] = serde_json::to_value(&m.env).unwrap_or(Value::Null);
        }
        o
    } else {
        serde_json::json!({})
    }
}

/// claude mcpServers entry: remote gets explicit `type:"http"`.
pub(crate) fn claude_mcp_value(m: &McpDef) -> Value {
    if let Some(url) = &m.url {
        let mut o = serde_json::json!({ "type": "http", "url": url });
        if !m.headers.is_empty() {
            o["headers"] = serde_json::to_value(&m.headers).unwrap_or(Value::Null);
        }
        o
    } else {
        kiro_mcp_value(m) // local stdio form is identical
    }
}

fn codex_key_segment(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap()
    }
}

pub(crate) fn codex_config_override(key: &str, value: Value) -> String {
    let assignment = format!("{}={}", key, serde_json::to_string(&value).unwrap());
    format!("-c {}", shell_quote(&assignment))
}

/// Codex CLI overrides for one extra MCP server. Team keeps the system
/// config.toml intact and layers room-specific MCP settings at launch.
pub(crate) fn codex_mcp_overrides(m: &McpDef) -> Vec<String> {
    let name = codex_key_segment(&m.name);
    let prefix = format!("mcp_servers.{}", name);
    let mut args = Vec::new();
    if let Some(url) = &m.url {
        args.push(codex_config_override(&format!("{}.url", prefix), Value::String(url.clone())));
        args.push(codex_config_override(&format!("{}.enabled", prefix), Value::Bool(true)));
        args.push(codex_config_override(
            &format!("{}.experimental_use_rmcp_client", prefix),
            Value::Bool(true),
        ));
        for (key, value) in &m.headers {
            match header_env_reference(value) {
                Some((name, true)) if key.eq_ignore_ascii_case("authorization") => {
                    args.push(codex_config_override(
                        &format!("{}.bearer_token_env_var", prefix),
                        Value::String(name.to_string()),
                    ));
                }
                Some((name, false)) => {
                    args.push(codex_config_override(
                        &format!("{}.env_http_headers.{}", prefix, codex_key_segment(key)),
                        Value::String(name.to_string()),
                    ));
                }
                _ => {
                    args.push(codex_config_override(
                        &format!("{}.http_headers.{}", prefix, codex_key_segment(key)),
                        Value::String(value.clone()),
                    ));
                }
            }
        }
    } else if let Some(cmd) = &m.command {
        args.push(codex_config_override(
            &format!("{}.command", prefix),
            Value::String(cmd.clone()),
        ));
        if !m.args.is_empty() {
            args.push(codex_config_override(
                &format!("{}.args", prefix),
                serde_json::to_value(&m.args).unwrap(),
            ));
        }
        for (key, value) in &m.env {
            args.push(codex_config_override(
                &format!("{}.env.{}", prefix, codex_key_segment(key)),
                Value::String(value.clone()),
            ));
        }
    }
    args
}

fn codex_team_mcp_overrides(m: &McpDef) -> Vec<String> {
    let mut args = codex_mcp_overrides(m);
    args.push(codex_config_override(
        "mcp_servers.team.tool_timeout_sec",
        Value::from(team_mcp_tool_timeout_ms() / 1000),
    ));
    args
}

fn team_mcp_tool_timeout_ms() -> u64 {
    TEAM_MCP_TOOL_TIMEOUT_MS
}

fn build_cli_system_prompt(
    role: &str,
    goal: &str,
    team_prompt: &str,
    cfg: &TeamConfig,
    skills: &[ResolvedSkill],
) -> String {
    let mut prompt = build_agent_prompt(role, goal, team_prompt, cfg);
    let skills = skills_index_text(skills);
    if !skills.is_empty() {
        prompt.push_str("\n\n<skills-system-prompt>\n");
        prompt.push_str(&skills);
        prompt.push_str("\n</skills-system-prompt>");
    }
    prompt
}

fn system_codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

/// Keep Team runtime state isolated while sharing the system Codex provider and
/// login. Links follow config/token refreshes without copying credentials.
pub(crate) fn inherit_codex_system_files(home: &Path) -> Result<(), String> {
    inherit_codex_system_files_from(home, &system_codex_home())
}

fn link_codex_system_file(
    home: &Path,
    system_home: &Path,
    filename: &str,
    replace_team_owned: bool,
) -> Result<(), String> {
    let source = system_home.join(filename);
    if !source.is_file() {
        if replace_team_owned {
            let target = home.join(filename);
            match std::fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
                    std::fs::remove_file(target).map_err(|e| e.to_string())?;
                }
                Ok(_) => return Err(format!("refusing to replace Codex path: {}", target.display())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        return Ok(());
    }
    let source = std::fs::canonicalize(source).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let target = home.join(filename);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                && std::fs::read_link(&target).is_ok_and(|path| path == source)
            {
                return Ok(());
            }
            if replace_team_owned && (metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            } else {
                return Err(format!(
                    "refusing to replace existing Codex path: {}",
                    target.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }

    symlink_file(&source, &target).map_err(|e| {
        format!(
            "failed to inherit Codex system file from {}: {}",
            source.display(),
            e
        )
    })
}

fn inherit_codex_system_files_from(home: &Path, system_home: &Path) -> Result<(), String> {
    // config.toml in the private home was Team-generated before MCP settings
    // moved to CLI overrides, so it is the one path Team may replace.
    link_codex_system_file(home, system_home, "config.toml", true)?;
    link_codex_system_file(home, system_home, ".env", false)?;
    link_codex_system_file(home, system_home, "auth.json", false)?;
    // Profile layers (`codex --profile <name>` reads `<name>.config.toml`).
    // A machine whose codex auth lives in a profile (e.g. a Bedrock provider
    // with the bearer token in .env, no ChatGPT login) needs these in the
    // isolated home or the agent boots into the sign-in screen.
    if let Ok(entries) = std::fs::read_dir(system_home) {
        for entry in entries.flatten() {
            let filename = entry.file_name();
            let name = filename.to_string_lossy();
            if name.ends_with(".config.toml") && entry.path().is_file() {
                link_codex_system_file(home, system_home, &name, true)?;
            }
        }
    }
    Ok(())
}

fn codex_developer_instructions(home: &Path, team_instructions: &str) -> Result<String, String> {
    let path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => {
            let config: toml::Value = toml::from_str(&content)
                .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
            match config.get("developer_instructions") {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "{} developer_instructions must be a string",
                                path.display()
                            )
                        })?
                        .trim()
                        .to_string(),
                ),
                None => None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("failed to read {}: {}", path.display(), error)),
    };
    let team = format!(
        "<tmux-mobile-team-instructions>\n{}\n</tmux-mobile-team-instructions>",
        team_instructions.trim()
    );
    Ok(match existing.filter(|value| !value.is_empty()) {
        Some(existing) => format!("{}\n\n{}", existing, team),
        None => team,
    })
}

#[cfg(unix)]
fn symlink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn symlink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, target)
}

/// Env the agent process exports so its `heartbeat.sh` hook can ping the daemon
/// (who am I, which room, where). Injected on EVERY backend's launch line.
pub(super) fn hb_env(name: &str, room: &str, cfg: &TeamConfig) -> Vec<(String, String)> {
    vec![
        ("TEAM_HB_URL".to_string(), format!("{}/api/heartbeat", cfg.url)),
        ("TEAM_AGENT".to_string(), name.to_string()),
        ("TEAM_ROOM".to_string(), room.to_string()),
    ]
}

pub(super) fn prepare_kiro(
    name: &str, role: &str, goal: &str, team_prompt: &str,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    prepare_kiro_home(paths).map_err(|e| e.to_string())?;
    let home = &paths.kiro;
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "chat.disableTrustAllConfirmation": true })).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("kiro");
    let tools = vec!["*".to_string(), "@team".to_string()];

    // Skills are loaded as native skill:// resources.
    let resources: Vec<String> = extras.skills.iter()
        .map(|sk| format!("skill://{}/SKILL.md", sk.dir.to_string_lossy()))
        .collect();
    // The team MCP server plus any extra per-agent servers from the team.yaml.
    let mut mcp_servers = serde_json::json!({
        "team": {
            "url": format!("{}/mcp", cfg.url),
            "timeout": team_mcp_tool_timeout_ms(),
            "headers": { "x-agent": name, "x-room": room }
        }
    });
    {
        let obj = mcp_servers.as_object_mut().unwrap();
        for m in &extras.mcp {
            if !m.name.is_empty() {
                obj.insert(m.name.clone(), kiro_mcp_value(m));
            }
        }
    }
    let conf = serde_json::json!({
        "name": name,
        "description": format!("{} on the team bus", role),
        "prompt": build_agent_prompt(role, goal, team_prompt, cfg),
        "tools": tools,
        "allowedTools": ["*"],
        "resources": resources,
        "mcpServers": mcp_servers,
        "hooks": {
            "preToolUse": [ { "matcher": "*", "command": heartbeat_command(&paths.heartbeat, "pre") } ],
            "postToolUse": [ { "matcher": "*", "command": heartbeat_command(&paths.heartbeat, "post") } ],
            "userPromptSubmit": [ { "command": heartbeat_command(&paths.heartbeat, "pulse") } ],
            "stop": [
                { "command": bash_script_command(&paths.keepalive) },
                { "command": notify },
                { "command": heartbeat_command(&paths.heartbeat, "post") }
            ]
        },
    });
    std::fs::write(
        home.join("agents").join(format!("{}.json", name)),
        serde_json::to_string_pretty(&conf).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("claude-sonnet-4.6");
    let mut env = vec![("KIRO_HOME".to_string(), home.to_string_lossy().to_string())];
    env.extend(hb_env(name, room, cfg));
    env.extend(extras.env.iter().cloned());
    let cmd = format!(
        "command kiro-cli chat --agent {} --model {} --trust-all-tools {}",
        shell_quote(name),
        shell_quote(m),
        shell_quote(&cfg.team_kick)
    );
    Ok((env, cmd, None))
}

// ---- Claude Code ----
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_claude(
    name: &str, role: &str, goal: &str, team_prompt: &str,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    let d = &paths.claude;
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("claude");
    let mcpfile = d.join(format!("{}.mcp.json", name));
    let mut mcp_servers = serde_json::json!({
        "team": {
            "type": "http",
            "url": format!("{}/mcp", cfg.url),
            "timeout": team_mcp_tool_timeout_ms(),
            "headers": { "x-agent": name, "x-room": room }
        }
    });
    {
        let obj = mcp_servers.as_object_mut().unwrap();
        for m in &extras.mcp {
            if !m.name.is_empty() {
                obj.insert(m.name.clone(), claude_mcp_value(m));
            }
        }
    }
    std::fs::write(
        &mcpfile,
        serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": mcp_servers })).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let settingsfile = d.join(format!("{}.settings.json", name));
    std::fs::write(
        &settingsfile,
        serde_json::to_string_pretty(&serde_json::json!({
            "skipDangerousModePermissionPrompt": true,
            "hooks": {
                "PreToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "pre") } ] } ],
                "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") } ] } ],
                "PostToolUseFailure": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") } ] } ],
                "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "pulse") } ] } ],
                "Notification": [ { "matcher": "permission_prompt|idle_prompt|agent_needs_input|agent_completed", "hooks": [ { "type": "command", "command": notify } ] } ],
                "Stop": [ { "hooks": [
                    { "type": "command", "command": bash_script_command(&paths.keepalive) },
                    { "type": "command", "command": notify },
                    { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") }
                ] } ],
                "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
            }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("sonnet");
    let system_prompt = build_cli_system_prompt(role, goal, team_prompt, cfg, &extras.skills);
    let cmd = format!(
        "command claude --mcp-config {} --strict-mcp-config --settings {} --model {} --dangerously-skip-permissions --append-system-prompt {} {}",
        shell_quote(&mcpfile.to_string_lossy()),
        shell_quote(&settingsfile.to_string_lossy()),
        shell_quote(m),
        shell_quote(&system_prompt),
        shell_quote(&cfg.team_kick)
    )
    .trim_end()
    .to_string();
    // Tool permissions are bypassed by the CLI flag. Workspace trust is a
    // separate first-use dialog with no public skip flag, so confirm it only
    // after all stable prompt markers are visible.
    let confirmation = StartupConfirmation {
        markers: CLAUDE_FOLDER_TRUST_MARKERS.to_vec(),
        ready_markers: vec!["bypass permissions on"],
        timeout: Duration::from_secs(120),
    };
    let mut env = hb_env(name, room, cfg);
    env.extend(extras.env.iter().cloned());
    env.push(("MCP_TOOL_TIMEOUT".to_string(), team_mcp_tool_timeout_ms().to_string()));
    env.push(("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(), "0".to_string()));
    Ok((env, cmd, Some(confirmation)))
}

// ---- Codex ----
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_codex(
    name: &str, role: &str, goal: &str, team_prompt: &str, cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    let home = paths.codex.join(name);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    inherit_codex_system_files(&home)?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("codex");
    let team_mcp = McpDef {
        name: "team".to_string(),
        url: Some(format!("{}/mcp", cfg.url)),
        headers: [
            ("x-agent".to_string(), name.to_string()),
            ("x-room".to_string(), room.to_string()),
        ]
        .into_iter()
        .collect(),
        ..McpDef::default()
    };
    let mut config_args = codex_team_mcp_overrides(&team_mcp);
    for m in &extras.mcp {
        if !m.name.is_empty() {
            config_args.extend(codex_mcp_overrides(m));
        }
    }
    let system_prompt = build_cli_system_prompt(role, goal, team_prompt, cfg, &extras.skills);
    config_args.push(codex_config_override(
        "developer_instructions",
        Value::String(codex_developer_instructions(&home, &system_prompt)?),
    ));
    std::fs::write(
        home.join("hooks.json"),
        serde_json::to_vec_pretty(&codex_hooks_value(&paths.keepalive, &paths.heartbeat, &notify)).unwrap(),
    ).map_err(|e| e.to_string())?;
    let mut env = vec![("CODEX_HOME".to_string(), home.to_string_lossy().to_string())];
    env.extend(hb_env(name, room, cfg));
    env.extend(extras.env.iter().cloned());
    if let Some(value) = model {
        config_args.push(format!("--model {}", shell_quote(value)));
    }
    if !cfg.codex_profile.is_empty() {
        config_args.push(format!("--profile {}", shell_quote(&cfg.codex_profile)));
    }
    config_args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    config_args.push("--dangerously-bypass-hook-trust".to_string());
    config_args.push(shell_quote(&cfg.team_kick));
    // `command` bypasses shell functions/aliases: users commonly wrap `codex`
    // in a function that injects its own flags (e.g. --profile), which would
    // collide with ours ("cannot be used multiple times") or change behavior.
    let cmd = format!("command codex {}", config_args.join(" "));
    let confirmation = StartupConfirmation {
        markers: CODEX_FOLDER_TRUST_MARKERS.to_vec(),
        ready_markers: vec!["Starting MCP servers"],
        timeout: Duration::from_secs(120),
    };
    Ok((env, cmd, Some(confirmation)))
}

fn codex_hooks_value(keepalive: &Path, heartbeat: &Path, notify: &str) -> Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [ { "matcher": "*", "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "pre") }
            ] } ],
            "PermissionRequest": [ { "hooks": [
                { "type": "command", "command": notify }
            ] } ],
            "PostToolUse": [ { "matcher": "*", "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "post") }
            ] } ],
            "UserPromptSubmit": [ { "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "pulse") }
            ] } ],
            "Stop": [ { "hooks": [
                { "type": "command", "command": bash_script_command(keepalive) },
                { "type": "command", "command": notify },
                { "type": "command", "command": heartbeat_command(heartbeat, "post") }
            ] } ]
        }
    })
}

fn bash_script_command(path: &Path) -> String {
    format!("/bin/bash {}", shell_quote(&path.to_string_lossy()))
}

fn heartbeat_command(path: &Path, mode: &str) -> String {
    format!("{} {}", bash_script_command(path), mode)
}

/// Single-quote a string for the shell (the agent launch line is sent to a
/// tmux pane's shell). Wraps in '…' and escapes embedded single quotes.
pub(crate) fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b'=' | b':')) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::launch::{folder_trust_prompt_visible, startup_already_ready, startup_prompt_visible};
    use super::super::workspace::prepare_home;
    
    

    #[test]
    fn codex_system_files_link_config_env_and_auth_idempotently() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-system-{}", uuid::Uuid::new_v4()));
        let system_home = root.join("system");
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&system_home).unwrap();
        std::fs::write(system_home.join("config.toml"), "model_provider = \"custom\"").unwrap();
        std::fs::write(system_home.join(".env"), "PROVIDER_TOKEN=secret").unwrap();
        std::fs::write(system_home.join("auth.json"), "{}").unwrap();
        std::fs::write(system_home.join("personal.config.toml"), "model_provider = \"bedrock\"").unwrap();

        inherit_codex_system_files_from(&agent_home, &system_home).unwrap();
        inherit_codex_system_files_from(&agent_home, &system_home).unwrap();

        for filename in ["config.toml", ".env", "auth.json", "personal.config.toml"] {
            let target = agent_home.join(filename);
            assert!(std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert_eq!(
            std::fs::read_to_string(agent_home.join("config.toml")).unwrap(),
            "model_provider = \"custom\""
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_system_files_missing_source_removes_only_team_config() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-no-auth-{}", uuid::Uuid::new_v4()));
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&agent_home).unwrap();
        std::fs::write(agent_home.join("config.toml"), "[mcp_servers.team]").unwrap();

        inherit_codex_system_files_from(&agent_home, &root.join("system")).unwrap();

        assert!(!agent_home.join("config.toml").exists());
        assert!(!agent_home.join(".env").exists());
        assert!(!agent_home.join("auth.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_team_instructions_follow_existing_user_instructions() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-codex-instructions-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            "developer_instructions = \"Keep the user's convention.\"\n",
        )
        .unwrap();

        let merged = codex_developer_instructions(&root, "Team contract.").unwrap();

        assert!(merged.starts_with("Keep the user's convention."));
        assert!(merged.contains(
            "<tmux-mobile-team-instructions>\nTeam contract.\n</tmux-mobile-team-instructions>"
        ));
        assert!(
            merged.find("Keep the user's convention.").unwrap()
                < merged.find("Team contract.").unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_auth_does_not_replace_an_existing_private_file() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-existing-auth-{}", uuid::Uuid::new_v4()));
        let system_home = root.join("system");
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&system_home).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();
        std::fs::write(system_home.join("auth.json"), "system").unwrap();
        std::fs::write(agent_home.join("auth.json"), "private").unwrap();

        let error = inherit_codex_system_files_from(&agent_home, &system_home).unwrap_err();

        assert!(error.contains("refusing to replace"));
        assert_eq!(std::fs::read_to_string(agent_home.join("auth.json")).unwrap(), "private");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_stop_hooks_keep_wait_loop_alive_and_notify() {
        let hooks = codex_hooks_value(
            Path::new("/team/keepalive.sh"),
            Path::new("/team/heartbeat.sh"),
            "notify codex",
        );
        let stop = hooks["hooks"]["Stop"][0]["hooks"].as_array().unwrap();

        assert_eq!(stop.len(), 3);
        assert_eq!(stop[0]["command"], "/bin/bash /team/keepalive.sh");
        assert_eq!(stop[1]["command"], "notify codex");
        assert_eq!(stop[2]["command"], "/bin/bash /team/heartbeat.sh post");
        assert_eq!(
            hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "/bin/bash /team/heartbeat.sh pre"
        );
        assert_eq!(
            hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "/bin/bash /team/heartbeat.sh post"
        );
    }

    #[test]
    fn backend_launch_permissions_and_startup_confirmations() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-backend-permissions-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::new(root.to_str().unwrap(), "backend-permissions");
        prepare_home(&paths).unwrap();
        let extras = Extras::default();
        let cfg = cfg();

        let (kiro_env, kiro_cmd, kiro_confirmation) = prepare_kiro(
            "lead",
            "lead",
            "coordinate",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(kiro_cmd.contains("--trust-all-tools"));
        assert!(kiro_cmd.contains("--model claude-sonnet-4.6"));
        assert!(kiro_confirmation.is_none());
        let kiro_settings: Value = serde_json::from_slice(
            &std::fs::read(paths.kiro.join("settings/cli.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            kiro_settings["chat.disableTrustAllConfirmation"],
            Value::Bool(true)
        );
        let kiro_agent: Value = serde_json::from_slice(
            &std::fs::read(paths.kiro.join("agents/lead.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            kiro_agent["hooks"]["stop"][0]["command"],
            format!("/bin/bash {}", paths.keepalive.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["stop"][2]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["preToolUse"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["postToolUse"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["userPromptSubmit"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["mcpServers"]["team"]["timeout"],
            TEAM_MCP_TOOL_TIMEOUT_MS
        );
        assert!(kiro_agent["prompt"]
            .as_str()
            .unwrap()
            .contains("<role-system-prompt>"));
        assert!(!kiro_env.iter().any(|(key, _)| key == "MCP_TOOL_TIMEOUT"));

        let (claude_env, claude_cmd, claude_confirmation) = prepare_claude(
            "planner",
            "planner",
            "plan",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(claude_cmd.contains("--dangerously-skip-permissions"));
        assert!(claude_cmd.contains("--model sonnet"));
        assert!(claude_cmd.contains("--append-system-prompt"));
        assert!(claude_cmd.contains("<role-system-prompt>"));
        assert!(
            claude_cmd.ends_with(" kick"),
            "the positional startup message contains only the kick: {claude_cmd}"
        );
        let claude_settings: Value = serde_json::from_slice(
            &std::fs::read(paths.claude.join("planner.settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            claude_settings["skipDangerousModePermissionPrompt"],
            Value::Bool(true)
        );
        assert_eq!(
            claude_settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["Stop"][0]["hooks"][2]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        let claude_mcp: Value = serde_json::from_slice(
            &std::fs::read(paths.claude.join("planner.mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            claude_mcp["mcpServers"]["team"]["timeout"],
            TEAM_MCP_TOOL_TIMEOUT_MS
        );
        assert!(claude_env.contains(&(
            "MCP_TOOL_TIMEOUT".to_string(),
            TEAM_MCP_TOOL_TIMEOUT_MS.to_string()
        )));
        assert!(claude_env.contains(&(
            "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(),
            "0".to_string()
        )));
        let confirmation = claude_confirmation.unwrap();
        assert_eq!(confirmation.timeout, Duration::from_secs(120));
        assert!(startup_prompt_visible(
            "Accessing workspace:\nYes, I trust this folder\nEnter to confirm",
            &confirmation
        ));
        assert!(!startup_prompt_visible(
            "Claude is ready for input",
            &confirmation
        ));
        assert!(startup_already_ready(
            "bypass permissions on (shift+tab to cycle)",
            &confirmation
        ));
        assert!(folder_trust_prompt_visible(
            "Accessing workspace:\nYes, I trust this folder\nEnter to confirm"
        ));

        let (_, codex_cmd, codex_confirmation) = prepare_codex(
            "builder",
            "builder",
            "build",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(codex_cmd.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(codex_cmd.contains("--dangerously-bypass-hook-trust"));
        assert!(codex_cmd.contains("developer_instructions="));
        assert!(codex_cmd.contains("<tmux-mobile-team-instructions>"));
        assert!(codex_cmd.contains("<role-system-prompt>"));
        assert!(
            codex_cmd.ends_with(" kick"),
            "the positional startup message contains only the kick: {codex_cmd}"
        );
        assert!(!codex_cmd.contains("--model"));
        assert!(!codex_cmd.contains("--profile"));
        // Machines whose codex auth lives in a profile layer get --profile.
        let mut cfg_profiled = cfg.clone();
        cfg_profiled.codex_profile = "personal".into();
        let (_, profiled_cmd, _) = prepare_codex(
            "builder", "builder", "build", "", &cfg_profiled, "room", &paths, None, &extras,
        )
        .unwrap();
        assert!(profiled_cmd.contains("--profile personal"));
        let confirmation = codex_confirmation.unwrap();
        assert!(startup_prompt_visible(
            "Do you trust the contents of this directory?\n1. Yes, continue\nPress enter to continue",
            &confirmation
        ));
        assert!(!startup_prompt_visible(
            "Do you trust the contents of this directory?\n2. No, quit",
            &confirmation
        ));
        assert!(folder_trust_prompt_visible(
            "Do you trust the contents of this directory?\n1. Yes, continue\nPress enter to continue"
        ));
        assert!(!folder_trust_prompt_visible(
            "A tool needs permission.\n1. Yes, continue\nPress enter to continue"
        ));
        let codex_hooks: Value = serde_json::from_slice(
            &std::fs::read(paths.codex.join("builder/hooks.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            codex_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            codex_hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            codex_hooks["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn backend_model_selection_is_forwarded() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-backend-models-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::new(root.to_str().unwrap(), "backend-models");
        prepare_home(&paths).unwrap();
        let extras = Extras::default();
        let cfg = cfg();
        let model = "provider/model name;not-a-command";
        let expected = format!("--model {}", shell_quote(model));

        let (_, kiro_cmd, _) = prepare_kiro(
            "lead", "lead", "coordinate", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();
        let (_, claude_cmd, _) = prepare_claude(
            "planner", "planner", "plan", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();
        let (_, codex_cmd, _) = prepare_codex(
            "builder", "builder", "build", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();

        assert!(kiro_cmd.contains(&expected), "{kiro_cmd}");
        assert!(claude_cmd.contains(&expected), "{claude_cmd}");
        assert!(codex_cmd.contains(&expected), "{codex_cmd}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_value_remote_and_local_per_backend() {
        let remote = McpDef {
            name: "gh".into(),
            url: Some("https://x/mcp".into()),
            headers: [
                ("Authorization".to_string(), "Bearer $API_TOKEN".to_string()),
                ("X-Features".to_string(), "${FEATURES}".to_string()),
                ("X-Static".to_string(), "literal".to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        // kiro remote omits an explicit type; claude tags it http.
        assert!(kiro_mcp_value(&remote).get("type").is_none());
        assert_eq!(claude_mcp_value(&remote)["type"], "http");
        assert_eq!(kiro_mcp_value(&remote)["url"], "https://x/mcp");
        assert_eq!(
            kiro_mcp_value(&remote)["headers"]["Authorization"],
            "Bearer ${API_TOKEN}"
        );
        assert_eq!(
            claude_mcp_value(&remote)["headers"]["X-Features"],
            "${FEATURES}"
        );
        assert_eq!(kiro_mcp_value(&remote)["headers"]["X-Static"], "literal");

        let remote_overrides = codex_mcp_overrides(&remote).join(" ");
        assert!(remote_overrides.contains("mcp_servers.gh.bearer_token_env_var"));
        assert!(remote_overrides.contains("API_TOKEN"));
        assert!(remote_overrides.contains("mcp_servers.gh.env_http_headers.X-Features"));
        assert!(remote_overrides.contains("FEATURES"));
        assert!(remote_overrides.contains("mcp_servers.gh.http_headers.X-Static"));
        assert!(!remote_overrides.contains("Bearer $API_TOKEN"));

        let local = McpDef { name: "pg".into(), command: Some("mcp-pg".into()), args: vec!["--stdio".into()], ..Default::default() };
        let overrides = codex_mcp_overrides(&local).join(" ");
        assert!(overrides.contains("mcp_servers.pg.command"));
        assert!(overrides.contains("mcp-pg"));
        assert!(overrides.contains("mcp_servers.pg.args"));
        assert!(overrides.contains("--stdio"));
    }

    #[test]
    fn team_tool_timeout_exceeds_coalesced_wait_budget() {
        let team = McpDef {
            name: "team".into(),
            url: Some("http://127.0.0.1:8787/mcp".into()),
            ..Default::default()
        };
        let overrides = codex_team_mcp_overrides(&team).join(" ");
        let timeout_ms = team_mcp_tool_timeout_ms();
        let timeout_secs = timeout_ms / 1000;

        assert!(timeout_ms > agora::mcp::MCP_WAIT_MAX_MS);
        assert_eq!(timeout_ms - agora::mcp::MCP_WAIT_MAX_MS, 60_000);
        assert!(overrides.contains(&format!(
            "mcp_servers.team.tool_timeout_sec={timeout_secs}"
        )));
        assert_eq!(timeout_ms, TEAM_MCP_TOOL_TIMEOUT_MS);
    }

    // Records seed_employee calls so we can assert the default roster.
    use super::super::test_util::cfg;

    #[test]
    fn shell_quote_plain_passthrough() {
        assert_eq!(shell_quote("kiro-cli"), "kiro-cli");
        assert_eq!(shell_quote("a/b_c.d"), "a/b_c.d");
    }

    #[test]
    fn shell_quote_wraps_spaces_and_unicode() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("你是「经理」"), "'你是「经理」'");
    }

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn cli_skill_index_is_part_of_the_system_prompt() {
        let cfg = TeamConfig {
            url: String::new(),
            model: String::new(),
            system_prompt: String::new(),
            team_rules: "Shared rule.".into(),
            team_kick: "kick".into(),
            codex_profile: String::new(),
        };
        let skills = vec![ResolvedSkill {
            name: "review".into(),
            dir: PathBuf::from("/skills/review"),
            description: "Review changes".into(),
        }];

        let prompt = build_cli_system_prompt("reviewer", "Review.", "", &cfg, &skills);

        assert!(prompt.contains("<skills-system-prompt>"));
        assert!(prompt.contains("[review] Review changes (at /skills/review/SKILL.md)"));
        assert!(!prompt.contains("kick"));
    }

}
