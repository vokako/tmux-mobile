//! Declarative projects: a workspace you can close and reopen.
//!
//! A project is a directory plus the windows it is made of. The declaration
//! lives in `state.db`; the tmux session is a disposable projection of it. See
//! `docs/exec-plans/projects-and-tasks.md` for the product design and
//! `docs/design-docs/features/projects.md` for what is implemented.
//!
//! Desktop-only, like the team supervisor: the phone is a client of a desktop
//! server, so nothing here would ever run on Android/iOS.

pub mod agents;
pub mod capture;
pub mod reconcile;
pub mod spawn;
pub mod store;
pub mod telemetry;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use store::{Project, Slot, Store};

/// How often the capturer folds live tmux state back into the declaration.
const CAPTURE_INTERVAL: Duration = Duration::from_secs(20);

/// Sessions we never adopt: Team creates and kills its own
/// (`tmm-team-<team-id>`) sessions, so a project declaration would fight it.
/// See `docs/design-docs/features/team.md`.
const TEAM_SESSION_PREFIX: &str = "tmm-team-";

/// How long a tmux session must have existed before it becomes a project on its
/// own. Same reasoning as `capture::SETTLE_SECS` one level up: a workspace is
/// something you come back to, a two-minute shell is not.
pub const SESSION_SETTLE_SECS: u64 = 120;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `state.db` next to the rest of the app's state. `TMM_STATE_DB` overrides it,
/// which is how the tests get an isolated database (and how a second profile on
/// one machine could get its own).
fn db_path() -> PathBuf {
    match std::env::var_os("TMM_STATE_DB") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => crate::config::config_dir().join("state.db"),
    }
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// Where agent conversation ids come from, installed once by the server (the
/// notification hub). Unset in tests and on a server without hooks, in which
/// case restored agents fall back to a directory-scoped resume.
static SESSIONS: OnceLock<std::sync::Arc<dyn capture::AgentSessions + Send + Sync>> =
    OnceLock::new();

pub fn set_agent_sessions(sessions: std::sync::Arc<dyn capture::AgentSessions + Send + Sync>) {
    let _ = SESSIONS.set(sessions);
}

fn agent_sessions() -> &'static (dyn capture::AgentSessions + Send + Sync) {
    match SESSIONS.get() {
        Some(s) => s.as_ref(),
        None => &capture::NoSessions,
    }
}

/// Run `f` against the process-wide store, opening it on first use. Errors are
/// returned rather than panicking so a broken database degrades to "the
/// Projects page is unavailable" instead of taking the server down.
fn with_store<T>(f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let cell = match STORE.get() {
        Some(c) => c,
        None => {
            let store = Store::open(&db_path())?;
            let _ = STORE.set(Mutex::new(store));
            STORE.get().ok_or("state.db unavailable")?
        }
    };
    let mut guard = cell.lock().map_err(|_| "state.db lock poisoned".to_string())?;
    f(&mut guard)
}

// ---- ids and names ------------------------------------------------------

/// Deterministic short digest (FNV-1a) so a path always maps to the same id.
fn digest(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:06x}", hash & 0xff_ffff)
}

/// tmux-safe, readable component of a name: no dots or colons (tmux target
/// syntax), no spaces, bounded length.
fn slug(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    let bounded: String = trimmed.chars().take(24).collect();
    if bounded.is_empty() {
        "project".to_string()
    } else {
        bounded.to_lowercase()
    }
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

fn canonical(path: &str) -> Result<String, String> {
    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{}", tmux::home_dir(), rest)
    } else {
        path.to_string()
    };
    let p = std::fs::canonicalize(&expanded).map_err(|e| format!("{expanded}: {e}"))?;
    if !p.is_dir() {
        return Err(format!("{} is not a directory", p.display()));
    }
    Ok(p.to_string_lossy().to_string())
}

use crate::tmux;

// ---- public API used by the RPC layer -----------------------------------

/// Every project, each with its slots and whether its session is live.
///
/// The client derives "untracked sessions" by subtracting these from
/// `list_sessions`, so there is exactly one place that decides what a session
/// is: tracked sessions live in the Projects section, everything else stays in
/// the session list.
pub fn list(include_archived: bool) -> Result<Value, String> {
    let projects = with_store(|store| {
        let projects = store.list_projects(include_archived)?;
        let mut out = Vec::with_capacity(projects.len());
        for p in &projects {
            let slots = store.slots(&p.id)?;
            let live = tmux::session_exists(&p.session);
            out.push(json!({
                "project": p,
                "slots": slots,
                "live": live,
            }));
        }
        Ok(out)
    })?;
    Ok(json!({ "projects": projects }))
}

