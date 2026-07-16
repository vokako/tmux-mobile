# Team Agent Authentication Inheritance

## Problem

Team agents use private backend homes so their MCP servers, hooks, and state do
not modify the user's global CLI configuration. Codex authentication may depend
on file login, OS keyring settings, or a custom provider plus environment file.
Replacing `CODEX_HOME` prevents a Team Codex agent from seeing all three.

Kiro authentication remains available with an isolated `KIRO_HOME`, and Claude
Code keeps its normal home while loading Team settings through `--settings`.
Only Codex needs an explicit bridge to the system authentication state.

## Candidate Approaches

1. Copy auth/provider files into every agent home. Rejected because copied
   credentials become stale and create additional secret-bearing files.
2. Stop isolating `CODEX_HOME`. Rejected because Team hooks, MCP configuration,
   and runtime state would then modify or collide with the user's global setup.
3. Link the private home's `config.toml`, `.env`, and `auth.json` to the system
   Codex home, then inject Team MCP settings with CLI `-c` overrides. Chosen
   because Codex keeps private runtime state while reading current global
   provider/login state without copying or modifying credential contents.

Environment-based and system-keychain authentication need no bridge: the child
process already inherits its environment and OS credential access.

## Acceptance Criteria

- A Team Codex home links existing `config.toml`, `.env`, and `auth.json` from
  `$CODEX_HOME`, or `~/.codex` when the variable is unset.
- Team MCP settings are launch-time overrides and do not modify global config.
- No credential file is read, copied, logged, or overwritten by tmux-mobile.
- A missing global auth file remains a no-op so environment/keychain auth works.
- An existing unrelated file in the private home is not overwritten.
- Repeated preparation is idempotent.

## Files

- `src-tauri/src/team.rs`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/team-auth-inheritance.md`

## Proof

```bash
cd src-tauri && cargo test team::tests::codex_auth -- --test-threads=1
cd src-tauri && cargo test -- --test-threads=1
npm run build
```
