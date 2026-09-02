//! Agent TEAMS (owner, 2026-09-02, board #74): "除了定制 agent 之外，我们可以
//! 定义 agent team，可以看作是 agent 加上一个特定的角色补充设定，组成一个小组".
//!
//! A team is a named list of members. Each member is ONE of:
//! - a registry agent plus a role supplement (`base` = the agent's name,
//!   `role` = what this member does in this team) — the common case: the
//!   owner's own agents, derived, not redefined;
//! - a team-only agent (`agent` = a full inline definition) — "我专门重新定义
//!   只有这个 team 才有的一个 agent".
//!
//! Spawning a team spawns every member as an ordinary managed agent (same
//! isolated home, same hooks, same `tmm`), so nothing downstream learns a new
//! species: the ONLY additions are the role block in the prompt, a roster
//! block naming every teammate (so they can `@` each other on day one), and a
//! `team` field in the launch recipe that lets the Hub group the cards.
//! The pure parts live here and are tested without tmux.

use serde::{Deserialize, Serialize};

use super::store::{RegAgent, RegTeam};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    /// The member's name in the team — becomes its window name (uniquified
    /// against the project if taken) and its `@name`.
    pub name: String,
    /// Registry agent this member derives from. Empty = `agent` carries the
    /// whole definition.
    #[serde(default)]
    pub base: String,
    /// Role supplement, appended to the base persona.
    #[serde(default)]
    pub role: String,
    /// Model / effort OVERRIDES for a derived member (owner, 2026-09-02: kiro
    /// offers many models with no quota, so one base agent can be the seed of
    /// four reviewers on four different models). Empty = the base's own.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
    /// Team-only definition (used when `base` is empty). Its `name` is ignored
    /// in favour of the member name.
    #[serde(default)]
    pub agent: Option<RegAgent>,
}

pub fn parse_members(team: &RegTeam) -> Result<Vec<Member>, String> {
    serde_json::from_str::<Vec<Member>>(&team.members).map_err(|e| format!("team '{}': members must be a JSON array: {e}", team.name))
}

/// Everything `teams_save` refuses. `known` = registry agent names; `cap` =
/// the project spawn cap, because a team that can never fully start is a
/// trap, not a configuration.
pub fn validate(team: &RegTeam, known: &[String], cap: usize) -> Result<Vec<Member>, String> {
    if team.name.trim().is_empty() {
        return Err("team name must not be empty".into());
    }
    let members = parse_members(team)?;
    if members.is_empty() {
        return Err("a team needs at least one member".into());
    }
    if members.len() > cap {
        return Err(format!("a team may have at most {cap} members (the project spawn cap), got {}", members.len()));
    }
    let mut seen = std::collections::HashSet::new();
    for m in &members {
        let n = m.name.trim();
        if n.is_empty() {
            return Err("every member needs a name".into());
        }
        if n.contains(char::is_whitespace) || n.contains('@') {
            return Err(format!("member name '{n}' must be one word without '@' — it becomes the window name and the @address"));
        }
        if !seen.insert(n.to_string()) {
            return Err(format!("member name '{n}' is used twice"));
        }
        if m.base.trim().is_empty() {
            let a = m.agent.as_ref().ok_or_else(|| format!("member '{n}' has neither a base agent nor an inline definition"))?;
            if !matches!(a.backend.as_str(), "kiro" | "claude" | "codex" | "grok") {
                return Err(format!("member '{n}': backend must be kiro|claude|codex|grok, got '{}'", a.backend));
            }
        } else if !known.iter().any(|k| k == m.base.trim()) {
            return Err(format!("member '{n}' derives from '{}', which is not in the registry", m.base));
        }
    }
    Ok(members)
}

/// The definition a member actually spawns with: the base (or inline) def,
/// renamed to the member, with the role and the team roster appended to its
/// persona. `roster` is every member's FINAL window name + role, in order —
/// computed by the caller after uniquifying, so the names in the prompt are
/// the names that exist.
pub fn effective_def(team: &RegTeam, member: &Member, base: Option<&RegAgent>, window_name: &str, roster: &[(String, String)]) -> Result<RegAgent, String> {
    let mut def = match (member.base.trim().is_empty(), base, &member.agent) {
        (false, Some(b), _) => b.clone(),
        (false, None, _) => return Err(format!("member '{}' derives from '{}', which is not in the registry", member.name, member.base)),
        (true, _, Some(a)) => a.clone(),
        (true, _, None) => return Err(format!("member '{}' has neither a base agent nor an inline definition", member.name)),
    };
    def.name = window_name.to_string();
    if !member.model.trim().is_empty() {
        def.model = member.model.trim().to_string();
    }
    if !member.effort.trim().is_empty() {
        def.effort = member.effort.trim().to_string();
    }
    let mut system = def.system.trim().to_string();
    let mut block = format!("## Your team: \"{}\"", team.name);
    if !team.description.trim().is_empty() {
        block += &format!("\n{}", team.description.trim());
    }
    block += &format!("\n\nYou are \"{window_name}\".");
    if !member.role.trim().is_empty() {
        block += &format!(" Your role: {}", member.role.trim());
    }
    let mates: Vec<String> = roster
        .iter()
        .filter(|(n, _)| n != window_name)
        .map(|(n, r)| if r.trim().is_empty() { format!("@{n}") } else { format!("@{n} — {}", one_line(r)) })
        .collect();
    if !mates.is_empty() {
        block += "\nYour teammates (address one with `tmm send \"@name …\"`):\n";
        for m in mates {
            block += &format!("- {m}\n");
        }
    }
    if !system.is_empty() {
        system += "\n\n";
    }
    system += block.trim_end();
    def.system = system;
    Ok(def)
}

