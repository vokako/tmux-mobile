# WebSocket JSON-RPC API Contract

## Transport
JSON-RPC over WebSocket (`ws://` or `wss://`).

## Authentication

### Encrypted Auth (default, when Web Crypto available)
```
← {"server_nonce": "<hex>"}                          // server sends nonce
→ {"method": "auth", "params": {"client_nonce": "<hex>", "proof": "<hex>"}}
← <binary AES-GCM frame containing the authenticated result>
```
- Key derivation: HKDF-SHA256(token, server_nonce + client_nonce)
- Proof: HMAC-SHA256(key, server_nonce)
- All subsequent messages are binary AES-256-GCM frames using the derived key
- Decrypted payload starts with a one-byte framing tag: raw UTF-8 JSON or raw-deflate JSON (compression is used only when it reduces payload size)
- Each direction uses a monotonic nonce counter; clients must serialize encrypted sends so wire order matches nonce order

### Legacy Plain Auth (fallback, no Web Crypto)
```
← {"server_nonce": "<hex>"}
→ {"method": "auth", "params": {"token": "..."}}
← {"result": {"authenticated": true, "machine_id": "uuid", "hostname": "my-mac"}}
```

### Rate Limiting
- Max auth failures tracked per IP
- Lockout after repeated failures (configurable timeout)

## Methods

### Session Management
| Method | Params | Response |
|--------|--------|----------|
| `ping` | — | `"pong"`. Used by the client's idle probe to detect half-open links. |
| `list_sessions` | — | Array of `{name, windows, attached, created, last_opened?}` objects. `last_opened` is unix seconds of the last time this session was opened via tmux-mobile (`subscribe` RPC); absent if never opened. |
| `list_panes` | `session` | Array of pane objects: `{session, window, pane, width, height, current_command, window_name, pane_title, current_path, active, child_cmd?}`. `current_path` is tmux `#{pane_current_path}`; `active` marks the window's active pane; `child_cmd` is the foreground descendant argv (detects interpreter-launched agent CLIs), omitted for bare shells. |
| `list_sessions_with_panes` | — | `{sessions, panes}` — the two lists above in one round-trip (saves 1+N RPCs on the Sessions page). |
| `new_session` | `name?`, `path?`, `command?` | OK |
| `kill_session` | `name` | OK |
| `new_window` | `session` | OK |
| `kill_window` | `target` | OK |

### Pane Operations
| Method | Params | Response |
|--------|--------|----------|
| `capture_pane` | `target`, `lines?` | `{output}` with ANSI colors |
| `send_keys` | `target`, `keys`, `literal` | OK |
| `paste_text` | `target`, `text` | OK. Real-terminal paste semantics via tmux `load-buffer` + `paste-buffer -p`: bracketed-paste markers (`\x1b[200~…\x1b[201~`) are added exactly when the pane app enabled mode `?2004`, so pasted newlines are not executed line by line. |
| `send_command` | `target`, `command` | OK |
| `pane_command` | `target` | `{command}` string |
| `resize_pane` | `target`, `cols`, `rows` | OK (auto-restores on disconnect) |
| `subscribe` | `target` | Starts polling, pushes `pane_output`. Side effect: stamps the session's `last_opened` timestamp (persisted to `session_usage.json`) for MRU sorting. |
| `unsubscribe` | `target` | Stops streaming |

