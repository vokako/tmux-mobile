# WebSocket JSON-RPC API Contract

## Transport
JSON-RPC over WebSocket (`ws://` or `wss://`).

## Authentication

### Encrypted Auth (default, when Web Crypto available)
```
← {"server_nonce": "<hex>"}                          // server sends nonce
→ {"method": "auth", "params": {"client_nonce": "<hex>", "proof": "<hex>"}}
← <base64-encrypted> {"result": {"authenticated": true, "machine_id": "uuid", "hostname": "my-mac"}}
```
- Key derivation: HKDF-SHA256(token, server_nonce + client_nonce)
- Proof: HMAC-SHA256(key, server_nonce)
- All subsequent messages encrypted with AES-256-GCM using derived key

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
| `list_sessions` | — | Array of `{name, windows, attached, created, last_opened?}` objects. `last_opened` is unix seconds of the last time this session was opened via tmux-mobile (`subscribe` RPC); absent if never opened. |
| `list_panes` | `session` | Array of pane objects: `{session, window, pane, width, height, current_command, window_name, pane_title, current_path}`. `current_path` is tmux `#{pane_current_path}` (the pane process's cwd). |
| `new_session` | `name?`, `path?`, `command?` | OK |
| `kill_session` | `name` | OK |
| `new_window` | `session` | OK |
| `kill_window` | `target` | OK |

### Pane Operations
| Method | Params | Response |
|--------|--------|----------|
| `capture_pane` | `target`, `lines?` | `{output}` with ANSI colors |
| `send_keys` | `target`, `keys`, `literal` | OK |
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
| `fs_download` | `path` | `{name, data}` base64 (≤50MB) |
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

## Server Push Messages
| Method | Params | Description |
|--------|--------|-------------|
| `pane_output` | `target`, `content?`, `cursor` | Pushed on content/cursor change during subscription |
| `pane_closed` | `target` | Pushed when pane becomes unreachable (after repeated capture failures) |

Cursor object: `{x, y, w, h, t}` (x/y position, width, height, trailing trimmed lines).
Content is omitted when only cursor position changed.

## Error Format
```json
{"error": {"code": -1, "message": "description"}}
```
