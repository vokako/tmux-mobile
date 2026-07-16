# Team @all Routing

## Problem

The bus expands `@all` when `requires_reply=true`, but MCP callers can omit
that flag. The shared contract also says only "addressed to you", which leaves
room to mistake `@all` for an informational broadcast. Conversely, unmentioned
natural-language requests such as "everyone introduce yourself" deliberately
create no obligation, so a manager-led team needs its lead to route that intent.

## Candidate Approaches

1. Infer group intent from words such as "everyone", "大家", or "each".
   Rejected because language-dependent intent classification would create
   accidental assignments and make bus behavior non-deterministic.
2. Clarify prompts only. Rejected because an MCP caller can still forget
   `requires_reply=true`, leaving no enforceable obligation.
3. Make `@all` an always-reply bus primitive and document lead routing.
   Chosen because addressing stays explicit while enforcement no longer
   depends on each caller setting a second flag correctly.

## Acceptance Criteria

- `@all` creates a reply obligation for every registered agent except sender,
  even when `requires_reply=false`.
- A plain broadcast creates no obligations.
- Every agent prompt states the addressing rule in two concise bullets.
- In manager-led teams, only the lead handles unaddressed human requests; the
  lead converts explicit group intent into an `@all` instruction.

## Files

- `src-tauri/crates/agora/src/bus.rs`
- `src-tauri/crates/agora/src/mcp.rs`
- `src-tauri/src/team.rs`
- `team/templates/mixed-engineering/team.yaml`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/team-all-routing.md`

## Proof

```bash
cd src-tauri/crates/agora && cargo test all_mention -- --test-threads=1
cd src-tauri/crates/agora && cargo test plain_broadcast -- --test-threads=1
cd src-tauri && cargo test -- --test-threads=1
node --test src/lib/*.test.js
npm run build
git diff --check
```
