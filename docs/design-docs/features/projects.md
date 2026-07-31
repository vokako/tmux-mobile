# Projects — a workspace you can close and reopen

> Implemented: P0 of [`docs/exec-plans/projects-and-tasks.md`](../../exec-plans/projects-and-tasks.md).
> Tasks, the board and scheduling are later phases and are NOT here yet.

## Problem

A tmux session exists only inside the running tmux server. A reboot, a crash or
one stray `kill-server` and the machine forgets which workspaces were open,
where each was rooted and which agent CLI was running in it. Rebuilding that by
hand costs enough that closing a workspace never feels free, so sessions only
accumulate — the entropy the owner reported.

Team sessions (`tmm-team-*`) already had a durable registry and startup
recovery. Ordinary sessions — the ones the user actually lives in — had nothing.

## Shape

**The declaration is the truth; the tmux session is a disposable projection.**

```
Project = { path, name, session, slots[] }
slot    = one window's intent = { window_name, cwd, kind: shell | agent }
```

Three verbs, all in `src-tauri/src/projects/`:

- **`up`** (`reconcile.rs`) — ensure the session exists, then give every settled
  slot a window. Idempotent *by construction*: windows are matched **by name**
  and only missing ones are created. Nothing is renamed, reordered or
  restarted, because the session being reconciled may be the one the user is
  typing in right now. A freshly created session already owns one window named
  after the shell; the first slot takes it over (`tmux::rename_window`) instead
  of leaving a stray behind.
- **`down`** — kill the session, keep the declaration. This is what makes
  closing a workspace free.
- **`adopt`** — take a session the user made by hand, keep its name, and settle
  its current windows immediately. An adopted project must be restorable even
  if the machine reboots a minute later.

### Every session is a project

There is no such thing as an untracked session any more, and there is one way to
make a workspace. Two consequences fall out of that:

- **Auto-tracking.** `auto_adopt_once` runs on the capture tick and adopts any
  session that isn't a project yet — so `tmux new -s foo` in a terminal shows up
  as a project by itself, and on first run this is also the migration for
  sessions that already existed. Three guards keep it from becoming a new source
  of entropy: a session must have existed for `SESSION_SETTLE_SECS` (120 s, the
  same reasoning as the window settle rule one level down — a workspace is
  something you come back to, a two-minute shell is not); team sessions are never
  adopted (Team creates and kills its own, so a declaration would fight it); and a
  project you ARCHIVED is never re-adopted, or "remove from projects" would undo
  itself on the next tick.
- **One create path.** The old "new session" form now creates a *project* and
  brings it up (`project_create` + `project_up`), so the second `+` in the
  Projects header is gone. A bare `new_session` would have been pointless anyway:
  the auto-tracker would adopt it seconds later. The form's Kiro/Claude/Codex
  presets became `agent` — they seed one settled agent slot instead of a raw
  command line, which is what makes the agent relaunch AND resume on every later
  `up`. Free-form commands were dropped from the form on purpose: a shell command
  is observed-only and never replayed, so offering to type one there would
  promise something `up` does not do.

The Track button is gone with them: tracking is not a decision the user has to
make per session any more.

### Identity is the session, not the directory

v1 made `path` UNIQUE and derived a project's directory from the session's
**active pane**. Both were wrong, and together they made most real sessions
un-trackable:

- Several sessions parked in `$HOME` is the normal case, not a conflict. With
  `UNIQUE(path)` the first one adopted claimed `~` and every later one failed
  with *"`/Users/me` is already project 'Default'"*.
- The focused window is often a shell in `$HOME` while the actual work sits in
  another window. A session whose second window ran an agent in
  `~/work/poc/260728-ds160` was recorded as a project rooted at `~`.

