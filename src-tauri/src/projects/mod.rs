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
pub mod models;
pub mod reconcile;
pub mod recovery;
pub mod spawn;
pub mod store;
pub mod telemetry;
pub mod vitals;

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
            room: String::new(),   // insert_project freezes it as proj:<session>
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
        room: String::new(),   // insert_project freezes it as proj:<session>
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

/// Rename a project — the LABEL only.
///
/// A project is named by its name and identified by its session (see
/// `create`), and three things are keyed on that session: the row's UNIQUE
/// column, the tmux session the declaration projects onto, and the chat room
/// `proj:<session>`. So renaming the session would silently orphan the
/// conversation and leave a live session no project claims; the name is the
/// part a user actually reads, and it is the part that moves.
pub fn rename(id: &str, name: &str) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("project name must not be empty".into());
    }
    let ts = now();
    with_store(|store| {
        let Some(project) = store.project(id)? else {
            return Err(format!("no project with id '{id}'"));
        };
        // The tmux SESSION follows the name, because it is the name the Terminal
        // and `tmux ls` show — leaving it behind made one project wear two names
        // (owner, 2026-08-19: "没有改tmux session的名字 所以在terminal显示不对").
        //
        // No exception for ADOPTED projects: `auto_adopt_once` adopts every
        // untracked session automatically, so `adopted` mostly means "the app
        // found it before it was declared", not "a human chose this name". The
        // first cut skipped them and thereby disabled the feature on 2 of the
        // owner's 4 projects, including the one they were renaming.
        let wanted = slug(name);
        // Everything that can REFUSE happens before anything is written: a rename
        // that moved the label and then failed on the session left the project
        // wearing two names again — the exact bug this feature exists to fix.
        if wanted != project.session {
            if tmux::session_exists(&wanted) {
                return Err(format!("a tmux session named '{wanted}' already exists"));
            }
            if store.session_taken_by_other(&wanted, id)? {
                let owner = store.project_by_session(&wanted)?;
                let label = owner.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                let archived = owner.as_ref().is_some_and(|p| p.archived);
                // A rename REFUSES a taken name; it does not decorate one.
                // `create` suffixes with a digest because there the alternative is
                // failing to make the project at all, but here the user typed a
                // name and `closetest-e110d2` is not an answer to that — measured
                // on the owner's own data, where an ARCHIVED (invisible) project
                // was holding the name.
                return Err(format!(
                    "session '{wanted}' belongs to {}project '{label}' — rename or delete that one first",
                    if archived { "the archived " } else { "" },
                ));
            }
        }

        if !store.set_name(id, name)? {
            return Err(format!("no project with id '{id}'"));
        }
        let mut session = project.session.clone();
        let mut renamed_session = false;
        if wanted != project.session {
            // tmux first: if it refuses anyway (a session created between the
            // check and here), the declaration must not drift away from it.
            let live = tmux::session_exists(&project.session);
            if !live || tmux::rename_session(&project.session, &wanted).is_ok() {
                store.set_session(id, &wanted, &project.session)?;
                session = wanted;
                renamed_session = true;
            }
        }
        store.mark_seen(id, ts)?;
        Ok(json!({
            "id": id,
            "name": name,
            "session": session,
            "session_renamed": renamed_session,
        }))
    })
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
///
/// It removes whatever of those three the agent still has, and only refuses when
/// there is NOTHING of it left in this project. The narrower rule — a managed
/// home must exist — made two ordinary cases unremovable (owner report,
/// 2026-08-19: "停止的 agent，没办法 remove"):
///
/// * a STOPPED agent still holds a slot, and the roster offers it exactly
///   because starting it resumes its conversation; refusing to remove it left
///   the declaration as the only way to get rid of it;
/// * an agent whose home was deleted by hand (or that was never ours — a window
///   the user started, which the capturer adopts into a slot all the same) could
///   never be dropped, so `up` kept recreating a window nobody wanted.
///
/// The slot is what makes it a member of the project, so the slot is what
/// authorizes the removal. `home_removed` reports whether there was a home.
pub fn agent_remove(session: &str, agent: &str) -> Result<Value, String> {
    let project = project_for_session(session)?
        .ok_or_else(|| format!("no project for session '{session}'"))?;
    let home = managed_home(session, agent);
    let declared = with_store(|store| store.slots(&project.id))?
        .iter()
        .any(|s| s.window_name == agent);
    // Kill the window first, or the capture loop would re-add the slot we are
    // about to delete from a window that is still alive.
    let mut window_killed = false;
    if let Ok(panes) = crate::tmux::list_panes(session) {
        if let Some(p) = panes.iter().find(|p| p.window_name == agent) {
            if home.is_some() || declared {
                let _ = crate::tmux::kill_window(&format!("{session}:{}", p.window));
                window_killed = true;
            }
        }
    }
    if home.is_none() && !declared && !window_killed {
        return Err(format!("'{agent}' is not an agent of project '{}'", project.name));
    }
    let slot_removed = with_store(|store| store.delete_slot(&project.id, agent))?;
    let home_removed = home.is_some_and(|h| std::fs::remove_dir_all(h).is_ok());
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
    if !matches!(agent.backend.as_str(), "kiro" | "claude" | "codex" | "grok") {
        return Err(format!("backend must be kiro|claude|codex|grok, got '{}'", agent.backend));
    }
    // Validate the JSON columns now, not at spawn time.
    serde_json::from_str::<Vec<String>>(&agent.skills).map_err(|e| format!("skills must be a JSON array of refs: {e}"))?;
    serde_json::from_str::<Vec<Value>>(&agent.mcp).map_err(|e| format!("mcp must be a JSON array of defs: {e}"))?;
    // Same rule for the model, and for a sharper reason: a backend that does
    // not know the id does not fail, it falls back to its default and says so
    // in a line nobody reads. See `models`.
    models::validate(&agent.backend, &agent.model)?;
    models::validate_effort(&agent.backend, &agent.effort)?;
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
/// The project a session name refers to. Falls back to the name the session
/// used to have, because a running agent carries `TMM_PROJECT` from the moment
/// it started: after a rename its `tmm send/status/done` would otherwise fail
/// until someone restarted it, which is not a thing a rename should cost.
pub fn project_for_session(session: &str) -> Result<Option<store::Project>, String> {
    with_store(|store| {
        if let Some(p) = store.project_by_session(session)? {
            return Ok(Some(p));
        }
        store.project_by_prev_session(session)
    })
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

// ---- archived messages ---------------------------------------------------
//
// The archive is OUR state (state.db), not the bus's: `agora` is a faithful copy
// of an upstream crate, and hiding a message is this app's idea. These are the
// four verbs the hub RPCs need, each fail-soft in the direction that keeps the UI
// honest — a read that fails hides nothing, a write that fails is reported.

/// Ids hidden in a room. A failure here must not blank the conversation, so it
/// degrades to "nothing is hidden".
pub fn archived_ids(room: &str) -> Vec<String> {
    with_store(|s| s.archived_ids(room)).unwrap_or_default()
}

/// The archive itself, newest first, each row carrying its own copy of the
/// message.
pub fn archived_msgs(room: &str) -> Vec<(String, u64, String, String, u64)> {
    with_store(|s| s.archived_msgs(room)).unwrap_or_default()
}

/// Hide one message.
pub fn archive_msg(room: &str, msg_id: &str, ts: u64, sender: &str, body: &str) -> Result<(), String> {
    with_store(|s| s.archive_msg(room, msg_id, ts, sender, body, now()))
}

/// Take messages out of the archive — a restore, or the bookkeeping half of a
/// purge once the messages themselves are gone.
pub fn unarchive_msgs(room: &str, ids: &[String]) -> Result<usize, String> {
    with_store(|s| s.unarchive_msgs(room, ids))
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
        // Piggybacks on the capture cadence: a transient model error is
        // noticed within one tick, and the backoff ladder is measured in tens
        // of seconds, so 20s granularity costs nothing.
        recovery::check_once();
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

    /// Renaming moves the label AND the tmux session, because the session name
    /// is what the Terminal and `tmux ls` show — leaving it behind made one
    /// project wear two names (owner, 2026-08-19). Two things must NOT move with
    /// it: the chat room (the conversation would be orphaned) and the old
    /// session's resolvability (a running agent carries `TMM_PROJECT` from the
    /// moment it started).
    #[test]
    fn renaming_a_project_moves_its_session_but_never_its_room() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-rename-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let made = create(&dir.to_string_lossy(), Some("Old Name"), None, None).unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        let born_session = made["session"].as_str().unwrap().to_string();
        assert_eq!(born_session, "old-name");
        let room = with_store(|s| s.project(&id)).unwrap().unwrap().room;
        assert_eq!(room, format!("proj:{born_session}"), "the room is frozen at birth");

        let out = rename(&id, "  New Name  ").unwrap();
        assert_eq!(out["name"].as_str(), Some("New Name"), "trimmed");
        assert_eq!(out["session"].as_str(), Some("new-name"), "the session follows the name");
        let after = with_store(|store| store.project(&id)).unwrap().unwrap();
        assert_eq!(after.name, "New Name");
        assert_eq!(after.session, "new-name");
        assert_eq!(after.path, made["path"].as_str().unwrap());
        // The conversation stays where it is. This is the whole reason the room
        // is a column instead of `proj:<session>`.
        assert_eq!(after.room, room, "the chat must not move with the name");

        // A name another project holds is REFUSED, not decorated — including when
        // that project is archived and therefore invisible in the list.
        let other = std::env::temp_dir().join(format!("tmm-rename-other-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&other).unwrap();
        let taken = create(&other.to_string_lossy(), Some("Taken Name"), None, None).unwrap();
        set_archived(taken["id"].as_str().unwrap(), true).unwrap();
        let err = rename(&id, "Taken Name").expect_err("a taken session must not be decorated");
        assert!(err.contains("archived"), "the message must explain WHY: {err}");
        assert!(err.contains("taken-name"), "and name the session: {err}");
        let untouched = with_store(|s| s.project(&id)).unwrap().unwrap();
        assert_eq!(untouched.session, "new-name", "a refused rename changes nothing");
        assert_eq!(untouched.name, "New Name", "…including the label: no half-applied rename");
        let _ = std::fs::remove_dir_all(&other);

        // The old name still resolves, so an agent started before the rename can
        // keep using the TMM_PROJECT it was launched with.
        let via_old = project_for_session(&born_session).unwrap();
        assert_eq!(via_old.map(|p| p.id), Some(id.clone()), "previous session name resolves");
        assert_eq!(project_for_session("new-name").unwrap().map(|p| p.id), Some(id.clone()));

        // An empty name is a mistake, not a way to clear the label.
        assert!(rename(&id, "   ").is_err());
        assert_eq!(with_store(|store| store.project(&id)).unwrap().unwrap().name, "New Name");
        assert!(rename("no-such-project", "x").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An ADOPTED project is renamed like any other. `adopted` is set by
    /// `auto_adopt_once` for every session the app finds untracked, so treating
    /// it as "a human named this" made the rename a no-op on most real projects.
    #[test]
    fn an_adopted_project_is_renamed_like_any_other() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-rename-adopted-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ts = now();
        let project = Project {
            id: "adopted-1".into(),
            name: "mine".into(),
            path: dir.to_string_lossy().to_string(),
            icon: None,
            session: "hand-made".into(),
            adopted: true,
            autostart: false,
            created_at: ts,
            last_up_at: None,
            last_seen_at: None,
            archived: false,
            room: String::new(),
        };
        with_store(|store| store.insert_project(&project)).unwrap();

        let out = rename("adopted-1", "A Better Label").unwrap();
        assert_eq!(out["name"].as_str(), Some("A Better Label"));
        assert_eq!(out["session"].as_str(), Some("a-better-label"), "adopted renames too");
        assert_eq!(out["session_renamed"].as_bool(), Some(true));
        let after = with_store(|store| store.project("adopted-1")).unwrap().unwrap();
        assert_eq!(after.session, "a-better-label");
        assert_eq!(after.name, "A Better Label");
        // The old name still resolves, and the room never moved.
        assert_eq!(
            project_for_session("hand-made").unwrap().map(|p| p.id).as_deref(),
            Some("adopted-1"),
        );
        assert_eq!(after.room, "proj:hand-made");

        let _ = std::fs::remove_dir_all(&dir);
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

    /// A STOPPED agent is the ordinary case: no window, no reason its home has
    /// to be intact, but a slot that keeps `up` recreating it. Requiring a
    /// managed home made those unremovable (owner, 2026-08-19).
    #[test]
    fn a_stopped_agent_can_be_removed_by_its_declaration_alone() {
        use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-rmstop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let made = create(&dir.to_string_lossy(), Some("rmstop"), None, None).unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        let session = made["session"].as_str().unwrap().to_string();
        let slot = |name: &str| store::Slot {
            id: None, ord: 0, window_name: name.into(), cwd: String::new(),
            kind: store::SlotKind::Agent, command: Some("kiro".into()),
            auto_run: true, agent_session_id: None,
            first_seen_at: now(), settled_at: Some(now()),
        };
        // Declared, never started (or started and stopped): no home on disk.
        with_store(|store| store.replace_slots(&id, &[slot("ghost")])).unwrap();
        assert!(managed_home(&session, "ghost").is_none(), "no home — that is the point");

        let r = agent_remove(&session, "ghost").unwrap();
        assert_eq!(r["slot_removed"].as_bool(), Some(true), "the declaration is what we can remove");
        assert_eq!(r["home_removed"].as_bool(), Some(false), "there was nothing to delete");
        let slots = with_store(|store| store.slots(&id)).unwrap();
        assert!(slots.is_empty(), "`up` cannot bring it back");
        // Nothing left of it anywhere: now it IS an unknown name.
        assert!(agent_remove(&session, "ghost").is_err());

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