/// Create a project for a directory. Idempotent by SESSION, not by path:
/// identity is the tmux session (several projects parked in the same
/// directory — typically `$HOME` — are separate workspaces), so only the
/// literal same request (same wanted session AND same canonical path) returns
/// the existing project (un-archived). A new project at a path some other
/// project already uses is a NEW project — merging on path silently swallowed
/// it (owner report, 2026-08-19).
///
/// `session` is the tmux session name the user asked for (falling back to the
/// directory basename); `agent` seeds the workspace with one agent window, which
/// is how the create form's Kiro/Claude presets survive the move from
/// "new session" to "new project".
pub fn create(
    path: &str,
    name: Option<&str>,
    session: Option<&str>,
    agent: Option<&str>,
) -> Result<Value, String> {
    let path = canonical(path)?;
    let label = name
        .or(session)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| basename(&path))
        .to_string();
    let ts = now();
    with_store(|store| {
        // The session follows the NAME when one was given: a project called
        // "closetest" living in /tmp must not become the session "tmp" (owner
        // report — the same folder-name-wins bug the Hub dialog hit). An
        // explicit `session` still overrides; basename is the last resort.
        let wanted = session
            .filter(|s| !s.trim().is_empty())
            .or(name.filter(|s| !s.trim().is_empty()))
            .map(slug)
            .unwrap_or_else(|| slug(basename(&path)));
        // Same session + same directory IS the same request: return the row
        // instead of a duplicate. A session-name clash with a DIFFERENT
        // directory falls through — free_session_name suffixes the new one.
        if let Some(existing) = store.project_by_session(&wanted)? {
            if existing.path == path {
                if existing.archived {
                    store.set_archived(&existing.id, false, ts)?;
                }
                return Ok(json!(existing));
            }
        }
        // Two projects may now legitimately share label + path, so the id
        // must be salted past a collision rather than assumed unique.
        let mut id = format!("{}-{}", slug(&label), digest(&path));
        let mut salt: u32 = 1;
        while store.project(&id)?.is_some() {
            salt += 1;
            id = format!("{}-{}", slug(&label), digest(&format!("{path}#{salt}")));
        }
        let session = free_session_name(store, &wanted, &id)?;
        let project = Project {
            id: id.clone(),
            name: label,
            path,
            icon: None,
            session,
            adopted: false,
            autostart: false,
            created_at: ts,
            last_up_at: None,
            last_seen_at: None,
            archived: false,
        };
        store.insert_project(&project)?;
        // A seeded agent slot is settled straight away: the user asked for that
        // agent, so `up` must create its window on the first run instead of
        // waiting for the capturer to notice it.
        if let Some(backend) = agent.filter(|b| agents::launch_for(b).is_some()) {
            store.replace_slots(
                &id,
                &[Slot {
                    id: None,
                    ord: 0,
                    window_name: backend.to_string(),
                    cwd: String::new(),
                    kind: store::SlotKind::Agent,
                    command: Some(backend.to_string()),
                    auto_run: true,
                    agent_session_id: None,
                    first_seen_at: ts,
                    settled_at: Some(ts),
                }],
            )?;
        }
        Ok(json!(project))
    })
}

/// Adopt a live tmux session: the user created it, so we keep its name and take
/// its current windows as the declaration straight away — an adopted project
/// must be restorable even if the machine reboots one minute later.
///
/// Identity is the SESSION, not the directory: several sessions parked in the
/// same directory (typically `$HOME`) are still separate workspaces.
pub fn adopt(session: &str, name: Option<&str>) -> Result<Value, String> {
    if !tmux::session_exists(session) {
        return Err(format!("no such tmux session: {session}"));
    }
    let ts = now();
    with_store(|store| adopt_in(store, session, name, ts))
}

fn adopt_in(
    store: &mut Store,
    session: &str,
    name: Option<&str>,
    ts: u64,
) -> Result<Value, String> {
    let path = canonical(&session_workspace(session)?)?;
    if let Some(existing) = store.project_by_session(session)? {
        return Err(format!(
            "session {session} is already tracked as project '{}'",
            existing.name
        ));
    }
    let id = format!("{}-{}", slug(session), digest(session));
    let project = Project {
        id: id.clone(),
        name: name.unwrap_or(session).to_string(),
        path: path.clone(),
        icon: None,
        session: session.to_string(),
        adopted: true,
        autostart: false,
        created_at: ts,
        last_up_at: Some(ts),
        last_seen_at: Some(ts),
        archived: false,
    };
    store.insert_project(&project)?;
    let observed = capture::observe(session, &path, agent_sessions())?;
    let merged = capture::merge(&[], &observed, ts, capture::SETTLE_SECS);
    // Settle immediately: these windows already exist and are the reason
    // the user is adopting the session.
    let slots: Vec<Slot> = merged
        .slots
        .into_iter()
        .map(|mut s| {
            s.settled_at = Some(ts);
            s
        })
        .collect();
    store.replace_slots(&id, &slots)?;
    Ok(json!({ "project": project, "slots": slots }))
}

/// Adopt every tmux session that isn't a project yet, so a session made outside
/// the app (`tmux new -s foo`) still survives a reboot. This is also the
/// migration path: on first run it picks up everything that already existed.
///
/// Three guards keep it from becoming a new source of entropy:
///
/// * a session must have existed for `SESSION_SETTLE_SECS` — a `tmux new` for a
///   30-second job must not leave a permanent declaration behind;
/// * team sessions are never adopted (Team owns their lifecycle);
/// * a session whose project was ARCHIVED is never re-adopted, or "remove from
///   projects" would undo itself on the next tick.
pub fn auto_adopt_once() -> Result<Vec<String>, String> {
    auto_adopt_with(&tmux::session_created_times(), now())
}

/// The decision half of `auto_adopt_once`, taking the session ages as data so
/// the guards are testable without waiting two minutes.
fn auto_adopt_with(created: &[(String, u64)], ts: u64) -> Result<Vec<String>, String> {
    with_store(|store| {
        let known: Vec<String> = store
            .list_projects(true)?
            .into_iter()
            .map(|p| p.session)
            .collect();
        let mut adopted = Vec::new();
        for (session, created_at) in created {
            if session.starts_with(TEAM_SESSION_PREFIX)
                || known.contains(session)
                || ts.saturating_sub(*created_at) < SESSION_SETTLE_SECS
            {
                continue;
            }
            match adopt_in(store, session, None, ts) {
                Ok(_) => adopted.push(session.clone()),
                Err(e) => eprintln!("projects: cannot track session {session}: {e}"),
            }
        }
        Ok(adopted)
    })
}