So (schema v2): uniqueness lives on `session` — two projects fighting over one
tmux session name is the real conflict — `path` is merely indexed, and the
workspace directory is decided by `pick_workspace` over ALL windows: most
frequent cwd wins, `$HOME` only wins when nothing else is on offer, ties break
toward the shortest path (the one closest to a project root when windows sit in
sibling subdirs). A window that does sit outside the project keeps an absolute
cwd, so it is restored where it was.

Migrations run with `PRAGMA foreign_keys=OFF` and this is not optional:
libsqlite3-sys builds its bundled SQLite with `SQLITE_DEFAULT_FOREIGN_KEYS=1`,
so enforcement is ON by default, and a schema rebuild's `DROP TABLE projects`
then performs an implicit `DELETE FROM` that cascades every slot and snapshot
away. The migration test caught exactly that.

The reverse direction, `capture.rs`, is why nobody hand-writes a project: a
20-second loop folds live tmux back into the declaration.

## The two capture rules

Both exist because a declaration that remembers everything is as useless as one
that remembers nothing.

1. **A window must survive `SETTLE_SECS` (120 s) before it becomes
   restorable.** The window you opened to grep one file and closed again must
   not reappear on every future `up`. Unsettled slots are still persisted —
   `first_seen_at` has to survive a server restart — but `up` skips them and
   the UI does not show them as windows it will restore.
2. **A window that disappeared leaves the declaration, but its topology stays
   in `snapshots`.** So "give me back yesterday's layout" is still answerable
   without the declaration becoming a graveyard of soft-deleted rows.

A snapshot is written only when the *topology* changed (window set, order, cwd,
agent), never when a slot merely settled — otherwise every project would
snapshot itself twice a minute. The newest 20 per project are kept.

## What `up` will and will not run

Only agent slots are relaunched. An observed `npm run dev` or `vim` is recorded
on the slot for display and **never replayed**: restoring a workspace must not
re-execute whatever you happened to be running last time. Each slot carries an
`auto_run` flag — set for agents, clear for shells — so this stays a data
decision rather than a special case in the reconciler.

`agents.rs` is one table used for **both** detection and relaunch, so the two
can never disagree; a detector that recognises "codex" but relaunches something
else would quietly rebuild the wrong workspace. Detection takes the EARLIEST
match in the pane's command text (`pane_current_command`, title, then the
foreground child's argv), mirroring the client's `detectAgent`: a later match is
a subprocess the agent spawned — a real case was codex spawning a
`kiro-web-search` helper and being labelled Kiro. Claude Code needs no special
case even though its process name is a bare version number, because its argv
path contains `.../claude/versions/<v>`.

## Restoring the conversation, not just the window

A rebuilt window with a blank agent prompt is only half a restore: the point of
reopening a workspace is to carry on. So an agent slot remembers **which
conversation** it was in and `up` resumes it.

Flags come from the installed CLIs' own `--help`, not from memory:

| backend | exact conversation | newest in this directory |
|---|---|---|
| kiro | `kiro-cli chat --resume-id <id>` | `kiro-cli chat --resume` |
| claude | `claude --resume <id>` | `claude --continue` |
| codex | `codex resume <id>` | — (see below) |

`agents::launch_line` picks in that order: exact id → directory resume → clean
start. Two decisions inside it are deliberate:

- **`codex resume --last` is not used.** It continues the most recent recorded
  session *machine-wide*, not per directory, so restoring project A could reopen
  project B's conversation. Without a recorded id, codex starts fresh.
- **kimi and openclaw get no resume flags** because theirs are unverified here.
  Relaunching clean is honest; guessing a flag is not.

Where the id comes from: the agent lifecycle hooks already carry `session_id`
(`agent_notifications.rs` has been normalising it into `agent_session_id` all
along, keyed by tmux session + window). The hub now keeps the last id per window
in memory and the capturer stamps it onto the slot, so the durable copy lives in
`state.db` — which is the point, since the thing that has to survive is exactly
the reboot that loses tmux. `projects` never names the notification types: it
declares an `AgentSessions` trait and the hub implements it.

