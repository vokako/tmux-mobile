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
pub mod global_prompt;
pub mod models;
pub mod reconcile;
pub mod recovery;
pub mod rooms;
pub(crate) mod skills;
pub mod spawn;
pub mod store;
pub mod teams;
pub mod telemetry;
pub mod vitals;
pub mod board;
pub mod registry;
pub mod managed;
pub use board::{BOARD_STATUSES, ISSUE_REF_CHARS, issue_ref, board_list, board_counts, board_get, board_save, board_note, board_delete};
pub use rooms::{archived_ids, archived_msgs, archive_msg, unarchive_msgs};
pub use registry::{registry_list, registry_save, registry_delete, registry_get, teams_list, team_get, teams_save, teams_delete, global_prompt_get, global_prompt_set};
pub(crate) use registry::{with_registry_mcp, with_registry_skills};
pub use managed::{managed_home, is_managed_in, spawned_by, team_of, agent_remove};
pub use skills::{seed_builtin_skills, skills_list, managed_skills_dir, skill_import, skill_files, skill_file, skill_save, skill_refresh, skill_read, skill_delete, mcp_list, mcp_save, mcp_delete};

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use store::{Project, Slot, Store};

/// How often the capturer folds live tmux state back into the declaration.
const CAPTURE_INTERVAL: Duration = Duration::from_secs(20);

/// How long a tmux session must have existed before it becomes a project on its
/// own. Same reasoning as `capture::SETTLE_SECS` one level up: a workspace is
/// something you come back to, a two-minute shell is not.
pub const SESSION_SETTLE_SECS: u64 = 120;

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `state.db` next to the rest of the app's state. `TMM_STATE_DB` overrides it,
/// which is how the tests get an isolated database (and how a second profile on
/// one machine could get its own).
pub(super) fn db_path() -> PathBuf {
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
    let cell = open_once(&STORE, || Store::open(&db_path()))?;
    let mut guard = cell.lock().map_err(|_| "state.db lock poisoned".to_string())?;
    f(&mut guard)
}

/// First use opens the store; every later use finds it. Exactly ONE caller
/// opens (board #150): the cold path is serialised by `OPENING`, and a caller
/// that waited re-reads the cell before opening — with the bare
/// `open → OnceLock::set` shape two first callers both ran the migration
/// ladder on the same file (reproduced in the #149 probe), and the ladder's
/// rebuild steps (deliveries v20/v21 DROP+RENAME) are not safe to run twice at
/// once. A failed open is NOT cached (`get_or_init` would have to cache it):
/// the next caller tries again, so a database that was unavailable for a
/// moment does not take the Projects page down for the process.
fn open_once<'a>(
    cell: &'a OnceLock<Mutex<Store>>,
    open: impl FnOnce() -> Result<Store, String>,
) -> Result<&'a Mutex<Store>, String> {
    static OPENING: Mutex<()> = Mutex::new(());
    if let Some(c) = cell.get() {
        return Ok(c);
    }
    let _opening = OPENING.lock().map_err(|_| "state.db init lock poisoned".to_string())?;
    if let Some(c) = cell.get() {
        return Ok(c);
    }
    let store = open()?;
    let _ = cell.set(Mutex::new(store));
    cell.get().ok_or_else(|| "state.db unavailable".to_string())
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
    // Rows under the lock, tmux outside it (board #149): `has-session` is a
    // subprocess (~9 ms here) and this ran it per project while every other
    // store caller queued — 7 live projects = ~60 ms of lock per list.
    let rows = with_store(|store| {
        let projects = store.list_projects(include_archived)?;
        let mut out = Vec::with_capacity(projects.len());
        for p in projects {
            let slots = store.slots(&p.id)?;
            out.push((p, slots));
        }
        Ok(out)
    })?;
    let projects: Vec<Value> = rows
        .into_iter()
        .map(|(p, slots)| {
            let live = tmux::session_exists(&p.session);
            json!({ "project": p, "slots": slots, "live": live })
        })
        .collect();
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
    let facts = adopt_facts(session)?;
    with_store(|store| adopt_in(store, session, name, ts, &facts))
}