pub fn up(id: &str) -> Result<Value, String> {
    let (project, slots) = load(id)?;
    let report = reconcile::up(&project, &slots)?;
    with_store(|store| store.mark_up(id, now()))?;
    Ok(json!(report))
}

pub fn down(id: &str) -> Result<Value, String> {
    let (project, _) = load(id)?;
    reconcile::down(&project)?;
    Ok(json!({ "session": project.session, "live": false }))
}

pub fn set_archived(id: &str, archived: bool) -> Result<Value, String> {
    with_store(|store| {
        store.set_archived(id, archived, now())?;
        Ok(json!({ "id": id, "archived": archived }))
    })
}

/// Delete a project for good: kill its session, remove every managed agent's
/// isolated home, then forget the row (slots cascade). Archive is the
/// reversible verb — "hide this from the list, I might come back"; this one is
/// for a project that should stop existing (owner: "除了关闭之外，还要可以删除").
///
/// What it does NOT touch: the workspace directory and anything in it that is
/// not ours. We delete `<path>/.tmm/agents/<name>/` — configs and launch
/// recipes this app wrote — and never the user's files. The chat room is kept
/// too: it is the record of what happened, and rooms are addressed by session
/// name, so a later project with the same name inherits its history rather
/// than losing it.
pub fn delete(id: &str) -> Result<Value, String> {
    let project = with_store(|store| store.project(id))?
        .ok_or_else(|| format!("no project with id '{id}'"))?;
    // Down first: killing the session while its declaration still exists is
    // what `down` is for, and it keeps the reconciler from re-creating windows
    // for a project that is about to vanish.
    let _ = down(id);
    let mut homes_removed = 0usize;
    let agents_root = std::path::Path::new(&project.path).join(".tmm").join("agents");
    if let Ok(entries) = std::fs::read_dir(&agents_root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() && std::fs::remove_dir_all(entry.path()).is_ok() {
                homes_removed += 1;
            }
        }
    }
    let deleted = with_store(|store| store.delete_project(id))?;
    Ok(json!({ "id": id, "deleted": deleted, "agent_homes_removed": homes_removed }))
}

/// Remove ONE agent from a project: kill its window if it is running, drop its
/// slot so `up` never recreates it, and delete its isolated home so it stops
/// counting as "an agent this app created". Stop is the pause button; this is
/// the eject button (owner: "stop 以外，也可以删除 Agent").
pub fn agent_remove(session: &str, agent: &str) -> Result<Value, String> {
    let project = project_for_session(session)?
        .ok_or_else(|| format!("no project for session '{session}'"))?;
    let home = managed_home(session, agent)
        .ok_or_else(|| format!("'{agent}' is not an agent this app started"))?;
    // Kill the window first, or the capture loop would re-add the slot we are
    // about to delete from a window that is still alive.
    if let Ok(panes) = crate::tmux::list_panes(session) {
        if let Some(p) = panes.iter().find(|p| p.window_name == agent) {
            let _ = crate::tmux::kill_window(&format!("{session}:{}", p.window));
        }
    }
    let slot_removed = with_store(|store| store.delete_slot(&project.id, agent))?;
    let home_removed = std::fs::remove_dir_all(&home).is_ok();
    Ok(json!({
        "session": session,
        "agent": agent,
        "slot_removed": slot_removed,
        "home_removed": home_removed,
    }))
}

pub fn set_autostart(id: &str, autostart: bool) -> Result<Value, String> {
    with_store(|store| {
        store.set_autostart(id, autostart)?;
        Ok(json!({ "id": id, "autostart": autostart }))
    })
}

// ---- agent registry (agents-v2) ----------------------------------------

pub fn registry_list() -> Result<Value, String> {
    with_store(|store| {
        store.reg_seed(now())?;
        let agents = store.reg_list()?;
        Ok(json!({ "agents": agents }))
    })
}

pub fn registry_save(def: &Value) -> Result<Value, String> {
    let agent: store::RegAgent =
        serde_json::from_value(def.clone()).map_err(|e| format!("invalid agent def: {e}"))?;
    if agent.name.trim().is_empty() {
        return Err("agent name must not be empty".into());
    }
    if !matches!(agent.backend.as_str(), "kiro" | "claude" | "codex") {
        return Err(format!("backend must be kiro|claude|codex, got '{}'", agent.backend));
    }
    // Validate the JSON columns now, not at spawn time.
    serde_json::from_str::<Vec<String>>(&agent.skills).map_err(|e| format!("skills must be a JSON array of refs: {e}"))?;
    serde_json::from_str::<Vec<Value>>(&agent.mcp).map_err(|e| format!("mcp must be a JSON array of defs: {e}"))?;
    with_store(|store| {
        store.reg_save(&agent, now())?;
        Ok(json!({ "ok": true, "name": agent.name }))
    })
}

pub fn registry_delete(name: &str) -> Result<Value, String> {
    with_store(|store| {
        let deleted = store.reg_delete(name)?;
        Ok(json!({ "ok": deleted }))
    })
}

// ---- central skills / MCP assets ----------------------------------------

pub fn skills_list() -> Result<Value, String> {
    with_store(|store| Ok(json!({ "skills": store.skills_list()? })))
}

/// The app-OWNED skills storage: `<state dir>/skills/<name>/`. Lives beside
/// state.db so the TMM_STATE_DB test override isolates it too. Agents load
/// from HERE; the recorded source is sync metadata.
pub fn managed_skills_dir() -> std::path::PathBuf {
    db_path().parent().map(|p| p.join("skills")).unwrap_or_else(|| "skills".into())
}

