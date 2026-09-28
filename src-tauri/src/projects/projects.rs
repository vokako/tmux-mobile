//! Project verbs: create / adopt / up / down / rename / archive / delete and the session-workspace lookups — a project is a declaration, the tmux session its disposable projection.
//!
//! One family of `projects` (board #152): moved whole from mod.rs, which stays
//! the facade — every `projects::X` path is a re-export of this file.

use super::*;

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
pub(super) struct AdoptFacts {
    path: String,
    observed: Vec<capture::Observed>,
}

pub(super) fn adopt_facts(session: &str) -> Result<AdoptFacts, String> {
    let path = canonical(&session_workspace(session)?)?;
    let observed = capture::observe(session, &path, agent_sessions())?;
    Ok(AdoptFacts { path, observed })
}

/// The row half of an adoption: only SQLite inside. The already-tracked check
/// runs here, under the lock, so two adopters racing on one session still get
/// the same answer they always did.
pub(super) fn adopt_in(
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
pub(super) fn auto_adopt_with(created: &[(String, u64)], ts: u64) -> Result<Vec<String>, String> {
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

/// Restart ONE agent slot of a project (board #210): the window returns with
/// its full recipe and exact conversation id; no other slot is touched and
/// the project's `last_up_at` is not a project-level `up`, so it is left
/// alone. `Ok(false)` = no such agent slot yet (the capture loop has not
/// recorded it) or the window could not be created.
pub fn up_agent(id: &str, window_name: &str) -> Result<bool, String> {
    let (project, slots) = load(id)?;
    Ok(match reconcile::up_agent(&project, &slots, window_name) {
        Some(r) => r.status == "created" || r.status == "existing",
        None => false,
    })
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

pub(super) fn load(id: &str) -> Result<(Project, Vec<Slot>), String> {
    with_store(|store| {
        let project = store
            .project(id)?
            .ok_or_else(|| format!("no such project: {id}"))?;
        let slots = store.slots(id)?;
        Ok((project, slots))
    })
}

/// A session name that is free both in tmux and in the store.
pub(super) fn free_session_name(store: &Store, base: &str, id: &str) -> Result<String, String> {
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
pub(super) fn session_workspace(session: &str) -> Result<String, String> {
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
pub(super) fn pick_workspace(cwds: &[String], home: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::super::tests::use_test_store;
    use super::*;

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
        let mut scratch = tmux::Scratch::new("auto");
        use_test_store();
        let path = scratch.path();
        let old = &scratch.session("old");
        let fresh = &scratch.session("fresh");
        for s in [old, fresh] {
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
    }

    #[test]
    fn two_sessions_in_the_same_directory_are_two_projects() {
        let mut scratch = tmux::Scratch::new("share");
        use_test_store();
        let path = scratch.path();
        let (a, b) = (scratch.session("a"), scratch.session("b"));
        for s in [&a, &b] {
            tmux::ensure_session(s, &path).unwrap();
        }

        let first = adopt(&a, None).unwrap();
        // Used to fail with "<path> is already project ..." — several sessions
        // parked in one directory (typically $HOME) is the normal case.
        let second = adopt(&b, None).unwrap();
        assert_eq!(first["project"]["path"], second["project"]["path"]);
        assert_ne!(first["project"]["id"], second["project"]["id"]);

        let again = adopt(&a, None);
        assert!(
            again.is_err_and(|e| e.contains("already tracked")),
            "the same session twice is the real conflict"
        );
    }

    /// The P0 acceptance criterion, end to end against a real tmux server:
    /// adopt a session the user made, kill it, and get it back.
    ///
    #[test]
    fn adopt_then_down_then_up_restores_the_workspace() {
        let mut scratch = tmux::Scratch::new("e2e");
        use_test_store();
        let path = scratch.path();
        std::fs::create_dir_all(format!("{path}/api")).unwrap();
        let session = &scratch.session("s");

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
    }
}
