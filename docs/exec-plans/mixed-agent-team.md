# Mixed Agent Engineering Team

## Problem

Team supports Kiro, Claude Code, and Codex agents, but every current built-in
template uses only Kiro. Users need a built-in roster that combines all three
backends and assigns work according to their documented strengths instead of
treating them as interchangeable shells.

Official guidance reviewed on 2026-07-16:

- Kiro [Specs](https://kiro.dev/docs/specs/), [Steering](https://kiro.dev/docs/steering/),
  and [Custom agents](https://kiro.dev/docs/cli/custom-agents/) emphasize
  requirements, design, tracked tasks, persistent project context, and
  specialized agent configuration.
- Claude Code [Best practices](https://code.claude.com/docs/en/best-practices)
  and [Subagents](https://code.claude.com/docs/en/sub-agents) emphasize
  exploration before implementation, explicit planning, context isolation,
  verification, and independent review.
- The current [Codex manual](https://developers.openai.com/codex/codex-manual.md)
  recommends bounded tasks with explicit goals, constraints, done criteria,
  tests, and review; parallel work is best kept independent.

## Candidate Approaches

1. Keep one generic role per backend. Rejected because it does not turn each
   tool's strengths into a coherent workflow or define safe handoffs.
2. Build a large roster with multiple agents per backend. Rejected because it
   adds coordination and token cost before the work can be split safely.
3. Use a fixed three-agent roster: Kiro lead, Claude architect/reviewer, and
   Codex builder/verifier. Chosen because it covers specification, design,
   implementation, and independent review with the smallest useful mixed team.

The roster is fixed rather than using runtime `hire()`, because dynamically
hired agents currently default to Kiro and cannot select Claude or Codex.

## Acceptance Criteria

- A `mixed-engineering` built-in is seeded and selectable like other templates.
- Its fixed roster contains exactly one Kiro, one Claude, and one Codex agent.
- Agent goals define non-overlapping ownership and explicit handoffs.
- The lead does not declare completion until the implementation has test
  evidence and the independent review is approved.
- Blank model fields preserve each backend's configured system default.

## Files

- `team/templates/mixed-engineering/team.yaml`
- `src-tauri/src/team.rs`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/mixed-agent-team.md`

## Proof

```bash
cd src-tauri && cargo test team::tests::mixed_engineering -- --test-threads=1
cd src-tauri && cargo test -- --test-threads=1
node --test src/lib/*.test.js
npm run build
git diff --check
```
