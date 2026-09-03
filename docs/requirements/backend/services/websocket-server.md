# WebSocket Server Service

## Purpose
Rust-based WebSocket server providing JSON-RPC interface to tmux and filesystem operations.

## Implementation
- Module: `src-tauri/src/server/` (`mod.rs` listener/bridge trait, `wire.rs` framing+crypto, `rpc.rs` dispatch, `team_rpc.rs` team/agent-hooks RPCs, `download.rs` HTTP side-channel, `connection.rs` connection pump)
- Dependencies: tokio, tokio-tungstenite, hmac, sha2, aes-gcm, base64
- Starts on desktop only (`#[cfg(desktop)]`)
- Standalone binary: `src-tauri/src/bin/server.rs`

## Authentication
- Two modes: encrypted (AES-256-GCM with HKDF key derivation) and legacy plain token
- Server sends `server_nonce` (+ `e2e: 2`, the newest handshake it speaks) on connect; client responds with `client_nonce` + HMAC proof (+ the `e2e` version it wants), or plain token
- Before auth a connection gets 8 KB per text frame and 10 s to finish the handshake; over either, the server closes it. The 80 MB message allowance exists for authenticated `fs_upload`, not for a stranger on a 0.0.0.0 port.
- E2E v2 derives three keys per session (proof, client→server, server→client) so the two directions never share a (key, nonce); a client that does not ask for v2 gets the single-key v1 handshake. Contract: `docs/requirements/api-contracts/websocket-rpc.md`
- Encrypted mode: all subsequent messages are binary AES-256-GCM frames; decrypted payloads use a one-byte raw-JSON/raw-deflate framing tag
- Token auto-generated on first run, persisted in `~/.config/tmux-mobile/config.toml` (mode 0600). Printed at startup only when stdout is a terminal; under a supervisor or redirect the line names the config file instead, so the secret never lands in a log
- Environment variable `TOKEN` overrides config
- Rate limiting: per-IP auth failure tracking with lockout after repeated failures

## Subscription Model
- `subscribe(target)` starts a polling loop (200ms interval)
- Polls `capture_pane_with_width` (ANSI escapes + joined soft-wrapped lines + CJK width fix)
- Also polls `cursor_info` for cursor position
- Pushes `pane_output` with content (only when changed), cursor position, and `current_command` on the first push or when it changes
- Pushes `pane_closed` after repeated capture failures (pane gone)
- One subscription map per connection (multiple targets supported)
- `unsubscribe` or disconnect stops the loop

## Resize Tracking
- `resize_pane` resizes tmux pane to match client viewport
- Two-level tracking:
  - Per-connection: which windows each connection has resized
  - Per-window: how many still-connected clients are "holding" the window at its current size, plus an optional pending-restore task handle
- On disconnect: decrement the holder count for each window; if the count reaches 0, schedule (not perform) a restore task that sleeps `disconnect_grace_secs` and then calls `resize-window -A`
- Any subsequent `resize_pane` on that window cancels the pending restore — so short reconnects (app backgrounded, network blip) avoid the disconnect → reflow → reconnect → reflow double-cycle
- `disconnect_grace_secs = 0` preserves the legacy immediate-restore behavior (no timer)
- Sets tmux hook (`client-session-changed`) so real terminal clients restore size when they next attach

## Configuration
- File: `~/.config/tmux-mobile/config.toml`
- Fields: token, host (default 0.0.0.0), port (default 9899), tmux_socket, tls_cert, tls_key, scrollback (default 500), disconnect_grace_secs (default 600)
- Env vars: TOKEN, HOST, PORT, TMUX_SOCKET, TLS_CERT, TLS_KEY, SCROLLBACK, DISCONNECT_GRACE_SECS
- TLS support via tls_cert + tls_key paths
- Preferences: separate `prefs.json` file for client-side settings (get_prefs/set_pref)
- Bookmarks: separate `bookmarks.json` file

## Security
- Full filesystem access by design (remote management tool)
- Path traversal protection on download filenames (`sanitize_filename()`)
- Git operations whitelisted to safe subset, shell metacharacters rejected in args
- iframe sandbox: `allow-same-origin` only (no scripts)

## Agent hooks RPC

- `agent_hooks_status`, `agent_hooks_install`, and `agent_hooks_remove` manage only tmux-mobile-owned Claude Code, Codex, and Kiro hooks.
- The hooks feed telemetry (status derivation, tool/prompt rows), the managed
  stop-hook auto-post into the project room, and the per-window conversation-id
  memory that project restore resumes with.
- The unread-inbox surface retired 2026-09-01: `agent_notifications_list`,
  `agent_notifications_mark_read` and the `agent_notification` push are no
  longer served — an old client's call gets METHOD_NOT_FOUND (soft error, same
  dispatcher still answers `agent_hooks_*`; a boundary test pins both). The
  room's auto-post + read cursor and the derived status dots carry the signal.
