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

`project_create` had to learn the same lesson a second time (owner report,
2026-08-19): the schema had moved uniqueness to `session`, but `create` still
opened with "is there a project at this canonical path? → return it", so
creating a NEW project inside a directory some existing project already used
silently merged into that project — the dialog said created, the store said
nothing happened. Dedup now follows identity: only the literal same request
(same wanted session name AND same canonical path) is idempotent and returns
the existing row (un-archiving it); a session-name clash at a different path
falls through to `free_session_name`'s suffixing, and the id is salted past a
collision so even same-name-same-path coexistence works when an explicit
session distinguishes them. `project_by_path` was deleted outright so
path-identity cannot creep back in.

Migrations run with `PRAGMA foreign_keys=OFF` and this is not optional:
libsqlite3-sys builds its bundled SQLite with `SQLITE_DEFAULT_FOREIGN_KEYS=1`,
so enforcement is ON by default, and a schema rebuild's `DROP TABLE projects`
then performs an implicit `DELETE FROM` that cascades every child row away
(slots, and the snapshots table that existed at the time). The migration test
caught exactly that.

The steps run in ONE transaction with the `user_version` stamp inside it
(`Store::migrate` → `migrate_steps`, 2026-09-03 review). Before, each step
autocommitted and the stamp was written last, so a failure after step k left
the database half-migrated but still stamped old: the next open re-ran step k,
hit "table already exists" (the plain `CREATE TABLE`s are deliberately not
idempotent — a step is a one-time schema change, not a floor; `heal()` is the
floor), and `Store::open` failed for ever. Now the database is either fully at
`SCHEMA_VERSION` or exactly as it was, and `a_failed_migration_step_rolls_every_step_back`
pins it. Two consequences: `init` sets `PRAGMA foreign_keys=OFF` BEFORE the
transaction because SQLite ignores that pragma inside one, and no step may
toggle it — the v7 step used to switch it back ON, which then applied to every
later step's rebuild.

The reverse direction, `capture.rs`, is why nobody hand-writes a project: a
20-second loop folds live tmux back into the declaration.

### Renaming moves the session too, and never the room

`project_rename` (RPC, `projects::rename`, `tmm project rename <session> --name`)
updates `projects.name` AND renames the tmux session to `slug(name)`. The Hub's
chat header is the control: the title is a button that becomes an input in place,
Enter/blur commits, Escape cancels.

The session has to follow, because it is the name the Terminal's header and
`tmux ls` show — renaming only the label left one project wearing two names
(owner, 2026-08-19: "没有改tmux session的名字 所以在terminal显示不对"). Three
things make that safe, and all three are the reason the first version did not do
it:

- **The chat room is recorded on the project, not derived from the session**
  (`projects.room`, schema v8, backfilled as `proj:<session>`). The room id was
  `proj:<session>`, so a rename would have orphaned the conversation. Now the id
  is frozen at birth and `project_room()` reads it, falling back to the derived
  form for anything older.
- **The previous session name keeps resolving** (`projects.prev_session`;
  `project_for_session` checks it after the current name). A running agent has
  `TMM_PROJECT=<session>` baked into its environment and a process cannot be told
  otherwise, so without this every `tmm send/status/done` from an agent that was
  already up would fail until someone restarted it. Only the most recent previous
  name is kept — two renames in a row leave the oldest unresolvable, which costs a
  restarted agent nothing and keeps this to one column instead of a table.
- **tmux goes first.** If `rename-session` fails (the name is taken by a session
  no project claims), the declaration is left alone rather than drifting away from
  the session it projects onto. `free_session_name` picks a free name first, so
  this is the belt to that braces.

There is no exception for **adopted** projects, and the first version's was a
mistake worth remembering: it skipped them on the theory that their session name
is their owner's. But `auto_adopt_once` adopts every untracked session
automatically — that is the migration path and the "every session is a project"
rule — so `adopted` mostly means "the app found it before it was declared", not "a
human chose this name". On the owner's own machine 2 of 4 projects were adopted,
including the one being renamed, so the exception silently disabled the feature
exactly where it was asked for ("tmux不能改名字吗", 2026-08-19). A rename typed
into our UI is the instruction; nothing else needs consulting.

Client-side, everything keyed by the session name follows. The Hub re-selects the
project under its new name and `hubPrefs.renameSession` moves the remembered lead
and the read marker (both keyed by session). The Terminal is the case that cannot
be reached from there: it may be showing `<old session>:<window>.<pane>`, which
stops resolving the instant tmux renames the session, so the Hub dispatches a
`project-renamed` event and App remaps the live target and every split cell
through `retarget` (pure, tested — only an exact session-name prefix moves, so
`older:1.0` is left alone).

## The capture rule## The capture rule

**A window must survive `SETTLE_SECS` (120 s) before it becomes restorable.**
The window you opened to grep one file and closed again must not reappear on
every future `up`. Unsettled slots are still persisted — `first_seen_at` has to
survive a server restart — but `up` skips them and the UI does not show them as
windows it will restore. A window that disappears simply leaves the declaration:
it is not part of the workspace any more.