/// Copy the resolved source directory into the managed store (atomic: build
/// a temp sibling, then swap).
fn sync_skill_files(name: &str, source: &str) -> Result<(), String> {
    let source = source.trim();
    if source.starts_with("http://") || source.starts_with("https://") {
        // A refresh must see the remote's CURRENT state, not the clone cache.
        crate::team::skills::invalidate_git_cache(source);
    } else if !std::path::Path::new(source).is_absolute() {
        return Err("local source must be an absolute path".into());
    }
    let resolved = crate::team::skills::resolve_skills(&[source.to_string()], "");
    let src_dir = resolved
        .first()
        .map(|r| r.dir.clone())
        .ok_or_else(|| format!("source did not resolve to a skill directory: {source}"))?;
    if !src_dir.join("SKILL.md").is_file() {
        return Err(format!("no SKILL.md in {}", src_dir.display()));
    }
    let root = managed_skills_dir();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let tmp = root.join(format!(".tmp-{name}"));
    let dest = root.join(name);
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&src_dir, &tmp)?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&tmp, &dest).map_err(|e| format!("swap into place: {e}"))
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let dest = to.join(entry.file_name());
        if ty.is_dir() {
            // .git in a copied local repo would be dead weight in the store.
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir(&entry.path(), &dest)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Import (or re-import) a skill: pull the files from `source` into the
/// managed store, then record the row. Name doubles as the directory name.
pub fn skill_save(def: &Value) -> Result<Value, String> {
    let mut sk: store::RegSkill = serde_json::from_value(def.clone()).map_err(|e| format!("invalid skill: {e}"))?;
    sk.name = sk.name.trim().to_string();
    if sk.name.is_empty() || sk.source.trim().is_empty() {
        return Err("skill needs a name and a source".into());
    }
    if !sk.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("skill name must be [a-zA-Z0-9_-] (it names a directory)".into());
    }
    sync_skill_files(&sk.name, &sk.source)?;
    sk.synced_at = Some(now());
    with_store(|store| {
        store.skill_save(&sk, now())?;
        Ok(json!({ "ok": true, "name": sk.name, "synced_at": sk.synced_at }))
    })
}

/// Re-sync a skill's files from its recorded source.
pub fn skill_refresh(name: &str) -> Result<Value, String> {
    let sk = with_store(|store| store.skill_get(name))?
        .ok_or_else(|| format!("no skill named '{name}'"))?;
    sync_skill_files(&sk.name, &sk.source)?;
    let mut updated = sk;
    updated.synced_at = Some(now());
    with_store(|store| {
        store.skill_save(&updated, now())?;
        Ok(json!({ "ok": true, "name": updated.name, "synced_at": updated.synced_at }))
    })
}

/// SKILL.md content from the managed store (for the UI preview).
pub fn skill_read(name: &str) -> Result<Value, String> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("invalid skill name".into());
    }
    let path = managed_skills_dir().join(name).join("SKILL.md");
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(json!({ "name": name, "content": content }))
}

pub fn skill_delete(name: &str) -> Result<Value, String> {
    let deleted = with_store(|store| store.skill_delete(name))?;
    if deleted {
        let _ = std::fs::remove_dir_all(managed_skills_dir().join(name));
    }
    Ok(json!({ "ok": deleted }))
}

pub fn mcp_list() -> Result<Value, String> {
    with_store(|store| Ok(json!({ "mcp": store.mcp_list()? })))
}

pub fn mcp_save(def: &Value) -> Result<Value, String> {
    let m: store::RegMcp = serde_json::from_value(def.clone()).map_err(|e| format!("invalid mcp: {e}"))?;
    if m.name.trim().is_empty() {
        return Err("mcp server needs a name".into());
    }
    serde_json::from_str::<Value>(&m.def).map_err(|e| format!("def must be JSON: {e}"))?;
    with_store(|store| {
        store.mcp_save(&m, now())?;
        Ok(json!({ "ok": true, "name": m.name }))
    })
}

pub fn mcp_delete(name: &str) -> Result<Value, String> {
    with_store(|store| Ok(json!({ "ok": store.mcp_delete(name)? })))
}

/// Central skills as name→ref (spawn-time resolution; empty on store errors —
/// a raw ref in the agent def still works).
pub(crate) fn with_registry_skills() -> std::collections::HashMap<String, String> {
    let root = managed_skills_dir();
    with_store(|store| {
        Ok(store
            .skills_list()?
            .into_iter()
            .map(|s| {
                let managed = root.join(&s.name);
                let target = if managed.is_dir() {
                    managed.to_string_lossy().to_string()
                } else {
                    // Files missing (deleted by hand?) — fall back to the
                    // source so the spawn still works, degraded not broken.
                    s.source
                };
                (s.name, target)
            })
            .collect())
    })
    .unwrap_or_default()
}

/// Central MCP defs as name→def-json.
pub(crate) fn with_registry_mcp() -> std::collections::HashMap<String, String> {
    with_store(|store| Ok(store.mcp_list()?.into_iter().map(|m| (m.name, m.def)).collect()))
        .unwrap_or_default()
}

/// Project row for a session (used by spawn to find the workspace).
pub fn project_for_session(session: &str) -> Result<Option<store::Project>, String> {
    with_store(|store| store.project_by_session(session))
}

