# Team Agent Permission Startup

## Problem

Team launches Kiro, Claude Code, and Codex non-interactively inside named tmux
windows. The three CLIs have different trust boundaries:

- Kiro tool confirmation is disabled by `--trust-all-tools` plus its private
  `chat.disableTrustAllConfirmation` setting.
- Claude's `--dangerously-skip-permissions` bypasses tool approvals but does not
  bypass the separate first-use workspace trust dialog.
- Codex's dangerous bypass flags cover command approvals, sandboxing, and hook
  trust; current Codex does not show a separate workspace dialog.

The launcher currently sends fixed keystrokes every four seconds. On a slower
Claude startup, Enter arrives before the trust dialog, the inline prompt arrives
while the dialog is active, and a later reconnect nudge cancels Claude back to
the shell. Codex receives an unnecessary blind Enter.

## Candidate Approaches

1. Pre-edit Claude's global `~/.claude.json` project trust map. Rejected because
   that is private global state, requires concurrent JSON mutation, and couples
   Team to an undocumented schema.
2. Continue using longer fixed sleeps. Rejected because startup time varies with
   auth, hooks, MCP startup, and machine load.
3. Pass Claude's initial prompt as a CLI argument and asynchronously confirm only
   when the pane contains the complete workspace trust prompt. Chosen because it
   uses the public CLI contract, avoids blind input, and does not serialize other
   agent launches behind Claude startup.

## Acceptance Criteria

- Kiro launch retains both tool-trust bypass mechanisms.
- Claude launch bypasses tool permissions and its dangerous-mode warning.
- Claude receives its first prompt as a launch argument.
- The launcher presses Enter only after detecting Claude's workspace trust
  dialog; already-trusted workspaces receive no synthetic input.
- Codex retains approval/sandbox and hook-trust bypass flags and receives no
  blind startup Enter.
- Startup confirmation polling is bounded and does not block roster launch.
- Team lifecycle hooks consume one JSON line and never wait for the Agent CLI to
  close stdin.
- Generated hook commands use an explicit system shell so macOS provenance
  metadata cannot kill direct script execution.

## Files

- `src-tauri/src/team.rs`
- `src-tauri/src/agent_notifications.rs`
- `team/hooks/heartbeat.sh`
- `team/hooks/keepalive.sh`
- `docs/design-docs/features/team.md`
- `docs/design-docs/features/agent-notifications.md`
- `docs/exec-plans/team-agent-permissions.md`

## Proof

```bash
cd src-tauri && cargo test team::tests::backend_launch_permissions_and_startup_confirmations -- --test-threads=1
cd src-tauri && cargo test -- --test-threads=1
git diff --check
```
