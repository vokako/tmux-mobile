# Configuration reference

Runtime configuration of the WebSocket server and the Team feature. Build and dev-loop settings are in `../conventions/development.md`.

File: `$XDG_CONFIG_HOME/tmux-mobile/config.toml` (fallback `~/.config/tmux-mobile/`) — token, host, port, tmux_socket, tls_cert, tls_key, scrollback, disconnect_grace_secs, team_bind, team_db, team_room, team_model, team_codex_profile.
Env vars `TOKEN`, `HOST`, `PORT`, `TMUX_SOCKET`, `TLS_CERT`, `TLS_KEY`, `SCROLLBACK`, `DISCONNECT_GRACE_SECS`, `TEAM_BIND`, `TEAM_DB`, `TEAM_ROOM`, `TEAM_MODEL`, `TEAM_CODEX_PROFILE` override config (legacy `CREW_*`/`AGORA_*` env vars + `crew_*`/`agora_*` config keys still accepted as aliases).
Default scrollback: 500 lines.
`<config>/AGENTS.md` — optional app-wide agent instructions, prepended to every managed agent's system prompt at spawn (`tmm prompt`, Settings → Agents). Absent = none. See `docs/design-docs/features/tmm-cli.md` § The app-wide instructions.
Default disconnect_grace_secs: 600 (10 min). Delay before a disconnected client's tmux window is auto-resized back; set to 0 for legacy immediate restore. See `docs/design-docs/features/disconnect-grace.md`.
Default team_bind: `127.0.0.1:8787` (in-process MCP daemon + dashboard for the Team feature, desktop only). team_db default: `<config>/tmux-mobile/team.db`; team_room default: `main`; team_model default: `claude-sonnet-4.6` (kiro-backed agents). Team launching is in-process (`src-tauri/src/team/`), per-workspace, triggered by the Team tab's Start button. room→workspace persisted to `teams.json` for restart recovery. See `docs/design-docs/features/team.md`.
