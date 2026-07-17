# Team prompt architecture cleanup

## Problem

Team behavior currently comes from overlapping layers: `config.toml`
`team_rules`, a hidden default `team_kick`, template prompt/role text, MCP server
instructions and tool descriptions, plus keepalive/recovery prompts. The overlap
creates concrete routing gaps and contradictory guidance:

- an unaddressed human message wakes every Agent but no rule assigns exactly one
  responder;
- Triad's planner review is not reliably addressed to the planner;
- repository-only artifact rules conflict with Triad's general Q&A purpose;
- verification ownership discourages an independent reviewer from rerunning
  risk-relevant evidence;
- Claude and Codex receive Team policy as an initial user message rather than
  through their native high-priority instruction surfaces.

## Approaches considered

1. Add exceptions to every repeated prompt. Rejected: each fix would increase
   drift and context cost.
2. Put all policy in each template. Rejected: acknowledgement, addressing,
   honesty, and handoff behavior should remain consistent across teams.
3. Give every layer one responsibility. Chosen:
   - `system_prompt.md`: optional operator-wide additions;
   - `team_rules`: the only cross-template collaboration policy;
   - template prompt: routing and workflow unique to that roster;
   - role/goal: one Agent's ownership and boundaries;
   - MCP descriptions: API semantics only;
   - kick/keepalive/recovery: short lifecycle commands only.

Kiro keeps its native Agent `prompt`. Claude receives the assembled Team prompt
through `--append-system-prompt`. Codex receives it through the documented
`developer_instructions` configuration key, merged after any existing user
developer instructions. The initial user message contains only the startup
kick.

## Done when

- Global rules define exactly when an unaddressed human message is actionable
  without assuming every template has the same manager name.
- Triad assigns unaddressed human messages to lead and has an explicit,
  addressed builder → planner review handoff.
- General conversational answers may stay in chat; durable task artifacts and
  decisions go to workspace files when appropriate.
- Independent reviewers may rerun focused risk-relevant evidence without
  mechanically repeating an entire suite.
- `team_kick` is seeded visibly into `config.toml`; runtime/MCP/keepalive text
  no longer repeats policy.
- Kiro, Claude, and Codex generated launch configurations carry Team policy in
  their native high-priority instruction surface.
- Focused prompt/config/backend tests and the complete Rust suite pass without
  restarting the running server.

## Files

- `src-tauri/src/config.rs`
- `src-tauri/src/team.rs`
- `src-tauri/crates/agora/src/mcp.rs`
- `team/hooks/keepalive.sh`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`
- local `~/.config/tmux-mobile/config.toml`
- local `~/.config/tmux-mobile/teams/triad/team.yaml`

## Implementation status

- Global collaboration rules and visible lifecycle kick: implemented.
- Runtime/MCP/keepalive policy deduplication: implemented.
- Triad routing, review handoff, and verification ownership: implemented
  without changing any `manage` value.
- Native Kiro/Claude/Codex instruction delivery: implemented.
- Documentation and focused regression coverage: implemented.
- Verified with the Agora test suite, complete Rust test suite, frontend unit
  tests, production frontend build, TOML/YAML parsing, and `git diff --check`.