There is no history beyond that, on purpose. **The declaration IS the last
observed state** — closing a project does not touch it (capture only reads live
sessions) and a restart reads it back — which is exactly the "state before I
closed it / before the reboot" people want. A 20-deep topology history existed
here briefly and was removed: two days of real use produced one snapshot per
project (the one written at adopt, identical to the current declaration), what
people actually want back is *content* rather than window names (covered by the
pane's scrollback and by agent resume), and its `restore` could not deliver
anyway — it rewrote the declaration without projecting it, so on a live project
the next capture tick threw the restored rows away. The only real cost of the
removal: changes in the ≤20 s before a crash are lost.

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
database). Tables: `projects` and `slots`, with `PRAGMA user_version` as the
migration marker.

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
agent's icon. While the project is down they come from the declaration instead
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
  their original directories.
- Resume was verified with a real agent, not a mock: a kiro conversation was
  told to remember the word PINEAPPLE, the session was tracked, killed and
  reopened, and the restored agent — in a brand-new pane — answered PINEAPPLE
  when asked what it had been told to remember. The slot held the same
  conversation id that `kiro-cli chat -l` listed for that directory.

## Known edges

- `autostart` is stored but nothing acts on it yet (decision 3: manual restore
  first).
- A window renamed by hand looks like "old window gone, new window appeared":
  the old slot leaves the declaration and the new one has to settle again.
- `up` restores windows, not panes: a split layout inside a window is not part
  of the declaration.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### Projects declare, tmux projects

a project (`state.db`, `src-tauri/src/projects/`) is a directory + the windows it is made of; the tmux session is a disposable projection.

**Every session is a project**: `auto_adopt_once` on the capture tick adopts anything untracked (that is also the migration for pre-existing sessions), guarded by a 120 s session age, the `tmm-team-` prefix (Team owns those), and "archived is never re-adopted". There is ONE create path — the `+` calls `project_create` + `project_up`, and its agent presets seed an agent slot rather than a raw command, because a shell command is observed-only and never replayed. `up` matches windows BY NAME and only creates what is missing (the session may be the one you're typing in), `down` kills the session and keeps the declaration, and the capture loop folds live tmux back in — nobody hand-writes a project. One rule keeps it honest: a window must survive 120 s to become restorable, and a vanished window just leaves the declaration. There is NO topology history — the declaration is the last observed state, which is what "restore what I had before closing/rebooting" means; a 20-deep snapshot list was removed as dead weight (its `restore` rewrote the declaration without projecting it, so a live project's next capture tick undid it).

**Identity is the SESSION, not the path** — several sessions parked in `$HOME` is normal, so `UNIQUE` sits on `session`; the workspace dir comes from `pick_workspace` over ALL windows (most frequent, `$HOME` only as a last resort), never from the focused pane. Migrations MUST `PRAGMA foreign_keys=OFF`: libsqlite3-sys defaults them ON, so a table rebuild's `DROP TABLE` cascades the children away. Only agent slots are relaunched: an observed `npm run dev` is recorded, never replayed. `projects/agents.rs` is ONE table for detection and relaunch so they cannot disagree (earliest match in the pane text wins — a later match is a subprocess); for MANAGED windows `detect_managed` reads the backend off the launch recipe FIRST, because the sniff lies for our own spawns — the npm codex runs as `node` with nothing saying "codex" anywhere, so a spawned codex fell out of delivery/roster/vitals/recovery until the record beat the sniff (measured 2026-08-22).

**An agent resumes its conversation, not a blank prompt**: the notify hooks already carry `session_id`, the hub keeps the last one per tmux window, the capturer stamps it onto the slot (sticky — a quiet cycle must not erase it), and `up` prefers `--resume-id`/`--resume <id>`/`codex resume <id>`/`grok --resume <id>` over the directory-scoped `kiro-cli chat --resume` / `claude --continue`.

**A managed agent restarts with its FULL identity**: `spawn` persists a launch recipe (`launch.json` in the isolated home: env + identity command, kick stripped) and the restart path replays it (`spawn::relaunch_line`, used by `reconcile::slot_command_in`); `refresh_hooks` backfills the recipe for pre-recipe kiro agents. Without the recipe the restart ran the bare backend line — no KIRO_HOME, no `--agent` — i.e. the user-space config whose hooks NEVER fire, so a restarted agent kept answering but went observably deaf: no tool rows, no auto-post, every delivery "unconfirmed" (owner report 2026-08-18). `codex resume --last` is banned: it is machine-wide, so it would reopen another project's conversation. Desktop-only, `rusqlite` gate == `mod projects` gate; `team.db` stays the agora bus. See `docs/design-docs/features/projects.md`.

### A project is named by its NAME, never by its folder

`project_create` derives the session as `session ?? name ?? basename(path)` — the folder is the LAST resort, because falling straight to it produced a project called "src-tauri" and a session called "tmp" for `--name closetest` (two owner reports, one bug seen from both ends). The Hub's create dialog also REQUIRES a name.