/// The tmux half of an adoption, read OUTSIDE the store lock (board #149):
/// the session's workspace and its windows. Two pane listings ≈ 90 ms on this
/// host, during which every other store caller used to queue.
struct AdoptFacts {
    path: String,
    observed: Vec<capture::Observed>,
}

fn adopt_facts(session: &str) -> Result<AdoptFacts, String> {
    let path = canonical(&session_workspace(session)?)?;
    let observed = capture::observe(session, &path, agent_sessions())?;
    Ok(AdoptFacts { path, observed })
}

/// The row half of an adoption: only SQLite inside. The already-tracked check
/// runs here, under the lock, so two adopters racing on one session still get
/// the same answer they always did.
fn adopt_in(
    store: &mut Store,
    session: &str,
    name: Option<&str>,
    ts: u64,
    facts: &AdoptFacts,
) -> Result<Value, String> {
    let path = facts.path.clone();
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
    let merged = capture::merge(&[], &facts.observed, ts, capture::SETTLE_SECS);
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
    // Lock once to learn what is known, read tmux with no lock held, then
    // lock once more for the row writes (board #149). `adopt_in` re-checks
    // "already tracked" under the lock, so a session adopted by hand between
    // the two locks is refused exactly as before.
    let known: Vec<String> =
        with_store(|store| Ok(store.list_projects(true)?.into_iter().map(|p| p.session).collect()))?;
    let mut candidates: Vec<(&String, AdoptFacts)> = Vec::new();
    for (session, created_at) in created {
        if known.contains(session) || ts.saturating_sub(*created_at) < SESSION_SETTLE_SECS {
            continue;
        }
        match adopt_facts(session) {
            Ok(facts) => candidates.push((session, facts)),
            Err(e) => eprintln!("projects: cannot track session {session}: {e}"),
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    with_store(|store| {
        let mut adopted = Vec::new();
        for (session, facts) in &candidates {
            match adopt_in(store, session, None, ts, facts) {
                Ok(_) => adopted.push((*session).clone()),
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
            // store-lock(tmux-ok): the ONE place tmux is touched under the store
            // lock on purpose — the session rename and the row re-key must not
            // drift apart (below), and the has-session check above is the
            // refuse-before-write precondition of the same act. One rename, a
            // handful of milliseconds, user-initiated (board #149 kept it).
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
/// than losing it. The Board is project task state, not the room record: its
/// issues and note threads are deleted before the session name is released,
/// so another project can never inherit them (board #41).
pub fn delete(id: &str) -> Result<Value, String> {
    let project = with_store(|store| store.project(id))?
        .ok_or_else(|| format!("no project with id '{id}'"))?;
    // Down first: killing the session while its declaration still exists is
    // what `down` is for, and it keeps the reconciler from re-creating windows
    // for a project that is about to vanish. And down MUST succeed before
    // anything is forgotten: a session that survives with its homes deleted
    // and its row gone is an untracked session full of windows nobody owns,
    // which `auto_adopt_once` re-adopts as a NEW project within 120 s —
    // delete would resurrect what it was asked to forget. (`down` is Ok when
    // the session is already gone.)
    down(id).map_err(|e| format!("'{}' was not deleted — its session did not go down: {e}", project.name))?;
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

pub fn set_autostart(id: &str, autostart: bool) -> Result<Value, String> {
    with_store(|store| {
        store.set_autostart(id, autostart)?;
        Ok(json!({ "id": id, "autostart": autostart }))
    })
}

// ---- central skills / MCP assets ----------------------------------------

// ---- the project task board ------------------------------------------------
//
// A kanban over the session, stored beside the projects it serves (owner,
// 2026-08-29: "借助软件工程，可以把我们的任务管理的更好"). Four fixed columns —
// a free-text status would fork the vocabulary per agent and the board would
// stop being readable at a glance. The HUMAN writes issues on the board page;
// agents read and update them through `tmm board`.

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
///
/// Two phases, and the store lock is held only in the second. Observing is
/// tmux work — `has-session`, `list-panes` (which runs `ps` for the child
/// commands), a `launch.json` read per window — tens to hundreds of
/// milliseconds per project; every RPC in the server goes through
/// `with_store`, so doing that under the lock stalled each of them behind the
/// tick. Folding is a few SQLite statements per project and is all the lock
/// protects.
pub fn capture_once() -> Result<Vec<String>, String> {
    let ts = now();
    let sessions = agent_sessions();
    let projects = with_store(|store| store.list_projects(false))?;

    // Phase 1 — no lock: which declared sessions are live, and what tmux shows.
    let mut seen: Vec<String> = Vec::new();
    let mut observed: Vec<(String, String, Vec<capture::Observed>)> = Vec::new();
    for project in projects {
        if !tmux::session_exists(&project.session) {
            continue;
        }
        seen.push(project.id.clone());
        match capture::observe(&project.session, &project.path, sessions) {
            Ok(o) => observed.push((project.id, project.path, o)),
            Err(_) => {} // session vanished mid-scan; next tick retries
        }
    }

    // Phase 2 — one short lock: fold the observations into the declarations.
    with_store(|store| {
        for id in &seen {
            store.mark_seen(id, ts)?;
        }
        let mut touched = Vec::new();
        for (id, path, observed) in observed {
            // Deleted between the phases: nothing to fold into, and inserting
            // slots for a vanished project would fail the whole tick.
            if store.project(&id)?.is_none() {
                continue;
            }
            let existing = store.slots(&id)?;
            // Stop is a pause for managed agents: their isolated home is the
            // durable membership record, so a missing window keeps its slot
            // and exact conversation id. Remove deletes both explicitly.
            let keep_missing = existing
                .iter()
                .filter(|s| s.kind == store::SlotKind::Agent)
                .filter(|s| is_managed_in(Some(&path), &s.window_name))
                .map(|s| s.window_name.clone())
                .collect();
            let merged = capture::merge_preserving(
                &existing,
                &observed,
                ts,
                capture::SETTLE_SECS,
                &keep_missing,
            );
            if !merged.dirty {
                continue;
            }
            store.replace_slots(&id, &merged.slots)?;
            touched.push(id);
        }
        Ok(touched)
    })
}

/// Background capturer, spawned once by the server.
///
/// The tick is tmux subprocesses, `ps`, SQLite and file reads — all blocking —
/// so it runs on the blocking pool, never inline on a runtime worker where it
/// would hold up every WebSocket connection's tasks for its duration.
pub async fn capture_loop() {
    // Built-ins materialize once per server start, before the first tick.
    seed_builtin_skills();
    loop {
        tokio::time::sleep(CAPTURE_INTERVAL).await;
        // One process-table snapshot for the whole tick (board #145): observe
        // and the recovery scan list the same sessions back to back, and each
        // `list_panes` used to run its own `ps` — 2 × live projects per tick.
        let tick = tokio::task::spawn_blocking(|| {
            tmux::with_process_snapshot(|| {
                if let Err(e) = auto_adopt_once() {
                    eprintln!("projects: auto-track failed: {e}");
                }
                if let Err(e) = capture_once() {
                    eprintln!("projects: capture failed: {e}");
                }
                // Piggybacks on the capture cadence: a transient model error is
                // noticed within one tick, and the backoff ladder is measured in
                // tens of seconds, so 20s granularity costs nothing.
                recovery::check_once();
            })
        })
        .await;
        if let Err(e) = tick {
            eprintln!("projects: capture tick panicked: {e}");
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
        let issue = board_save(
            "hand-made",
            None,
            Some("board follows rename"),
            Some("private to this project"),
            None,
            None,
            "human",
        ).unwrap();
        board_note("hand-made", issue, "human", "keep the thread").unwrap();

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
        let moved = board_get("a-better-label", issue).expect("the Board follows the current session name");
        assert_eq!(moved["title"], "board follows rename");
        assert_eq!(moved["notes"].as_array().unwrap().len(), 1);
        assert!(board_get("hand-made", issue).is_err(), "the raw old Board key no longer owns rows");

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
        let session = made.get("session").and_then(|v| v.as_str()).unwrap().to_string();
        let issue = board_save(&session, None, Some("delete with project"), None, None, None, "human").unwrap();
        board_note(&session, issue, "human", "this task is project state").unwrap();

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
        assert!(board_get(&session, issue).is_err(), "permanent delete releases the session with no old Board to inherit");
        // Deleting twice is an error, not a silent success — the caller asked
        // about a project that no longer exists.
        assert!(delete(&id).is_err());

        let _ = std::fs::remove_dir_all(&dir);
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
        for s in [old, fresh] {
            let _ = tmux::kill_session(s);
            tmux::ensure_session(s, &path).unwrap();
        }

        // Pretend `old` has been around long enough; the other was just made.
        // (The tmm-team-* exclusion died with the Team system, board #100 —
        // a leftover session with that name is an ordinary session now.)
        let ts = now();
        let ages: Vec<(String, u64)> = vec![
            (old.to_string(), ts - SESSION_SETTLE_SECS - 1),
            (fresh.to_string(), ts),
        ];
        let adopted = auto_adopt_with(&ages, ts).unwrap();
        assert_eq!(adopted, vec![old.to_string()], "only the settled session");

        // Running again must not duplicate it.
        assert!(auto_adopt_with(&ages, ts).unwrap().is_empty());

        // Removing a project from the list must stick: no re-tracking.
        let id = with_store(|s| Ok(s.project_by_session(old)?.unwrap().id)).unwrap();
        set_archived(&id, true).unwrap();
        assert!(
            auto_adopt_with(&ages, ts).unwrap().is_empty(),
            "an archived project must not come back on the next tick"
        );

        for s in [old, fresh] {
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

    /// architecture.md Must: never hold the store lock while observing tmux
    /// (board #149; the capture tick once queued every RPC behind a tmux walk,
    /// 2026-09-03, and adopt/list did the same in smaller doses — measured here:
    /// an adopt held the lock 90 ms, a list ~9 ms per project). Every
    /// `with_store(|…| …)` closure in this file must be SQLite only. The one
    /// deliberate exception carries `// store-lock(tmux-ok):` with its reason
    /// (rename keeps the session rename and the row re-key under one lock).
    #[test]
    fn no_tmux_work_runs_under_the_store_lock() {
        // Every file of the projects family (board #152 moved the closures out
        // of mod.rs; the rule follows them), production text only.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/projects");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
        files.sort();
        let mut checked = 0;
        let mut bad = Vec::new();
        for path in &files {
            let src = std::fs::read_to_string(path).unwrap();
            let prod = match src.find("\n#[cfg(test)]\n") { Some(k) => &src[..k], None => &src[..] };
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut at = 0;
            while let Some(k) = prod[at..].find("with_store(|") {
                let start = at + k;
                let mut depth = 0i32;
                let mut end = start + "with_store".len();
                for (off, c) in prod[start + "with_store".len()..].char_indices() {
                    depth += (c == '(') as i32 - (c == ')') as i32;
                    if depth == 0 {
                        end = start + "with_store".len() + off + 1;
                        break;
                    }
                }
                let body = &prod[start..end];
                checked += 1;
                let touches_tmux = ["tmux::", "capture::observe", "session_workspace("].iter().any(|t| body.contains(t));
                if touches_tmux && !body.contains("// store-lock(tmux-ok):") {
                    let line = prod[..start].matches('\n').count() + 1;
                    bad.push(format!("{name}:{line}"));
                }
                at = end;
            }
        }
        assert!(checked >= 40, "scan found only {checked} with_store closures");
        assert!(bad.is_empty(), "tmux work under the store lock at {bad:?}");
    }

    /// Two threads first-calling a cold store together must produce ONE open:
    /// the migration ladder has rebuild steps (deliveries v20/v21 DROP+RENAME)
    /// that are not safe to run twice at once (board #150; reproduced in #149's
    /// probe as "migrate to 1: table projects already exists"). The open here
    /// is slow on purpose so both callers are inside the cold window.
    #[test]
    fn a_cold_store_is_opened_exactly_once_under_a_race() {
        static CELL: OnceLock<Mutex<Store>> = OnceLock::new();
        static OPENS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let slow_open = || {
            OPENS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(80));
            Store::open_memory()
        };
        let a = std::thread::spawn(move || open_once(&CELL, slow_open).map(|_| ()));
        let b = std::thread::spawn(move || open_once(&CELL, slow_open).map(|_| ()));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        assert_eq!(OPENS.load(std::sync::atomic::Ordering::SeqCst), 1, "both first callers opened (and would have migrated) the store");
    }
}