/// The isolated home of a MANAGED agent, or `None` when this window is not one.
///
/// This is THE definition of "an agent this app created", and it has to be one
/// function because three unrelated places gate on it and they must not drift:
/// `hub_agents` (who is a chat participant), `maybe_auto_post` (whose replies
/// get posted to the room) and `deliver_mentions` (whose pane we are allowed to
/// type into). The marker is the directory `spawn` materialized — a window the
/// user started by hand can share a name with a registry agent, but it has no
/// isolated home, and typing into it or publishing its replies would reach into
/// a session this app does not own.
pub fn managed_home(session: &str, window_name: &str) -> Option<std::path::PathBuf> {
    let project = project_for_session(session).ok().flatten()?;
    let dir = std::path::Path::new(&project.path)
        .join(".tmm")
        .join("agents")
        .join(window_name);
    dir.is_dir().then_some(dir)
}

/// Same question, when the caller already knows the workspace path (it is
/// listing every window of one session and must not hit the store per row).
pub fn is_managed_in(workspace: Option<&str>, window_name: &str) -> bool {
    workspace.is_some_and(|ws| {
        std::path::Path::new(ws).join(".tmm").join("agents").join(window_name).is_dir()
    })
}

pub fn registry_get(name: &str) -> Result<Option<store::RegAgent>, String> {
    with_store(|store| {
        store.reg_seed(now())?;
        store.reg_get(name)
    })
}

fn load(id: &str) -> Result<(Project, Vec<Slot>), String> {
    with_store(|store| {
        let project = store
            .project(id)?
            .ok_or_else(|| format!("no such project: {id}"))?;
        let slots = store.slots(id)?;
        Ok((project, slots))
    })
}

/// A session name that is free both in tmux and in the store.
fn free_session_name(store: &Store, base: &str, id: &str) -> Result<String, String> {
    if !tmux::session_exists(base) && !store.session_taken_by_other(base, id)? {
        return Ok(base.to_string());
    }
    let suffixed = format!("{base}-{}", digest(id));
    Ok(suffixed)
}

/// The working directory a session represents.
///
/// NOT simply the active pane's cwd: the window that happens to be focused is
/// often a shell parked in `$HOME`, which says nothing about the workspace (a
/// real case: a session whose second window ran an agent in
/// `~/work/poc/260728-ds160` while the focused first window sat in `$HOME`).
/// Ask every window and let `pick_workspace` decide.
fn session_workspace(session: &str) -> Result<String, String> {
    let panes = tmux::list_panes(session)?;
    let mut cwds: Vec<(usize, String)> = Vec::new();
    for pane in &panes {
        if pane.current_path.is_empty() || cwds.iter().any(|(w, _)| *w == pane.window) {
            continue;
        }
        // One vote per window, from its active pane where there is one.
        if !pane.active && panes.iter().any(|p| p.window == pane.window && p.active) {
            continue;
        }
        cwds.push((pane.window, pane.current_path.clone()));
    }
    cwds.sort_by_key(|(w, _)| *w);
    let ordered: Vec<String> = cwds.into_iter().map(|(_, p)| p).collect();
    pick_workspace(&ordered, &tmux::home_dir())
        .ok_or_else(|| format!("cannot determine a directory for session {session}"))
}

/// Choose the directory that best represents a set of window cwds.
///
/// Most frequent wins; `$HOME` only wins when nothing else is on offer (a
/// parked shell is not a workspace); ties break toward the shortest path, which
/// is the one closest to a project root when windows sit in sibling subdirs.
fn pick_workspace(cwds: &[String], home: &str) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for cwd in cwds {
        match counts.iter_mut().find(|(p, _)| *p == cwd.as_str()) {
            Some((_, n)) => *n += 1,
            None => counts.push((cwd.as_str(), 1)),
        }
    }
    let home_trimmed = home.trim_end_matches('/');
    let best = |only_non_home: bool| -> Option<&str> {
        counts
            .iter()
            .filter(|(p, _)| !only_non_home || p.trim_end_matches('/') != home_trimmed)
            .copied()
            .reduce(|a, b| {
                if b.1 > a.1 || (b.1 == a.1 && b.0.len() < a.0.len()) {
                    b
                } else {
                    a
                }
            })
            .map(|(p, _)| p)
    };
    best(true).or_else(|| best(false)).map(str::to_string)
}

// ---- the capture loop ---------------------------------------------------

/// Fold live tmux state into every live project's declaration once.
/// Returns the ids that were written.
pub fn capture_once() -> Result<Vec<String>, String> {
    let ts = now();
    let sessions = agent_sessions();
    with_store(|store| {
        let mut touched = Vec::new();
        for project in store.list_projects(false)? {
            if !tmux::session_exists(&project.session) {
                continue;
            }
            store.mark_seen(&project.id, ts)?;
            let observed = match capture::observe(&project.session, &project.path, sessions) {
                Ok(o) => o,
                Err(_) => continue, // session vanished mid-scan; next tick retries
            };
            let existing = store.slots(&project.id)?;
            let merged = capture::merge(&existing, &observed, ts, capture::SETTLE_SECS);
            if !merged.dirty {
                continue;
            }
            store.replace_slots(&project.id, &merged.slots)?;
            touched.push(project.id);
        }
        Ok(touched)
    })
}

/// Background capturer, spawned once by the server.
pub async fn capture_loop() {
    loop {
        tokio::time::sleep(CAPTURE_INTERVAL).await;
        if let Err(e) = auto_adopt_once() {
            eprintln!("projects: auto-track failed: {e}");
        }
        if let Err(e) = capture_once() {
            eprintln!("projects: capture failed: {e}");
        }
    }
}