The stamp is **sticky**. A hook only reports at the end of a turn, so an
observation without an id must not erase what we know; a *different* id replaces
it, and a window that stops being an agent drops it.

Known edge: a slot has no id until a hook fires while the project is tracked. An
agent that has been idle since the server started falls back to the
directory-scoped resume — correct for kiro and claude, a clean start for codex.

## Storage

`state.db` — SQLite, ours, at `$XDG_CONFIG_HOME/tmux-mobile/state.db`
(`TMM_STATE_DB` overrides it; that is how the tests stay off the real
database). Tables: `projects`, `slots`, `snapshots`, with
`PRAGMA user_version` as the migration marker.

Two boundaries worth keeping:

- **`team.db` is not touched.** That is the vendored `agora` bus schema; we do
  not mix our tables into a library's database.
- **Files hold what a human writes, SQLite holds what the machine observed.**
  Slot cwds are stored *relative* to the project path, so moving a workspace
  keeps the declaration valid. When P2 introduces real agent definitions they
  will live in files (git-trackable, able to carry `skills/` assets) and the DB
  will reference them by name — no second copy.

Desktop-only, like the team supervisor: the phone is a client of a desktop
server, so `rusqlite` is excluded from mobile builds entirely. The gate on
`mod projects` matches the dependency gate in `Cargo.toml` exactly, and
`handle_project_request` is the single place where mobile answers
method-not-found — the same contract the Team tab uses to hide itself.

## UI

`src/lib/projects/Projects.svelte` renders above the raw session list on the
Sessions page, and the two halves divide the world between them: **a tracked
session appears only in Projects, everything else only in the session list.**
Since every session becomes a project on its own, what remains in the session
list is the short-lived (younger than the settle window), team sessions, and
anything you deliberately removed from Projects. One session, one home.

A project row is name + live dot + path, with its **windows on their own row as
individual buttons**: a project is a set of windows and jumping into the one you
want is the whole point. While the session is live the buttons come from the live
panes — the source of truth for what you can actually open, including a window
that has not settled into the declaration yet — and each carries the running
agent's icon plus an attention dot when that window has an unread agent
notification. While the project is down they come from the declaration instead
(dimmed, no targets yet) and tapping one brings the project up and lands you in
that window.

Tracking is automatic (see above), so there is no per-session Track control and
no separate "new session" form: the single `+` creates a project and brings it
up. The panes the Sessions page already polls are passed down as a prop, so the
window buttons cost no extra RPC.

Display logic that is worth testing (row ordering, which windows to show and
where each one points, path shortening) lives in `projects.ts` so `node --test`
can reach it without a DOM.

## Verification

- `cargo test --lib` — 25 project tests, including an end-to-end case against a
  real tmux server: adopt a two-window session, `down` it, `up` it, and assert
  both windows return with the right relative cwd; plus a second `up` that must
  change nothing, two sessions sharing one directory becoming two projects, and
  a v1→v2 migration that must keep its children.
- The pure rules (settle, removal, reorder, agent detection) are unit-tested
  without tmux.
- The UI flow was exercised in a real browser against a live server: track
  `demoproj` → the session dies on Close → Open rebuilds `editor` and `api` at
  their original directories → the layout-history menu lists the snapshot.
- Resume was verified with a real agent, not a mock: a kiro conversation was
  told to remember the word PINEAPPLE, the session was tracked, killed and
  reopened, and the restored agent — in a brand-new pane — answered PINEAPPLE
  when asked what it had been told to remember. The slot held the same
  conversation id that `kiro-cli chat -l` listed for that directory.

## Known edges

- `autostart` is stored but nothing acts on it yet (decision 3: manual restore
  first).
- A window renamed by hand looks like "old window gone, new window appeared" —
  the old one's history stays in snapshots, the new one has to settle again.
- `up` restores windows, not panes: a split layout inside a window is not part
  of the declaration.