### Filesystem
| Method | Params | Response |
|--------|--------|----------|
| `fs_cwd` | `session` | `{path}` |
| `fs_list` | `path`, `show_hidden?` | `{entries, path}` |
| `fs_stat` | `path` | FileStat object |
| `fs_read` | `path` | `{content}` (≤512KB) |
| `fs_write` | `path`, `content` | OK |
| `fs_mkdir` | `path` | OK |
| `fs_delete` | `path` | OK |
| `fs_rename` | `from`, `to` | OK |
| `fs_download` | `path` | `{name, data}` base64 (≤50MB). For inline preview; user-initiated downloads use `fs_download_url` + HTTP `/dl` streaming instead. |
| `fs_download_url` | `path` | `{url, name}` where `url` = `/dl?path=…&ts=…&sig=…`. Client GETs it on the same host (http:// for ws://, https:// for wss://) to stream the file. HMAC-SHA256 signature over token+path+ts, 60 s TTL. No server-side size limit. |
| `fs_upload` | `path`, `data` | OK |
| `fs_convert` | `path`, `format?` | `{html}` (currently only pptx→html) |

### Git
| Method | Params | Response |
|--------|--------|----------|
| `git` | `subcmd`, `args[]`, `cwd?` | `{stdout, stderr, code}` |

Whitelisted subcmds: status, diff, log, show, branch, rev-parse, push, add, commit, restore.
Shell metacharacters rejected in args.

### Config / Preferences
| Method | Params | Response |
|--------|--------|----------|
| `set_socket` | `socket` | OK |
| `get_bookmarks` | — | `{bookmarks}` array |
| `save_bookmarks` | `bookmarks` | OK |
| `get_prefs` | — | Preferences JSON |
| `set_pref` | `key`, `value` | OK |

### Agent hooks (agent lifecycle telemetry)
The `agent_notifications_list` / `agent_notifications_mark_read` unread-inbox
RPCs and the `agent_notification` push retired 2026-09-01 with the old
notification-dot UI: the project room's auto-post + read cursor and the derived
status dots are the one notification language. The hooks themselves remain — they
feed telemetry, status derivation and the auto-post.

| Method | Params | Response |
|--------|--------|----------|
| `agent_hooks_status` | — | Per-agent install state `{claude?: {installed}, codex?: {installed}, kiro?: {installed}}` |
| `agent_hooks_install` | — | Installs the notify hooks into agent configs; returns updated status |
| `agent_hooks_remove` | — | Removes them; returns updated status |

### Projects (desktop-only — method-not-found on servers without `state.db`)
A project is a workspace declaration; the tmux session is its projection. The
Projects section hides itself when these return -32601. See
[`docs/design-docs/features/projects.md`](../../design-docs/features/projects.md).

| Method | Params | Response |
|--------|--------|----------|
| `project_list` | `include_archived?` | `{projects: [{project, slots, live}]}` — the client subtracts these from `list_sessions` to get the untracked ones |
| `project_create` | `path`, `name?`, `session?`, `agent?` | The project (existing one for that path is returned, un-archived). `agent` seeds one settled agent window (`kiro`/`claude`/`codex`) |
| `project_adopt` | `session`, `name?` | `{project, slots}` — takes a live session's windows as the declaration |
| `project_up` | `id` | `{session, created_session, slots: [{window_name, status, error?}]}` |
| `project_down` | `id` | `{session, live: false}` — kills the session, keeps the declaration |
| `project_archive` | `id`, `archived?` | Hides/unhides without deleting |
| `project_rename` | `id`, `name` | `{id, name, session, session_renamed}` — renames the tmux session to `slug(name)` too. The chat room is recorded on the project so it does NOT move, the Board rows move transactionally to the new session key, and the previous session name stays reserved/resolving for agents already running with `TMM_PROJECT` (another project cannot reuse the alias) |
| `project_autostart` | `id`, `autostart?` | Flag only; no boot integration yet |
| `project_delete` | `id` | Forgets the project (slots cascade), closes the session, deletes `<path>/.tmm/agents/*` and its Board issues/note threads — the chat room and the user's files survive. Reached in the UI only through the recycle bin |
| `models_list` | `backend?` (default `kiro`) | `{backend, models}` — model ids the backend accepts, asked of its own CLI (cached). `models` is `null` when it cannot be enumerated (claude/codex), and the agent editor keeps free text |

### Project Hub (desktop-only — the `tmm` CLI's surface)
One chat room per project on the bus (`proj:<session>`), agent status
declarations, derived agent states. Consumed by the `tmm` CLI (agents and
humans) and the desktop hub UI. See
[`docs/design-docs/features/tmm-cli.md`](../../design-docs/features/tmm-cli.md).

| Method | Params | Response |
|--------|--------|----------|
| `hub_post` | `session`, `body`, `from?` (default `human`), `requires_reply?` | The stored message |
| `hub_log` | `session`, `since_ts?` (exclusive, epoch ms), `limit?` (default 100, cap 1000 PER PAGE — not a history horizon), `before_seq?` (exclusive; the page STRICTLY OLDER than that log position — 0/absent = the newest page) | `{messages: […], has_more, head_seq, oldest_seq?}` — oldest first; archived messages are filtered out. Nothing is ever pruned from a room, so `before_seq` = `oldest_seq` of the page you hold is the lazy-load step backwards. `has_more` is measured (the store is asked for one row more), so "that is the whole conversation" is distinguishable from "your page ended exactly at the limit". `oldest_seq` is the cursor for the next page back: a SURVIVING row's seq when the page has one, else the RAW page's oldest seq — a page can lose every row to the archive/`since_ts` filters, and `has_more: true` with no cursor would stop scroll-up dead at a hidden run. `seq` is globally monotonic across ALL rooms (one AUTOINCREMENT log), so gaps inside one room's sequence are normal — treat it as an opaque cursor, never as a count of what is missing. On a `since_ts` query `has_more` changes meaning: true iff rows NEWER than `since_ts` remain behind this page (the raw page did not reach back to the cursor) — the client then walks `before_seq` pages until one does, so a room that gained more than a page while unwatched has no hole (2026-09-03) |
| `hub_rooms` | — (the one hub method with no `session`) | `{rooms: {"<room>": ts_ms}}` — newest message per room, one grouped query; orders the project sidebar by conversation |
| `hub_msg_archive` | `session`, `ids[]` | `{archived: n}` — HIDES messages (they stay in team.db, so `hub_log` filters them out on the way). Reversible, so no confirmation. Bodies are looked up by exact id, so a message the user scrolled back to is as archivable as a fresh one |
| `hub_msg_restore` | `session`, `ids[]` | `{restored: n}` — take them back out of the archive |
| `hub_msg_purge` | `session`, `ids[]` | `{deleted: n}` — forgets them for good: deletes from team.db first, then drops the archive rows |
| `hub_archive` | `session` | `{messages: [{id, ts, from, body, archived_at}]}` — what is hidden, newest first; each row carries its own copy of the message |
| `hub_status` | `session`, `agent` (window name), `state` (`working\|waiting\|blocked`), `note?` | `{ok, window}` |
| `hub_done` | `session`, `agent`, `summary?` | `{ok, window}` — a summary is posted as `[tmm done] <summary>` FROM THE AGENT (a message, not app narration); a summary-less done posts the `[tmm] done` lifecycle line |
| `hub_command` | `session`, `agent` (window name or `all`), `text` (must start with `/`) | `{sent: [names], command}` — types the text VERBATIM into the managed agent's pane (no stamp, no sender): slash commands are read by the CLI, not the model. Recorded in the room as a `[tmm] ` lifecycle line |
| `hub_agents` | `session` | `{agents: [{window, name, command, agent, managed, state, detail, since, vitals}]}` — `vitals` is `{model, context_pct, effort, branch}` SNIFFED from the last lines of a managed agent's pane (a CLI's live state has no API): every field may be null, and `vitals` itself is null when nothing could be read |
| `hub_activity` | `session`, `since_ts?`, `limit?` (default 600, hard cap 1000 PER PAGE — not a history horizon), `before_ts?` + `before_id?` (exclusive cursor; the page strictly OLDER than it) | `{events: […], has_more, oldest: {ts, id}?, total, first_ts}` — oldest first; durable telemetry rows (tool calls, prompts, receipts, warns) the feed folds into tool lanes. The log is COMPLETE (nothing is pruned), so the read is the bounded half: pass `oldest` straight back as `before_ts`/`before_id` to walk as far back as the log goes. The cursor needs BOTH halves — a busy turn writes several events inside one millisecond; `before_id` omitted means "older than that whole millisecond" (always progresses, may skip same-ms siblings). `total`/`first_ts` describe what the server holds, for a "N of M loaded" affordance |
| `hub_spawn` | `session`, `agent` (registry name), `brief?`, `by?` (spawning agent, empty = human) | `{window_name, pane}` — materializes an isolated home, opens the session if it is down; capped at 4 agents/project; `by` needs `can_hire` and is recorded as `spawned_by` in the launch recipe (the done-summary feedback edge) |
| `hub_agent_interrupt` | `session`, `agent` | `{ok}` — resets the derived state FIRST (`record_interrupt`), then types the named `Escape` key into the pane; posts `[tmm] interrupted <name>` |
| `hub_agent_stop` | `session`, `agent` | `{ok}` — kills the window, keeps the slot (a stopped agent restarts via `up`/restart) |
| `hub_agent_restart` | `session`, `agent` | `{ok}` — replays the launch recipe (full identity: env, `--agent`, model in config) and resumes the recorded conversation |
| `hub_agent_remove` | `session`, `agent` | `{ok}` — ejects: kills the window, DROPS the slot, deletes the isolated home; refuses only when nothing of the agent is left |
| `hub_board_list` | `session` | `{issues: […], statuses}` — the project task board, four fixed columns (`todo/doing/review/done`); each issue carries a note COUNT. Public `id`/`#N` is a session-local sequence: every project's first issue is `#1`, deletes leave gaps, and numbers are never reused. Every single-issue operation resolves `session + id`; the database-wide row key remains internal for note FKs only |
| `hub_board_counts` | — (no `session`, like `hub_rooms`) | `{counts: {"<session>": {todo, doing, review, done, total}}}` — issue counts for EVERY board in one grouped read; the four statuses are zero-filled server-side, `total` is explicit, and a project with an empty board is ABSENT (absence = hide) |
| `hub_board_get` | `session`, `id` | The issue with its full `notes` thread and server-computed `editable` (`true` only while unassigned and before any Agent save/note activity) |
| `hub_board_save` | `session`, `id?`, `title?`, `body?`, `status?`, `assignee?`, `who?` | Create (no id) or PATCH (id + only the changed fields — COALESCE, so a `move` cannot erase a body edited meanwhile). Once assigned or touched by an Agent, title/body are immutable server-side; workflow fields remain patchable. `{ok, id}` |
| `hub_board_note` | `session`, `id`, `body`, `who?` | Appends to the issue's own thread, bumps `updated_at` |
| `hub_board_delete` | `session`, `id` | `{ok}` — deletes the issue and cascades its notes |