#[cfg(test)]
// pub(crate): `use_test_store` must be reachable from tests in OTHER module
// trees (server/hub_rpc). STORE is a OnceLock — whichever test opens it first
// decides the path for the whole process, so a test that reaches the store
// without pointing it at a throwaway db can send every later test's writes into
// the user's real state.db.
pub(crate) mod tests {
    use super::*;

    /// Point the process-wide store at a throwaway database, wiped once per
    /// test process. `STORE` is a `OnceLock`, so every test that touches it must
    /// go through here — the first opener decides the path for the whole run.
    /// pub(crate): spawn's tests reach the store through mcp_defs/
    /// resolve_skill_refs (central-asset resolution), and skipping this once
    /// pointed the WHOLE test process at the user's real state.db.
    pub(crate) fn use_test_store() {
        static TEST_DB: OnceLock<()> = OnceLock::new();
        TEST_DB.get_or_init(|| {
            let dir = std::env::temp_dir().join("tmm-projects-test");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("TMM_STATE_DB", dir.join("state.db"));
        });
    }

    #[test]
    fn the_session_follows_the_project_name_not_the_folder() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-name-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_string_lossy().to_string();

        // A name given, no session: BOTH follow the name. The folder name is a
        // fallback, never a winner — it produced projects called "src-tauri"
        // and sessions called "tmp" (owner reports).
        let made = create(&path, Some("Close Test"), None, None).unwrap();
        assert_eq!(made.get("name").and_then(|v| v.as_str()), Some("Close Test"));
        // slug() lowercases and hyphenates — a tmux session name, not a label.
        assert_eq!(made.get("session").and_then(|v| v.as_str()), Some("close-test"));

