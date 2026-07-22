# Backend module split: team.rs + server.rs

Date: 2026-07-22
Status: in progress

## Problem

`team.rs` (2991 lines) and `server.rs` (2334 lines) are single files holding
several distinct subsystems each. Finding anything means scrolling; every
change churns a giant file; test placement is a single 1100-line mod at the
bottom. Goal: split into subsystem modules, purely mechanical (no logic
changes), tests distributed to the module they test.

## Constraints

- **Mechanical moves only.** No logic edits, no renames, no "improvements"
  in the same commit. Visibility widening (`pub(super)`/`pub(crate)`) where
  a move crosses a module boundary is the only permitted source change.
- **External API frozen.** Callers use 16 symbols (`team::team_slug`,
  `server::TeamBridge`, …). `mod.rs` re-exports keep every existing path
  working; zero edits outside the two subsystems.
- **One extraction per commit**, `cargo test -- --test-threads=1` green
  before each commit (baseline: 115 tests).
- History preserved: first commit per subsystem is `git mv x.rs x/mod.rs`.

## Target layout

```
team/
  mod.rs        TeamConfig, start(), seed_template, merge_list/merge_env,
                reconcile constants, re-exports
  templates.rs  builtin template consts + template file CRUD + seeding
  workspace.rs  slugs (workspace/team/runtime), Paths, prepare_home,
                prepare_kiro_home, system prompt file
  skills.rs     resolve_skills, read_skill_meta, fetch_git_skill,
                parse_github, skills_index_text, skills_cache_dir
  backends.rs   McpDef + per-backend config (kiro/claude/codex mcp values,
                codex overrides + system-file inheritance, hooks values,
                shell_quote, prepare_kiro/prepare_claude/prepare_codex)
  launch.rs     launch_agent, build_agent_prompt, StartupConfirmation +
                prompt markers + confirm_startup_prompt, hb_env
  reconcile.rs  reconcile_loop, SleepState/SleepAction, liveness helpers,
                recovery predicates, nudge_adopted_agents, nudge_pane

server/
  mod.rs        TeamBridge trait, shared type aliases + consts,
                start/start_with_socket, re-exports
  wire.rs       wire tags, encode/decode_wire_payload, hex helpers,
                derive_key, HalfCipher, provided_token_matches
  rpc.rs        Request/Response/ErrorInfo, ERR_* codes, require_str,
                valid_process_arg, handle_request (the dispatch),
                handle_subscribe/unsubscribe
  team_rpc.rs   handle_team_request, handle_notification_request,
                notification_push_loop, team_push_loop
  download.rs   sign/verify_download, range parsing, dl detection,
                handle_http_download
  connection.rs handle_connection, handle_connection_ws, subscription_loop,
                enable_tcp_keepalive, ws_config
```

Tests move with their subject (test names map cleanly: `sleep_state_*` →
reconcile, `wire_*`/cipher → wire, `codex_*`/`mcp_value_*` → backends, …).
Shared test scaffolding (MockBus, `req()`) lives in a `#[cfg(test)]
pub(super) mod test_util` under each subsystem's mod.rs where needed.

## Order (least → most coupled)

1. `git mv team.rs team/mod.rs` (compile-identical)
2. templates → workspace → skills → backends → launch → reconcile
3. `git mv server.rs server/mod.rs`
4. wire → download → team_rpc → rpc → connection
5. clippy full pass; findings fixed separately or logged in unresolved.md
6. docs sync (CLAUDE.md Testing section names the files; team.md/design docs
   reference `team.rs`/`server.rs` paths)

## Done when

- No src-tauri/src file > ~1000 lines except generated/vendored
- 115 tests green, same test names, no test deleted or weakened
- `cargo clippy` no new warnings vs baseline
- External callers untouched (`git diff` shows no changes outside
  src-tauri/src/{team,server}*, docs)
