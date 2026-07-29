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
pub mod store;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use store::{Project, Slot, Store};

/// How often the capturer folds live tmux state back into the declaration.
const CAPTURE_INTERVAL: Duration = Duration::from_secs(20);

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

/// Every project, each with its slots and whether its session is live, plus the
/// tmux sessions no project claims yet (the adopt candidates).
pub fn list(include_archived: bool) -> Result<Value, String> {
    let (projects, unmanaged) = with_store(|store| {
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
        let claimed: Vec<String> = store
            .list_projects(true)?
            .into_iter()
            .map(|p| p.session)
            .collect();
        let unmanaged: Vec<String> = tmux::list_sessions()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .filter(|name| !claimed.contains(name))
            .collect();
        Ok((out, unmanaged))
    })?;
    Ok(json!({ "projects": projects, "unmanaged": unmanaged }))
}

/// Create a project for a directory. Idempotent: an existing project for the
/// same canonical path is returned (and un-archived) instead of duplicated.
pub fn create(path: &str, name: Option<&str>) -> Result<Value, String> {
    let path = canonical(path)?;
    let name = name.unwrap_or_else(|| basename(&path)).to_string();
    with_store(|store| {
        if let Some(existing) = store.project_by_path(&path)? {
            if existing.archived {
                store.set_archived(&existing.id, false, now())?;
            }
            return Ok(json!(existing));
        }
        let id = format!("{}-{}", slug(basename(&path)), digest(&path));
        let session = free_session_name(store, &slug(basename(&path)), &id)?;
        let project = Project {
            id: id.clone(),
            name,
            path,
            icon: None,
            session,
            adopted: false,
            autostart: false,
            created_at: now(),
            last_up_at: None,
            last_seen_at: None,
            archived: false,
        };
        store.insert_project(&project)?;
        Ok(json!(project))
    })
}

/// Adopt a live tmux session: the user created it, so we keep its name and take
/// its current windows as the declaration straight away — an adopted project
/// must be restorable even if the machine reboots one minute later.
pub fn adopt(session: &str, name: Option<&str>) -> Result<Value, String> {
    if !tmux::session_exists(session) {
        return Err(format!("no such tmux session: {session}"));
    }
    let path = session_cwd(session)?;
    let path = canonical(&path)?;
    let ts = now();
    with_store(|store| {
        if let Some(existing) = store.project_by_path(&path)? {
            return Err(format!(
                "{} is already project '{}' (session {})",
                path, existing.name, existing.session
            ));
        }
        if store.session_taken_by_other(session, "")? {
            return Err(format!("session {session} already belongs to another project"));
        }
        let id = format!("{}-{}", slug(session), digest(&path));
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
        let observed = capture::observe(session, &path)?;
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
        store.add_snapshot(&id, ts, &slots, capture::SNAPSHOT_KEEP)?;
        Ok(json!({ "project": project, "slots": slots }))
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

pub fn set_autostart(id: &str, autostart: bool) -> Result<Value, String> {
    with_store(|store| {
        store.set_autostart(id, autostart)?;
        Ok(json!({ "id": id, "autostart": autostart }))
    })
}

pub fn snapshots(id: &str) -> Result<Value, String> {
    with_store(|store| Ok(json!(store.snapshots(id)?)))
}

/// Replace the declaration with a stored topology. The session is left alone —
/// call `up` afterwards to project the restored declaration onto tmux.
pub fn restore(id: &str, snapshot_id: i64) -> Result<Value, String> {
    let ts = now();
    with_store(|store| {
        let slots = store
            .snapshot_slots(id, snapshot_id)?
            .ok_or_else(|| format!("snapshot {snapshot_id} not found for {id}"))?;
        let slots: Vec<Slot> = slots
            .into_iter()
            .map(|mut s| {
                s.id = None;
                s.settled_at = Some(ts);
                s
            })
            .collect();
        store.replace_slots(id, &slots)?;
        Ok(json!({ "id": id, "slots": slots }))
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

/// The working directory a session represents: its active pane's cwd.
fn session_cwd(session: &str) -> Result<String, String> {
    let panes = tmux::list_panes(session)?;
    panes
        .iter()
        .find(|p| p.active)
        .or_else(|| panes.first())
        .map(|p| p.current_path.clone())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| format!("cannot determine a directory for session {session}"))
}

// ---- the capture loop ---------------------------------------------------

/// Fold live tmux state into every live project's declaration once.
/// Returns the ids that were written.
pub fn capture_once() -> Result<Vec<String>, String> {
    let ts = now();
    with_store(|store| {
        let mut touched = Vec::new();
        for project in store.list_projects(false)? {
            if !tmux::session_exists(&project.session) {
                continue;
            }
            store.mark_seen(&project.id, ts)?;
            let observed = match capture::observe(&project.session, &project.path) {
                Ok(o) => o,
                Err(_) => continue, // session vanished mid-scan; next tick retries
            };
            let existing = store.slots(&project.id)?;
            let merged = capture::merge(&existing, &observed, ts, capture::SETTLE_SECS);
            if !merged.dirty {
                continue;
            }
            store.replace_slots(&project.id, &merged.slots)?;
            if merged.topology_changed {
                let settled: Vec<Slot> =
                    merged.slots.iter().filter(|s| s.is_settled()).cloned().collect();
                if !settled.is_empty() {
                    store.add_snapshot(&project.id, ts, &settled, capture::SNAPSHOT_KEEP)?;
                }
            }
            touched.push(project.id);
        }
        Ok(touched)
    })
}

/// Background capturer, spawned once by the server.
pub async fn capture_loop() {
    loop {
        tokio::time::sleep(CAPTURE_INTERVAL).await;
        if let Err(e) = capture_once() {
            eprintln!("projects: capture failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The P0 acceptance criterion, end to end against a real tmux server:
    /// adopt a session the user made, kill it, and get it back.
    ///
    /// This is the only test that touches the process-wide store, so it owns
    /// the `TMM_STATE_DB` override. Any future test that needs the store must
    /// point at the SAME path — `STORE` is a `OnceLock`, so the first opener
    /// wins for the rest of the process.
    #[test]
    fn adopt_then_down_then_up_restores_the_workspace() {
        let root = std::env::temp_dir().join("tmm-proj-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::env::set_var("TMM_STATE_DB", root.join("state.db"));
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

        // The board only offers sessions nobody owns yet.
        let listed = list(false).unwrap();
        assert!(
            !listed["unmanaged"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == session),
            "an adopted session is no longer an adopt candidate"
        );
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

        // A snapshot exists from the adopt, and restoring it is a no-op here.
        let snaps = snapshots(&id).unwrap();
        let snaps = snaps.as_array().unwrap();
        assert_eq!(snaps.len(), 1);
        restore(&id, snaps[0]["id"].as_i64().unwrap()).unwrap();
        assert_eq!(with_store(|s| s.slots(&id)).unwrap().len(), 2);

        // Capturing a live project must not disturb a settled declaration.
        capture_once().unwrap();
        assert_eq!(with_store(|s| s.slots(&id)).unwrap().len(), 2);

        let _ = tmux::kill_session(session);
        let _ = std::fs::remove_dir_all(&root);
    }
}