**Renaming moves the tmux SESSION too** (`project_rename` / `tmm project rename`; the chat header's title is the control — a button that becomes an input in place), because the session name is what the Terminal and `tmux ls` show and a label-only rename left one project wearing two names (owner, 2026-08-19). Three things make that safe: the chat room is RECORDED on the project (`projects.room`, schema v8, backfilled `proj:<session>`) instead of derived from the session, so the conversation cannot be orphaned; `projects.prev_session` keeps the old name resolving in `project_for_session` and RESERVED from reuse by another project, because a running agent has `TMM_PROJECT` baked into its env and would otherwise go mute until restarted; and tmux is renamed FIRST, so a refusal leaves the declaration matching reality. There is NO adopted exception (the first cut had one and it disabled the feature on most real projects: `auto_adopt_once` adopts every untracked session, so `adopted` means "found before declared", not "a human named it"). Client-side `hubPrefs.renameSession` moves the per-session lead/read-marker keys, and a `project-renamed` event lets App `retarget` the live terminal + split cells, whose `<session>:<win>.<pane>` would otherwise be dead. The chat header then shows exactly one of `Open`/`Close` (`project_up`/`project_down`) depending on whether a live session exists; Close shares the agent-stop confirmation because it kills every pane. The path comes from `DirPicker` (`src/lib/files/DirPicker.svelte`) — the file browser's own `fs_list` RPC, directories only, read-only — so nobody types an absolute path from memory.

### Stop pauses, remove ejects; archive hides, delete forgets

`hub_agent_remove` / `projects::agent_remove` kills the window, DROPS THE SLOT so `up` cannot recreate it, and deletes the isolated home so `is_managed_in` stops recognising it — and it removes whatever of those three is still there, refusing only when NOTHING of the agent is left in the project: requiring a managed home made a STOPPED agent (slot, no window) and a home-less declaration unremovable, so `up` kept recreating a window nobody wanted (owner, 2026-08-19). The slot is membership, so the slot is what authorizes removal; the Hub's stopped card therefore carries the same dot menu (Start / Remove).

**And the roster ROW outlives its cards**: the `+ agent` button is the strip's STICKY last card — inside the family ("加agent应该放到最后，和其他agent放到一起…不用强行一直占一个位置", owner 2026-08-25; icon-only `+` while agents exist, labelled only in an empty roster) yet `position: sticky; right: 0` so it can never scroll invisible, which is the bug that first exiled it to a pinned slot (owner, 2026-08-21) — and the row renders for every SELECTED project — empty roster and CLOSED session included, and the empty-feed preset panel drops the same gate. Two gates hid the one entry point to an empty room, each in a different situation: gated on a non-empty roster it vanished with the last agent (the preset panel that would have covered it only renders in an EMPTY feed, so a room with history had no `+` at all), and gated on a live session a closed project still had none (owner, 2026-08-24, twice). A closed session was never a real constraint: `projects::spawn` calls `tmux::ensure_session` itself, so a spawn into a project that is down OPENS it (verified live — `hub_spawn` into a `○` project returned `{window_name, pane}` and the session appeared).

**That dot menu is a context menu**, a `position: fixed` layer placed from the trigger's rect (`menuPlacement` — right-aligned, flips above, clamps to the viewport; divide the rect by `--ui-zoom` because a client rect is visual px while a fixed child's `left` is zoomed px), never a popover INSIDE the horizontally-scrolling roster, which would clip it — that clipping is why it was a full-width bar until the owner asked for "类似右键菜单" (2026-08-19); it dismisses on outside pointerdown / Escape / roster scroll / resize.

**Every transient layer follows that rule** (owner, 2026-08-22: "在其他操作之后应该自动隐藏 不应该一直常驻显示"): the message action row (copy/raw under a tapped bubble) and the recipient picker close on outside pointerdown + Escape, copy closes its row after the "Copied" beat, and a tap outside the composer parks the `/` palette exactly like Escape (paletteOff, reset by the next text change). Raw view is deliberately NOT a popup — an opened raw source stays until retoggled. Modal dialogs keep their backdrop-click dismissal. `project_delete` / `projects::delete` closes the session, removes every `<path>/.tmm/agents/<name>/` and forgets the row (slots cascade) — but in the chat UI it is reachable only through the RECYCLE BIN (owner, 2026-08-21, same two-step rule as messages): the header's Delete does `project_down` + `project_archive` and the project waits in a folded "回收站" section at the sidebar's bottom (hidden while empty), where restore is free and the confirmed `project_delete` is the only irreversible step; archived is never re-adopted, which keeps the bin stable. Both deliberately spare the user's files (we only delete inside `.tmm/agents/`) and the chat history (the room is the record, keyed by session name).

**CLI/UI parity is a rule, not a nice-to-have**: every one of these verbs exists as `tmm agent interrupt|stop|restart|remove` and `tmm project up|down|archive|delete`, because an agent that can only be managed by a human cannot manage a teammate.