        // An explicit session still wins over the name.
        let other = std::env::temp_dir().join(format!("tmm-name-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&other).unwrap();
        let made2 = create(&other.to_string_lossy(), Some("Label"), Some("chosen"), None).unwrap();
        assert_eq!(made2.get("session").and_then(|v| v.as_str()), Some("chosen"));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn two_projects_can_share_one_directory() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-share-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_string_lossy().to_string();

        // Identity is the SESSION, not the path: a second create at the same
        // directory with a different name is a NEW project, not a merge into
        // the first (owner report, 2026-08-19).
        let a = create(&path, Some("alpha-proj"), None, None).unwrap();
        let b = create(&path, Some("beta-proj"), None, None).unwrap();
        assert_ne!(
            a.get("id").and_then(|v| v.as_str()),
            b.get("id").and_then(|v| v.as_str()),
            "same path, different name → two projects"
        );
        assert_eq!(b.get("name").and_then(|v| v.as_str()), Some("beta-proj"));
        assert_eq!(b.get("session").and_then(|v| v.as_str()), Some("beta-proj"));

        // The literal same request IS idempotent: same wanted session + same
        // path returns the existing row instead of a duplicate.
        let a2 = create(&path, Some("alpha-proj"), None, None).unwrap();
        assert_eq!(
            a.get("id").and_then(|v| v.as_str()),
            a2.get("id").and_then(|v| v.as_str()),
            "same session + same path → the same project"
        );

        // Same name + same path a THIRD way: even when the id seed collides,
        // the salt keeps ids unique — session name gets suffixed, not merged.
        let other = std::env::temp_dir().join(format!("tmm-share-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&other).unwrap();
        let c = create(&path, Some("alpha-proj"), Some("alpha-two"), None).unwrap();
        assert_ne!(
            a.get("id").and_then(|v| v.as_str()),
            c.get("id").and_then(|v| v.as_str()),
            "explicit different session at the same path → a third project"
        );
        assert_eq!(c.get("session").and_then(|v| v.as_str()), Some("alpha-two"));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn delete_forgets_the_project_and_its_agent_homes_but_not_your_files() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-del-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // A file of the user's, and an agent home of ours.
        std::fs::write(dir.join("keep-me.txt"), "mine").unwrap();
        let home = dir.join(".tmm").join("agents").join("lead");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("launch.json"), "{}").unwrap();

        let path = dir.to_string_lossy().to_string();
        let made = create(&path, Some("deltest"), None, None).unwrap();
        let id = made.get("id").and_then(|v| v.as_str()).unwrap().to_string();

        let r = delete(&id).unwrap();
        assert_eq!(r.get("deleted").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(r.get("agent_homes_removed").and_then(|v| v.as_u64()), Some(1));
        assert!(!home.exists(), "the agent's isolated home is gone");
        assert!(dir.join("keep-me.txt").is_file(), "the user's files are untouched");
        // Gone from the store even with archived rows included: delete is not
        // archive.
        let listed = list(true).unwrap();
        let ids: Vec<String> = listed
            .get("projects").and_then(|v| v.as_array()).unwrap()
            .iter()
            .filter_map(|p| p.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert!(!ids.contains(&id), "delete removes the row: {ids:?}");
        // Deleting twice is an error, not a silent success — the caller asked
        // about a project that no longer exists.
        assert!(delete(&id).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_an_agent_drops_its_slot_and_home() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-rm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_string_lossy().to_string();
        let made = create(&path, Some("rmtest"), None, None).unwrap();
        let id = made.get("id").and_then(|v| v.as_str()).unwrap().to_string();
        let session = made.get("session").and_then(|v| v.as_str()).unwrap().to_string();

        // A managed agent: a slot in the declaration plus the isolated home
        // that makes it "ours".
        let home = dir.join(".tmm").join("agents").join("dev");
        std::fs::create_dir_all(&home).unwrap();
        with_store(|store| {
            store.replace_slots(&id, &[store::Slot {
                id: None, ord: 0, window_name: "dev".into(), cwd: String::new(),
                kind: store::SlotKind::Agent, command: Some("kiro".into()),
                auto_run: true, agent_session_id: None,
                first_seen_at: now(), settled_at: Some(now()),
            }])
        })
        .unwrap();

        let r = agent_remove(&session, "dev").unwrap();
        assert_eq!(r.get("slot_removed").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(r.get("home_removed").and_then(|v| v.as_bool()), Some(true));
        assert!(!home.exists());
        // `up` must not bring it back: the slot is gone from the declaration.
        let slots = with_store(|store| store.slots(&id)).unwrap();
        assert!(!slots.iter().any(|s| s.window_name == "dev"), "slot is gone");
        // A name that was never ours is rejected rather than half-handled.
        assert!(agent_remove(&session, "nope").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one definition of "an agent this app created". Three gates share it
    /// (chat participants, stop-hook auto-post, pane delivery), so it is worth
    /// a test of its own: the marker is the isolated home `spawn` materialized,
    /// NOT the window name — a hand-started window may share the name.
    #[test]
    fn managed_is_the_isolated_home_not_the_name() {
        let ws = std::env::temp_dir().join(format!("tmm-managed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(ws.join(".tmm/agents/dev")).unwrap();
        let path = ws.to_string_lossy().to_string();
        assert!(is_managed_in(Some(&path), "dev"), "spawn materialized this one");
        assert!(!is_managed_in(Some(&path), "byhand"), "same session, no isolated home");
        assert!(!is_managed_in(None, "dev"), "a session with no project owns nothing");
        // A file where the directory should be is not a home either.
        std::fs::write(ws.join(".tmm/agents/file"), "x").unwrap();
        assert!(!is_managed_in(Some(&path), "file"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn skill_import_owns_the_files_and_refresh_resyncs() {
        use_test_store();
        // A local source directory with a SKILL.md.
        let src = std::env::temp_dir().join(format!("tmm-skill-src-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: demo\n---\nv1").unwrap();
        std::fs::write(src.join("helper.py"), "print(1)").unwrap();

        // Import copies the files into the app-managed store.
        skill_save(&serde_json::json!({
            "name": "demo-skill",
            "source": src.to_string_lossy(),
            "description": "d"
        }))
        .unwrap();
        let managed = managed_skills_dir().join("demo-skill");
        assert!(managed.join("SKILL.md").is_file(), "files live in the managed dir");
        assert!(managed.join("helper.py").is_file());
        assert!(std::fs::read_to_string(managed.join("SKILL.md")).unwrap().ends_with("v1"));
        let listed = with_store(|st| st.skills_list()).unwrap();
        assert!(listed[0].synced_at.is_some(), "import records the sync time");

        // Agents resolve to the MANAGED copy, not the source.
        let map = with_registry_skills();
        assert_eq!(map.get("demo-skill").unwrap(), &managed.to_string_lossy().to_string());

        // Source changes → refresh re-syncs the managed copy.
        std::fs::write(src.join("SKILL.md"), "---\nname: demo\n---\nv2").unwrap();
        skill_refresh("demo-skill").unwrap();
        assert!(std::fs::read_to_string(managed.join("SKILL.md")).unwrap().ends_with("v2"));

        // Delete removes row AND files.
        skill_delete("demo-skill").unwrap();
        assert!(!managed.exists());
        std::fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn skill_import_rejects_bad_names_and_sources() {
        use_test_store();
        let err = skill_save(&serde_json::json!({ "name": "../evil", "source": "/tmp" })).unwrap_err();
        assert!(err.contains("a-zA-Z0-9"), "directory-unsafe names refused: {err}");
        let err = skill_save(&serde_json::json!({ "name": "ok", "source": "relative/path" })).unwrap_err();
        assert!(err.contains("absolute"), "{err}");
        let empty = std::env::temp_dir().join(format!("tmm-noskill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&empty).unwrap();
        let err = skill_save(&serde_json::json!({ "name": "ok", "source": empty.to_string_lossy() })).unwrap_err();
        assert!(err.contains("SKILL.md"), "{err}");
        std::fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn slugs_are_tmux_safe_and_bounded() {
        assert_eq!(slug("my.project"), "my-project");
        assert_eq!(slug("Web App!"), "web-app");
        assert_eq!(slug("---"), "project");
        assert_eq!(slug("a".repeat(40).as_str()).len(), 24);
        assert!(!slug("a:b.c").contains(':'));
    }

    #[test]
    fn digest_is_stable_and_path_specific() {
        assert_eq!(digest("/w/app"), digest("/w/app"));
        assert_ne!(digest("/w/app"), digest("/w/app2"));
        assert_eq!(digest("/w/app").len(), 6);
    }

    #[test]
    fn basename_handles_trailing_slashes() {
        assert_eq!(basename("/w/app"), "app");
        assert_eq!(basename("/w/app/"), "app");
        assert_eq!(basename("app"), "app");
    }

    #[test]
    fn a_workspace_is_the_directory_the_windows_agree_on() {
        let home = "/Users/me";
        // The focused window sits in $HOME, the work happens elsewhere: the
        // real session that exposed this bug.
        assert_eq!(
            pick_workspace(&["/Users/me".into(), "/Users/me/work/poc/ds160".into()], home).as_deref(),
            Some("/Users/me/work/poc/ds160"),
        );
        // Nothing but $HOME on offer — then $HOME is the honest answer.
        assert_eq!(
            pick_workspace(&["/Users/me".into(), "/Users/me".into()], home).as_deref(),
            Some("/Users/me"),
        );
        // Majority wins over a single deeper window.
        assert_eq!(
            pick_workspace(
                &["/w/app".into(), "/w/app".into(), "/w/app/packages/api".into()],
                home
            )
            .as_deref(),
            Some("/w/app"),
        );
        // All different: the shortest is the one closest to a project root.
        assert_eq!(
            pick_workspace(&["/w/app/api".into(), "/w/app".into(), "/w/app/web".into()], home)
                .as_deref(),
            Some("/w/app"),
        );
        assert_eq!(pick_workspace(&[], home), None);
    }

    /// Auto-tracking is what makes "every session is a project" true, including
    /// for sessions made outside the app — and it is the migration path for the
    /// ones that already existed. The guards are the interesting part.
    #[test]
    fn auto_track_picks_up_outside_sessions_but_respects_the_guards() {
        let root = std::env::temp_dir().join("tmm-proj-auto");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        use_test_store();
        let path = root.canonicalize().unwrap().to_string_lossy().to_string();
        let old = "tmm-test-auto-old";
        let fresh = "tmm-test-auto-fresh";
        let team = "tmm-team-test-auto";
        for s in [old, fresh, team] {
            let _ = tmux::kill_session(s);
            tmux::ensure_session(s, &path).unwrap();
        }

        // Pretend `old` has been around long enough; the others were just made.
        let ts = now();
        let ages: Vec<(String, u64)> = vec![
            (old.to_string(), ts - SESSION_SETTLE_SECS - 1),
            (fresh.to_string(), ts),
            (team.to_string(), ts - SESSION_SETTLE_SECS - 1),
        ];
        let adopted = auto_adopt_with(&ages, ts).unwrap();
        assert_eq!(adopted, vec![old.to_string()], "only the settled non-team session");

        // Running again must not duplicate it.
        assert!(auto_adopt_with(&ages, ts).unwrap().is_empty());

        // Removing a project from the list must stick: no re-tracking.
        let id = with_store(|s| Ok(s.project_by_session(old)?.unwrap().id)).unwrap();
        set_archived(&id, true).unwrap();
        assert!(
            auto_adopt_with(&ages, ts).unwrap().is_empty(),
            "an archived project must not come back on the next tick"
        );

        for s in [old, fresh, team] {
            let _ = tmux::kill_session(s);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_sessions_in_the_same_directory_are_two_projects() {        let root = std::env::temp_dir().join("tmm-proj-share");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        use_test_store();
        let path = root.canonicalize().unwrap().to_string_lossy().to_string();
        for s in ["tmm-test-share-a", "tmm-test-share-b"] {
            let _ = tmux::kill_session(s);
            tmux::ensure_session(s, &path).unwrap();
        }

        let first = adopt("tmm-test-share-a", None).unwrap();
        // Used to fail with "<path> is already project ..." — several sessions
        // parked in one directory (typically $HOME) is the normal case.
        let second = adopt("tmm-test-share-b", None).unwrap();
        assert_eq!(first["project"]["path"], second["project"]["path"]);
        assert_ne!(first["project"]["id"], second["project"]["id"]);

        let again = adopt("tmm-test-share-a", None);
        assert!(
            again.is_err_and(|e| e.contains("already tracked")),
            "the same session twice is the real conflict"
        );

        for s in ["tmm-test-share-a", "tmm-test-share-b"] {
            let _ = tmux::kill_session(s);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The P0 acceptance criterion, end to end against a real tmux server:
    /// adopt a session the user made, kill it, and get it back.
    ///
    #[test]
    fn adopt_then_down_then_up_restores_the_workspace() {
        let root = std::env::temp_dir().join("tmm-proj-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("api")).unwrap();
        use_test_store();
        let path = root.canonicalize().unwrap().to_string_lossy().to_string();
        let session = "tmm-test-e2e";
        let _ = tmux::kill_session(session);

        tmux::ensure_session(session, &path).unwrap();
        tmux::rename_window(&format!("{session}:^"), "editor").unwrap();
        tmux::new_named_window(session, "api", &format!("{path}/api")).unwrap();

        let adopted = adopt(session, None).unwrap();
        let id = adopted["project"]["id"].as_str().unwrap().to_string();
        let slots = adopted["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 2, "both live windows became slots: {slots:?}");
        assert!(
            slots.iter().all(|s| s["settled_at"].is_number()),
            "adopted windows are restorable immediately: {slots:?}"
        );
        let api = slots
            .iter()
            .find(|s| s["window_name"] == "api")
            .expect("api slot");
        assert_eq!(api["cwd"], "api", "cwd is stored relative to the project");

        // The board lists it as a project, which is what removes it from the
        // client's session list.
        let listed = list(false).unwrap();
        assert_eq!(
            listed["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["project"]["id"] == id.as_str())
                .map(|e| e["live"].as_bool().unwrap()),
            Some(true)
        );

        down(&id).unwrap();
        assert!(!tmux::session_exists(session), "down kills the session");

        let report = up(&id).unwrap();
        assert_eq!(report["created_session"], true);
        let windows: Vec<String> = tmux::list_named_windows(session)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(windows.len(), 2, "declaration rebuilt the topology: {windows:?}");
        assert!(windows.contains(&"editor".to_string()));
        assert!(windows.contains(&"api".to_string()));

        // Capturing a live project must not disturb a settled declaration.
        capture_once().unwrap();
        assert_eq!(with_store(|s| s.slots(&id)).unwrap().len(), 2);

        let _ = tmux::kill_session(session);
        let _ = std::fs::remove_dir_all(&root);
    }
}
