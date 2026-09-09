# Configuration reference

Runtime configuration of the WebSocket server and the Team feature. Build and dev-loop settings are in `../conventions/development.md`.

File: `$XDG_CONFIG_HOME/tmux-mobile/config.toml` (fallback `~/.config/tmux-mobile/`) — token, host, port, tmux_socket, tls_cert, tls_key, scrollback, disconnect_grace_secs.
Env vars `TOKEN`, `HOST`, `PORT`, `TMUX_SOCKET`, `TLS_CERT`, `TLS_KEY`, `SCROLLBACK`, `DISCONNECT_GRACE_SECS` override config. (The retired Team feature's `TEAM_*` keys are gone, board #100; `projects::rooms::import_legacy` still reads `TEAM_DB`/`team_db`/`crew_db`/`agora_db` once to locate a legacy team.db for the one-off chat-history import.)
Default scrollback: 500 lines.
`<config>/AGENTS.md` — optional app-wide agent instructions, prepended to every managed agent's system prompt at spawn (`tmm prompt`, Settings → Agents). Absent = none. See `docs/design-docs/features/tmm-cli.md` § The app-wide instructions.
Default disconnect_grace_secs: 600 (10 min). Delay before a disconnected client's tmux window is auto-resized back; set to 0 for legacy immediate restore. See `docs/design-docs/features/disconnect-grace.md`.
