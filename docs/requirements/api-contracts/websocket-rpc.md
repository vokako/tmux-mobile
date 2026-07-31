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

### Agent Notifications (agent lifecycle hooks)
| Method | Params | Response |
|--------|--------|----------|
| `agent_notifications_list` | — | Unread-notification snapshot `{unread: [{session, window, …}]}` |
| `agent_notifications_mark_read` | `session`, `window` | Updated snapshot |
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
| `project_autostart` | `id`, `autostart?` | Flag only; no boot integration yet |
| `project_snapshots` | `id` | `[{id, at, windows}]`, newest first (last 20 kept) |
| `project_restore` | `id`, `snapshot_id` | Replaces the declaration with that topology (call `project_up` after) |

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
| `agent_notification` | notification snapshot | Unread agent-lifecycle notifications changed |

Cursor object: `{x, y, w, h, t}` (x/y position, width, height, trailing trimmed lines).
Content is omitted when only cursor position changed.

## Error Format
```json
{"error": {"code": -1, "message": "description"}}
```