/// A role's first sentence-ish, for the roster line: one line, ≤120 chars.
fn one_line(role: &str) -> String {
    let flat = role.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        let cut: String = flat.chars().take(119).collect();
        format!("{}…", cut.trim_end())
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, system: &str) -> RegAgent {
        RegAgent { name: name.into(), backend: "claude".into(), model: String::new(), effort: String::new(), system: system.into(), skills: "[]".into(), mcp: "[]".into(), can_hire: false }
    }
    fn team(members: &str) -> RegTeam {
        RegTeam { name: "dev-squad".into(), description: "Ships features end to end.".into(), members: members.into() }
    }

    #[test]
    fn validate_pins_the_shape() {
        let known = vec!["claude".to_string(), "codex".to_string()];
        let ok = team(r#"[{"name":"dev","base":"claude","role":"implement"},{"name":"rev","base":"codex","role":"review"}]"#);
        assert_eq!(validate(&ok, &known, 4).unwrap().len(), 2);
        assert!(validate(&team("[]"), &known, 4).unwrap_err().contains("at least one"));
        assert!(validate(&team(r#"[{"name":"a","base":"nope"}]"#), &known, 4).unwrap_err().contains("not in the registry"));
        assert!(validate(&team(r#"[{"name":"a","base":"claude"},{"name":"a","base":"codex"}]"#), &known, 4).unwrap_err().contains("used twice"));
        assert!(validate(&team(r#"[{"name":"two words","base":"claude"}]"#), &known, 4).unwrap_err().contains("one word"));
        assert!(validate(&team(r#"[{"name":"a","base":"claude"},{"name":"b","base":"claude"},{"name":"c","base":"claude"}]"#), &known, 2).unwrap_err().contains("at most 2"));
        // inline, team-only member: needs a real backend
        assert!(validate(&team(r#"[{"name":"x","agent":{"name":"","backend":"gpt"}}]"#), &known, 4).unwrap_err().contains("backend must be"));
        assert_eq!(validate(&team(r#"[{"name":"x","agent":{"name":"","backend":"grok"}}]"#), &known, 4).unwrap().len(), 1);
        assert!(validate(&team(r#"[{"name":"x"}]"#), &known, 4).unwrap_err().contains("neither"));
    }

    #[test]
    fn effective_def_derives_renames_and_names_the_teammates() {
        let t = team(r#"[{"name":"dev","base":"claude","role":"implement the change"},{"name":"rev","base":"claude","role":"review every diff before it merges"}]"#);
        let members = parse_members(&t).unwrap();
        let base = agent("claude", "You are a 10x developer.");
        let roster = vec![("dev".to_string(), "implement the change".to_string()), ("rev-2".to_string(), "review every diff before it merges".to_string())];
        let d = effective_def(&t, &members[0], Some(&base), "dev", &roster).unwrap();
        assert_eq!(d.name, "dev", "the member name is the identity, not the base's");
        assert_eq!(d.backend, "claude", "everything else is inherited");
        assert!(d.system.starts_with("You are a 10x developer."), "base persona first");
        assert!(d.system.contains("## Your team: \"dev-squad\""));
        assert!(d.system.contains("Ships features end to end."));
        assert!(d.system.contains("You are \"dev\". Your role: implement the change"));
        assert!(d.system.contains("- @rev-2 — review every diff"), "the roster uses the FINAL (uniquified) names");
        assert!(!d.system.contains("- @dev"), "never lists itself");
    }

    #[test]
    fn effective_def_inline_member_ignores_the_inline_name() {
        let t = team(r#"[{"name":"critic","role":"find holes","agent":{"name":"whatever","backend":"grok","system":"Be harsh."}}]"#);
        let members = parse_members(&t).unwrap();
        let d = effective_def(&t, &members[0], None, "critic", &[("critic".into(), "find holes".into())]).unwrap();
        assert_eq!(d.name, "critic");
        assert_eq!(d.backend, "grok");
        assert!(d.system.starts_with("Be harsh."));
        assert!(!d.system.contains("teammates"), "a one-member team has no roster block");
        assert!(effective_def(&t, &Member { name: "x".into(), base: "gone".into(), role: String::new(), model: String::new(), effort: String::new(), agent: None }, None, "x", &[]).is_err());
    }

    #[test]
    fn effective_def_applies_model_and_effort_overrides() {
        let t = team(r#"[{"name":"a","base":"kiro","model":"claude-opus-5","effort":"high"},{"name":"b","base":"kiro"}]"#);
        let members = parse_members(&t).unwrap();
        let mut base = agent("kiro", "");
        base.backend = "kiro".into();
        base.model = "auto".into();
        let a = effective_def(&t, &members[0], Some(&base), "a", &[]).unwrap();
        assert_eq!((a.model.as_str(), a.effort.as_str()), ("claude-opus-5", "high"), "an override replaces the base's");
        let b = effective_def(&t, &members[1], Some(&base), "b", &[]).unwrap();
        assert_eq!((b.model.as_str(), b.effort.as_str()), ("auto", ""), "empty keeps the base's");
    }
}
