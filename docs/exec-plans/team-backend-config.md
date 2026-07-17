# Team Backend Configuration Audit

## Problem

The Team template editor exposes one backend-neutral agent schema, but not every
field reaches every CLI:

- Kiro and Claude receive a non-empty `model`; Codex silently drops it.
- Kiro and Claude model values are interpolated into a shell command without
  quoting.
- The global system prompt can be edited and persisted, but launched agents
  never receive it.
- Remote MCP headers advertise `$VAR` credentials, but Kiro/Claude require
  `${VAR}` interpolation and Codex requires its native environment-header keys.
- Claude and Codex tool-permission bypass flags do not skip the first-use
  folder-trust dialog for a new workspace.

The remaining supported fields (`role`, `goal`, `manage`, `env`, `mcp`,
`skills`, and the team-wide prompt) already flow through the seed and backend
adapter paths.

## Candidate Approaches

1. Keep separate Kiro, Claude, and Codex schemas. Rejected because templates
   would stop being portable and the editor would need backend-specific forms.
2. Fix only Codex model forwarding. Rejected because the same audit found two
   other exposed settings whose behavior does not match the documented contract.
3. Keep the unified schema and repair its backend adapters. Chosen because each
   CLI already has a dedicated `prepare_*` function and the change remains
   local to the launch boundary.

## Acceptance Criteria

- A non-empty per-agent model is passed through the native model flag for Kiro,
  Claude, and Codex.
- Model arguments are shell-quoted; whitespace-only model values behave as
  empty.
- Empty models retain the documented backend defaults: configured Team model
  for Kiro, `sonnet` for Claude, and the user's Codex configuration.
- The persisted global system prompt is prepended to every backend's launch
  prompt and is refreshed whenever a team starts.
- Remote MCP header environment references are translated without reading or
  persisting the credential value.
- New and recovered Claude/Codex panes automatically confirm only their complete
  backend-specific folder-trust dialogs; unrelated prompts receive no key.
- Tests cover all three model adapters, empty/default behavior, quoting, and
  global/team/role prompt ordering.
- Existing environment, MCP, skill, hook, and authentication behavior is
  unchanged.

## Files

- `src-tauri/src/team.rs`
- `src-tauri/src/tmux.rs`
- `src-tauri/src/team_bridge.rs`
- `team/templates/default/team.yaml`
- `docs/design-docs/features/team.md`
- `docs/unresolved.md`
- `docs/exec-plans/team-backend-config.md`

## Proof

```bash
cd src-tauri
cargo test team::tests::backend_model_selection_is_forwarded -- --test-threads=1
cargo test team::tests::build_agent_prompt_structure -- --test-threads=1
cargo test team::tests::mcp_value_remote_and_local_per_backend -- --test-threads=1
cargo test team::tests::backend_launch_permissions_and_startup_confirmations -- --test-threads=1
cargo test -- --test-threads=1
git diff --check
```
