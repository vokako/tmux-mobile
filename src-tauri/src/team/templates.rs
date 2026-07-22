//! Team templates: the built-in rosters and the on-disk template store
//! (`<config>/tmux-mobile/teams/<name>/team.yaml`). Split from team.rs
//! 2026-07-22 — content unchanged.

use std::path::PathBuf;
use serde_json::Value;

// ─── Team templates (named rosters under <config>/tmux-mobile/teams/) ──────
// A template is a JSON file `teams/<name>.json` = { "agents": [ {name, backend,
// role, goal, model, manage}, … ] }. The user edits these from the
// app (Templates panel); `start_team` seeds the chosen template into the room.
// The built-in default is written to teams/default.json on first run so there
// is always something to edit.

/// Default model placeholder substituted in when a kiro agent leaves model empty.
pub const BUILTIN_TEMPLATE: &str = include_str!("../../../team/templates/default/team.yaml");

/// A ready-made software-development roster (tech-lead / product / architect /
/// coder / reviewer / tester), seeded alongside the default so it appears in
/// the app's template picker out of the box. The whole collaboration workflow
/// lives in each agent's `goal` (role isolation) — team-brief.md stays a
/// role-agnostic, workflow-free communication contract.
pub const SOFTWARE_DEV_TEMPLATE: &str = include_str!("../../../team/templates/software-dev/team.yaml");

/// A financial-research roster modeled on Dexter (virattt/dexter): a research
/// director plus fundamentals / market+sentiment / valuation(DCF) / memo /
/// reviewer analysts. Dexter's single-agent skills (DCF, investment memo, X
/// sentiment) become specialist roles; its data discipline (figures carry
/// sources, the deliverable is a file, chat is a scannable header) and its
/// educational-only / not-investment-advice posture are baked into the goals.
pub const FINANCIAL_RESEARCH_TEMPLATE: &str =
    include_str!("../../../team/templates/financial-research/team.yaml");

/// A deep-research roster: a director who decomposes the question, two parallel
/// researchers, a synthesist, and a skeptic — every claim sourced, output to
/// report.md.
pub const DEEP_RESEARCH_TEMPLATE: &str = include_str!("../../../team/templates/deep-research/team.yaml");

/// A content-studio roster (editor-in-chief / researcher / writer / copy editor)
/// for shipping a publish-ready article or docs in a shared house style.
pub const CONTENT_STUDIO_TEMPLATE: &str = include_str!("../../../team/templates/content-studio/team.yaml");

/// A data-analysis roster (lead / data engineer / analyst / reporter) that
/// answers a question from data with reproducible work and honest caveats.
pub const DATA_ANALYSIS_TEMPLATE: &str = include_str!("../../../team/templates/data-analysis/team.yaml");

/// A lean mixed-backend engineering roster: Kiro coordinates requirements and
/// delivery, Claude designs and reviews, and Codex implements and verifies.
pub const MIXED_ENGINEERING_TEMPLATE: &str =
    include_str!("../../../team/templates/mixed-engineering/team.yaml");

/// Built-in templates seeded into teams/ on first run: (file stem, contents).
const BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    ("default", BUILTIN_TEMPLATE),
    ("software-dev", SOFTWARE_DEV_TEMPLATE),
    ("financial-research", FINANCIAL_RESEARCH_TEMPLATE),
    ("deep-research", DEEP_RESEARCH_TEMPLATE),
    ("content-studio", CONTENT_STUDIO_TEMPLATE),
    ("data-analysis", DATA_ANALYSIS_TEMPLATE),
    ("mixed-engineering", MIXED_ENGINEERING_TEMPLATE),
];

/// The teams/ template directory.
fn templates_dir() -> PathBuf {
    crate::config::config_dir().join("teams")
}

/// Sanitize a template name to a safe single path segment (no escaping the dir).
fn sanitize_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if safe.trim_matches('-').is_empty() { "default".to_string() } else { safe }
}

/// A template now lives in its OWN folder `teams/<name>/`, holding `team.yaml`
/// (the roster + per-agent env/mcp/skills) and optionally a `skills/` dir of
/// local skills bundled with the team. The folder is the unit so a team can
/// carry its own assets.
pub(super) fn team_dir(name: &str) -> PathBuf {
    templates_dir().join(sanitize_name(name))
}

fn template_yaml_path(name: &str) -> PathBuf {
    team_dir(name).join("team.yaml")
}

