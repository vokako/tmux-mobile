# tmux Wrapper Service

## Purpose
Rust wrapper around tmux CLI commands. File: `src-tauri/src/tmux.rs`

## Operations
- `list_sessions` — list all sessions with `name`, `windows`, `attached`, `created` (tmux's `session_activity`), and `last_opened` (per-session timestamp of the last `subscribe` from tmux-mobile, sourced from `config::get_session_usage()`). Baseline sort is `session_activity` descending; clients apply MRU on top.
- `list_panes(session)` — list all panes across all windows (id, session, window, index, active, command, title, cwd, width, height)
- `capture_pane(target, lines?)` — capture content with ANSI escapes, joined soft-wrapped lines
- `capture_pane_with_width(target, lines?, width)` — capture with CJK double-width character fix, returns (content, trailing_trimmed)
- `cursor_info(target)` — get cursor position (x, y), pane height and width
- `send_keys(target, keys, literal)` — send keystrokes. Literal mode splits
  C0 control bytes out as tmux *named* keys (`\x03` → `C-c`; ESC+C0 → `M-C-x`):
  with `extended-keys on`, tmux silently drops raw C0 bytes sent via
  `send-keys -l` to panes whose app enabled an extended keyboard protocol
  (`#{pane_key_mode}` = `Ext …` — kiro/claude/codex all do). Named keys are
  re-encoded by tmux to match the pane's key mode, so they reach both legacy
  and extended panes. `\t` `\n` `\r` and standalone ESC stay literal.
- `send_command(target, command)` — send text + Enter
- `paste_text(target, text)` — paste via tmux `load-buffer -b tmm-paste -` +
  `paste-buffer -p -d`: tmux wraps the block in bracketed-paste markers
  exactly when the pane app enabled mode `?2004`, so multi-line pastes are
  one paste, not one command per line; legacy apps get the raw text.
- `pane_command(target)` — get current running command
- `pane_cwd(target)` — get pane working directory
- `new_session(name, path?, command?)` — create session (checks for name conflicts)
- `kill_session(name)` — kill session
- `new_window(session)` — create new window in session
- `kill_window(target)` — kill window
- `resize_pane(target, cols, rows)` — resize pane dimensions
- `run_resize_window_auto(target)` — restore window to auto-size
- `set_resize_hook(session)` — set tmux hook for auto-restore on client attach
- `visible_width(s)` — calculate visible width accounting for CJK double-width chars
- `is_server_running()` — check if tmux server is running

## Key Details
- **A session name is an exact target.** tmux resolves a bare `-t name` as
  exact name → prefix of a name → fnmatch glob, so with only `dev-2` alive
  `has-session -t dev` succeeds and `kill-session -t dev` kills `dev-2`. Every
  wrapper that takes a session NAME (`list_panes`, `session_exists`,
  `new_session`'s conflict check, `kill_session`, `new_window`,
  `rename_session`, `ensure_session`, `find_window_by_name`,
  `list_named_windows`, `new_named_window`, `window_activity_times`) passes
  `=name:` (`exact_session`): `=` for exact, the trailing `:` because
  target-window/pane commands (`list-panes -s`, `new-window`,
  `display-message`) otherwise read `=name` as a window name and fall back to
  the prefix search. Exceptions, verified on tmux 3.6a: `set-option` and
  `set-hook` reject an `=` target and get the bare name — safe there because the
  exact session was just created and tmux tries the exact name first.
- **A hook body is re-parsed by tmux.** `set_resize_hook` embeds the session
  name in a command string; it is quoted with `tmux_quote` (double quotes, `\`
  `"` `$` escaped), or a name with a space or `;` becomes extra commands.
- **Named-key `send-keys` puts `--` before the key** so a key string that starts
  with `-` can never be read as a send-keys flag.
- capture_pane flags: `-p` (stdout), `-e` (ANSI escapes), `-J` (join soft-wrapped lines)
- Custom socket path support via `-S` flag (global `RwLock`, set/get via `set_socket`/`get_socket`)
- Scrollback lines configurable via `set_scrollback`/`get_scrollback` (default 500, atomic)
- tmux binary auto-detected: checks `/opt/homebrew/bin/tmux`, `/usr/local/bin/tmux`, then PATH
- CJK double-width wrapping fix (`join_unflagged_wraps`): tmux `-J` misses a
  wrap only when a 2-cell char didn't fit in the last column (column left
  empty, WRAPPED flag not set). Join strictly on that signature: visible
  width == pane_width − 1 AND the next line starts with a wide char. Never
  join exactly-full lines — full-width TUI box borders (kimi) are complete
  rows, and joining them shears every following row.
