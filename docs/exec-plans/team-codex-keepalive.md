# Team Codex Keepalive

## Problem

A Team Codex agent can follow its prompt, reply with `post`, call `wait` once,
then end the turn after that 50-second wait returns no messages. Its generated
`Stop` hook currently sends only a completion notification, so nothing returns
the agent to the Team wait loop. Kiro and Claude already run `keepalive.sh` from
their stop lifecycle.

The current Codex [Hooks documentation](https://learn.chatgpt.com/docs/hooks)
defines `Stop` as a turn-scoped event and runs all matching command hooks. That
makes Stop the reliable enforcement point; prompt wording alone is advisory.

## Candidate Approaches

1. Strengthen the role prompt. Rejected because the existing explicit
   "always end your turn with wait" instruction already failed in a live run.
2. Have the supervisor poll and nudge every idle pane. Rejected because bus idle
   and a stopped TUI are deliberately distinct, and polling would interrupt
   healthy long waits.
3. Add the existing keepalive command beside the Codex Stop notification hook.
   Chosen because it matches Kiro/Claude behavior and acts only when Codex ends
   a turn.

## Acceptance Criteria

- Codex Stop runs both Team keepalive and completion notification commands.
- Keepalive submits its prompt after the Codex turn-complete redraw instead of
  leaving text unsubmitted in the composer.
- Kiro and Claude hook behavior is unchanged.
- A live Codex agent that ends after an empty wait is re-prompted and calls
  `wait` again.
- A pending `@builder` obligation is received and answered after the wake-up.

## Files

- `src-tauri/src/team.rs`
- `team/hooks/keepalive.sh`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/team-codex-keepalive.md`

## Proof

```bash
cd src-tauri && cargo test team::tests::codex_stop_hooks -- --test-threads=1
cd src-tauri && cargo test -- --test-threads=1
node --test src/lib/*.test.js
npm run build
git diff --check
```