### Registry, skills & MCP defs (desktop-only)
Central definitions that `spawn` materializes into isolated agent homes. The
skill store is app-owned files under `<state dir>/skills/<name>/`; three
built-ins (`tmm-cli`, `mem`, `mcp-cli`) ship inside the binary and reseed at
server start (`source = "builtin"`; save/delete refuse their names).

| Method | Params | Response |
|--------|--------|----------|
| `registry_list` | — | `{agents: [{name, backend, model, effort, system, skills, mcp, can_hire}]}` |
| `registry_save` | `def` | Validates backend, model id (against the backend's own CLI) and effort enum |
| `registry_delete` | `name` | OK |
| `skills_list` | — | `{skills: [{name, source, description, synced_at}]}` |
| `skills_save` | `name`, `source`, `description?` | Imports/re-syncs the files from `source` (abs dir, github url, or `builtin`) |
| `skills_import` | `url` (or abs dir) | `{imported: [names]}` — walks the fetched tree for EVERY dir holding a SKILL.md (claude plugins/marketplaces work as-is); each row's source points at its own subdir; frontmatter names, built-in names skipped |
| `skills_read` | `name` | `{markdown}` — the managed SKILL.md |
| `skills_files` | `name` | `{files: [{path, size}]}` — every managed file of the skill |
| `skills_file` | `name`, `path` | `{content}` — 256 KB cap, text only, path escapes rejected |
| `skills_refresh` | `name` | Re-syncs from the recorded source (builtin = the running build) |
| `skills_delete` | `name` | OK (refused for built-ins) |
| `mcp_list` / `mcp_save` / `mcp_delete` | `name`, `def?` | Central MCP server defs; materialized into each backend's native config at spawn |

### Team (desktop-only — method-not-found on servers without the bus)
All chat operations are scoped to a team `room`; `team_status` / `team_teams`
are team-agnostic. The Team tab hides itself when these return -32601.

| Method | Params | Response |
|--------|--------|----------|
| `team_status` | — | `{available, teams, system_prompt, …}` |
| `team_teams` | — | Team list (room, workspace, agents) |
| `team_start_team` | `workspace`, `template?` | Starts a team for the workspace; returns its descriptor |
| `team_close_team` | `room` | OK |
| `team_history` | `room`, `limit?` | Message history |
| `team_roster` | `room` | Live roster with agent states |
| `team_employees` | `room` | Desired-roster employee list |
| `team_post` | `room`, `body`, `requires_reply?` | Posts a chat message as the human |
| `team_templates` | — | Named roster templates |
| `team_template_save` | `name`, `def` | OK |
| `team_template_delete` | `name` | OK |
| `team_system_prompt_save` | `text` | OK |

## Server Push Messages
| Method | Params | Description |
|--------|--------|-------------|
| `pane_output` | `target`, `content?`, `cursor`, `current_command?` | Pushed on content/cursor change; `current_command` appears on the first push and when the command changes |
| `pane_closed` | `target` | Pushed when pane becomes unreachable (after repeated capture failures) |
| `team_message` | `room`, `message` | New group-chat message in a team room |

Cursor object: `{x, y, w, h, t}` (x/y position, width, height, trailing trimmed lines).
Content is omitted when only cursor position changed.

## Error Format
```json
{"error": {"code": -1, "message": "description"}}
```