/// One-time migration of the old flat `teams/<name>.json` files into the new
/// `teams/<name>/team.yaml` folder layout. The legacy file is renamed to
/// `<name>.json.bak` (kept, not deleted) so the move is reversible.
fn migrate_legacy_json() {
    let dir = templates_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        let yaml_path = team_dir(stem).join("team.yaml");
        if yaml_path.exists() {
            continue; // already migrated
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(val) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(parent) = yaml_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(yaml) = serde_yml::to_string(&val) {
            if std::fs::write(&yaml_path, yaml).is_ok() {
                let _ = std::fs::rename(&p, p.with_extension("json.bak"));
                println!("🜂 team: migrated legacy template '{}' → team.yaml", stem);
            }
        }
    }
}

/// Ensure the teams/ dir exists and holds the built-in templates. Migrates any
/// legacy `*.json` first, then seed-once per folder: an existing template is
/// never overwritten, so a user's edits (and custom templates) survive restarts.
pub fn ensure_templates_seeded() {
    let dir = templates_dir();
    let _ = std::fs::create_dir_all(&dir);
    migrate_legacy_json();
    for (name, body) in BUILTIN_TEMPLATES {
        let path = template_yaml_path(name);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, body);
        }
    }
}

/// List available template names (folders in teams/ that hold a team.yaml).
pub fn list_templates() -> Vec<String> {
    ensure_templates_seeded();
    let mut names: Vec<String> = std::fs::read_dir(templates_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.is_dir() && p.join("team.yaml").is_file() {
                        p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Read a template's full definition object (`{ env?, agents }`) from YAML, or
/// `null` if missing/bad.
pub fn read_team_def(name: &str) -> Value {
    std::fs::read_to_string(template_yaml_path(name))
        .ok()
        .and_then(|s| serde_yml::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null)
}

/// Read a template's agent list (the `agents` array), or empty if missing/bad.
pub fn read_template(name: &str) -> Vec<Value> {
    read_team_def(name)
        .get("agents")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Read every template as `{ name, env, mcp, skills, prompt, agents }` for the
/// editor panel (team-wide fields + the roster).
pub fn read_all_templates() -> Vec<Value> {
    list_templates()
        .into_iter()
        .map(|name| {
            let def = read_team_def(&name);
            serde_json::json!({
                "name": name,
                "env": def.get("env").cloned().unwrap_or(serde_json::json!({})),
                "mcp": def.get("mcp").cloned().unwrap_or(serde_json::json!([])),
                "skills": def.get("skills").cloned().unwrap_or(serde_json::json!([])),
                "prompt": def.get("prompt").cloned().unwrap_or(serde_json::json!("")),
                "agents": def.get("agents").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect()
}

/// Write a template from the full definition object `{ env?, mcp?, skills?,
/// prompt?, agents }`. Empty team-wide fields are pruned so the YAML stays tidy.
pub fn save_template(name: &str, def: &Value) -> Result<(), String> {
    ensure_templates_seeded();
    // Accept either a full def object or a bare agents array (legacy callers).
    let def = if def.is_array() {
        serde_json::json!({ "agents": def })
    } else {
        def.clone()
    };
    let mut out = serde_json::Map::new();
    if let Some(env) = def.get("env").and_then(|v| v.as_object()) {
        if !env.is_empty() { out.insert("env".into(), Value::Object(env.clone())); }
    }
    if let Some(mcp) = def.get("mcp").and_then(|v| v.as_array()) {
        if !mcp.is_empty() { out.insert("mcp".into(), Value::Array(mcp.clone())); }
    }
    if let Some(sk) = def.get("skills").and_then(|v| v.as_array()) {
        if !sk.is_empty() { out.insert("skills".into(), Value::Array(sk.clone())); }
    }
    if let Some(p) = def.get("prompt").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() { out.insert("prompt".into(), Value::String(p.to_string())); }
    }
    out.insert("agents".into(), def.get("agents").cloned().unwrap_or(serde_json::json!([])));
    let yaml = serde_yml::to_string(&Value::Object(out)).map_err(|e| e.to_string())?;
    let path = template_yaml_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, yaml).map_err(|e| e.to_string())
}

/// Delete a template folder (the built-in default is protected).
pub fn delete_template(name: &str) -> Result<(), String> {
    if name == "default" {
        return Err("the default template cannot be deleted".into());
    }
    std::fs::remove_dir_all(team_dir(name)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_default_template_is_minimal_manager_worker() {
        // The default template is the minimal demo: a manager + one worker that
        // shows the delegate→report loop and can grow via the manager's hire().
        let v: Value = serde_yml::from_str(BUILTIN_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2, "minimal demo = manager + worker");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"manager") && names.contains(&"worker"));
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 1, "exactly one manager");
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");
    }

    #[test]
    fn software_dev_template_roster_and_tools() {
        // The software-dev roster is a built-in (teams/software-dev/team.yaml).
        let v: Value = serde_yml::from_str(SOFTWARE_DEV_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 8, "manager+product+architect+frontend+backend+reviewer+tester+devops");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        for expected in ["manager", "product", "architect", "frontend", "backend", "reviewer", "tester", "devops"] {
            assert!(names.contains(&expected), "missing role '{expected}': {names:?}");
        }
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 0, "hire/fire off: no manage=true agent");
        // Workflow lives in each role's goal (team-brief.md is contract-only), so
        // every agent must carry a substantive goal.
        assert!(
            agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
            "each role's goal must carry its slice of the workflow"
        );
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");

        // Per-agent tools wired via the new schema.
        let agent = |n: &str| agents.iter().find(|a| a["name"] == n).unwrap();
        let mcp_names = |n: &str| -> Vec<String> {
            agent(n)["mcp"].as_array().map(|a| {
                a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()
            }).unwrap_or_default()
        };
        for dev in ["architect", "frontend", "backend", "reviewer", "tester", "devops"] {
            assert!(mcp_names(dev).contains(&"context7".to_string()), "{dev} should have context7");
        }
        // architect/backend/reviewer/devops reach the AWS knowledge base.
        for n in ["architect", "backend", "reviewer", "devops"] {
            assert!(mcp_names(n).contains(&"aws-knowledge".to_string()), "{n} has AWS knowledge");
        }
        assert!(mcp_names("tester").contains(&"chrome-devtools".to_string()), "tester has chrome-devtools for e2e");
        let fe_skills = agent("frontend")["skills"].as_array().unwrap();
        assert!(
            fe_skills.iter().any(|s| s.as_str().map(|x| x.contains("frontend-design")).unwrap_or(false)),
            "frontend has the frontend-design skill"
        );
    }

    #[test]
    fn builtin_templates_are_seeded() {
        // Isolate config dir so we don't touch the real ~/.config.
        let dir = std::env::temp_dir().join(format!("teamtest-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        ensure_templates_seeded();
        let mut names = list_templates();
        names.sort();
        for expected in [
            "default",
            "software-dev",
            "financial-research",
            "deep-research",
            "content-studio",
            "data-analysis",
            "mixed-engineering",
        ] {
            assert!(names.contains(&expected.to_string()), "{expected} seeded: {names:?}");
        }

        // Each built-in parses and every agent carries a substantive goal.
        for (name, body) in BUILTIN_TEMPLATES {
            let v: Value = serde_yml::from_str(body).unwrap_or_else(|e| panic!("{name} bad yaml: {e}"));
            let agents = v["agents"].as_array().unwrap_or_else(|| panic!("{name} has no agents"));
            assert!(!agents.is_empty(), "{name} empty roster");
            assert!(
                agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
                "{name}: every role needs a substantive goal"
            );
        }

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mixed_engineering_template_uses_all_backends_with_explicit_handoffs() {
        let v: Value = serde_yml::from_str(MIXED_ENGINEERING_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 3, "keep the mixed team lean");

        for backend in ["kiro", "claude", "codex"] {
            assert_eq!(
                agents.iter().filter(|agent| agent["backend"] == backend).count(),
                1,
                "expected exactly one {backend} agent"
            );
        }
        let agent = |name: &str| agents.iter().find(|agent| agent["name"] == name).unwrap();
        assert_eq!(agent("lead")["backend"], "kiro");
        assert_eq!(agent("architect")["backend"], "claude");
        assert_eq!(agent("builder")["backend"], "codex");
        assert!(agents.iter().all(|agent| agent["model"] == ""));
        assert!(agents.iter().all(|agent| agent["manage"] == false));

        let prompt = v["prompt"].as_str().unwrap();
        for handoff in ["@lead", "@architect", "@builder", "verification", "review"] {
            assert!(prompt.contains(handoff), "missing workflow handoff: {handoff}");
        }
    }

    #[test]
    fn financial_research_template_has_lead_and_default_models() {
        let v: Value = serde_yml::from_str(FINANCIAL_RESEARCH_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert!(agents.len() >= 5, "a multi-analyst research team");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        for expected in ["lead", "fundamentals", "valuation", "memo", "reviewer"] {
            assert!(names.contains(&expected), "missing role '{expected}': {names:?}");
        }
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 0, "hire/fire off: no manage=true agent");
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");
        assert!(
            agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
            "each role's goal must carry its methodology"
        );
    }
}
