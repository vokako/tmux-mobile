# TODO: Gaps Against the Tenets

> Draft · 2026-09-09. This document records gaps and remaining work, not
> decisions. Decisions belong in `tenet.md`; rules belong in `guidance/`.
> Detailed historical context remains in `docs/unresolved.md`; this list
> gives short descriptions and priorities. Consolidating the two is a
> remaining task below.
> Priorities: **P0** implementation violates an established tenet;
> **P1** correctness/security; **P2** quality debt; **P3** recorded limitations.

---

## A. Implementing the Tenets (P0)

- [x] **Delete the desktop Team / agora bus completely** (owner, 2026-09-09;
  board #100/#107 completed 2026-09-09). Prerequisite moves went to
  `projects/backends/shared.rs` and `projects/skills.rs`. Hub messages moved
  into state.db `hub_msgs` through `projects/rooms.rs`, including a one-time
  proj:* history import. The agora crate, `src-tauri/src/team/`,
  `team_bridge.rs`, `server/team_rpc.rs`, `src/lib/team/`, `team/` and
  `TEAM_*` configuration were removed. Documents such as `team.md` were
  archived in `exec-plans/`.
- [ ] **Consolidate backend knowledge:** one `Backend` trait/enum, one file
  per backend, containing detection needles, launch command, resume syntax,
  rendering, hook installation/payload parsing, effort values, status-line
  inspection, icons and color names.
  `materialize`/`refresh_hooks`/`normalize`/`resume_command` become trait calls;
  the frontend obtains backend lists and resource names from the server.
  Source-contract tests reject backend literals outside `backends/`, except
  tests and seed data. See the inventory in section D.
- [x] **Shorten `tmm-cli.md`** (owner: "太长了不对"; board #102, completed
  2026-09-09): 2337 to about 340 lines. The command reference fits one screen.
  Redundant narrative was removed; condensed rules already live in their
  design documents. Unique material moved: board #30/#31 rules to `board.md`;
  configuration drift, grok, claude-Bedrock and backend parity to
  `agents-overview.md`; board #9's read contract, drawer switcher and project
  header to `hub-feed.md`; user vocabulary to `design-language.md`.
  Every existing command passed the tenet 6 audit; see `tmm-cli.md`,
  "What this is".
- [ ] **English documentation:** translate the finalized `tenet.md`,
  `guidance/*.md` and this document, as the owner selected English for
  documentation. The #103 language-copy decision is to maintain English
  without new `.zh.md` copies, retaining owner quotes in Chinese.
- [ ] **Entry-point map:** CLAUDE.md's ownership boundaries and documentation
  map point to `tenet.md` and `guidance/`; `<config>/AGENTS.md` references
  the short Zen list, describing process only (tenet 11).
- [ ] **Consolidate `docs/unresolved.md` and this document** in either direction,
  leaving one entry point for remaining work.
- [ ] **Review process:** define how agents review separate disciplines,
  with one reviewer per dimension, the corresponding guidance checklist,
  and conclusions recorded in board notes.

## B. Correctness (P1)

- [ ] Registry definition edits do not reach existing agents. Add
  `slots.agent_def`; `refresh_hooks` should synchronize model/MCP/resources
  without changing the prompt. `refresh_agent` partly addresses this;
  verify what remains.
- [x] `is_managed_in` no longer re-arms merely because kiro recreates the
  `KIRO_HOME` subtree after `agent_remove`. It now requires `launch.json`
  or a pre-recipe `agents/<name>.json` (board #112, 2026-09-09).
- [ ] The roughly 1300-line touch-gesture state machine is embedded in
  `Terminal.svelte`'s effect closure with no tests; `kbLocked` invariants
  rely only on documentation.
- [x] #108: a bounded stateful filter covers fragmented/coalesced `onData`
  replies. The source of the original `?62;22;52c` report still requires
  measurement; see `unresolved.md`.
- [x] #109: full snapshots restore history. The false-tail event, lost news
  flag and repeated redraw caused by synchronous `clear()` are fixed by
  in-frame `CSI 3J`.
- [ ] Telemetry uses window INDEX while identity uses NAME, causing
  `renumber-windows` mismatches. Renaming between hook and consumption loses
  a post; identical bodies confuse receipts; delivery lacks backpressure;
  `SPAWN_CAP` includes windows we do not own.
- [ ] Backend parity: claude's `/` palette is not transcribed;
  claude/codex/grok have no auto-continue; codex has no StopFailure.
- [ ] Smoke-test CSP on a real device with Markdown, PDF, Mermaid and Hub.
  Browser/PWA responses have no CSP header.
- [ ] The Android signing key was in git history before 60992d4 and has not
  been rotated, by owner decision. Keep this recorded.
- [ ] Isolate flaky `adopt_then_down_then_up_restores_the_workspace` tests
  on a separate tmux socket.
- [ ] Consolidate three shell quoters in `agent_notifications.rs`, `tasks.rs`
  and `team/backends.rs`.
- [ ] `auto_adopt_with` invokes tmux while holding the store lock.
- [ ] The `@all` recipient is stored as `'all'` but not restored by `pickLead`;
  `hubLog` drops `since_ts` when `before_seq` is present.
- [x] Vitals and pane inspection policy is **decided** (owner, 2026-09-09):
  observing screens that people can also read is permitted. `statusLine`
  changes display, not agent behavior, and is allowed. Remaining work is
  moving backend-specific inspection into backend files (section A).

## C. Quality Debt (P2)

- [ ] `Hub.svelte` has 4094 lines, with contiguous Feed / Composer / Roster /
  Sidebar / Drawer / Dialogs blocks. Coupled state includes
  `selected`/`agents`/`feed`/`following`/`recipient`/`roomCache`;
  the `onGoBack` chain needs a layer stack.
- [ ] `store.rs` 2989 / `projects/mod.rs` 2223 / `spawn.rs` 2364 /
  `bin/tmm.rs` 1312 / `vitals.rs` 1308:
  split store by projects/registry/board/activity, move mod.rs skills into
  `skills.rs`, and move spawn `render_*` functions into backend files.
- [ ] The 750-line `hub_rpc.rs` match mixes dispatch, delivery and board
  notification policy. Replace about 40 `require_str→match→err` branches
  with a `?`-returning inner function.
- [ ] `Store::hub_search` (board #107) scans all rows and filters in Rust.
  Current scale is acceptable, but move matching into SQL (`lower(body) LIKE`)
  or FTS before rooms grow (reviewer, 2026-09-09).
- [x] #110: Files navigation history/Back decisions moved into `file-nav.ts`;
  preview body/CSS and renderers moved into `FilePreview.svelte` /
  `file-preview.ts`, preserving behavior.
- [ ] Follow up #110: move renderer state down into `FilePreview` so the host
  passes only the file and callbacks, instead of binding `showAllLines` and four DOM references.
- [ ] Consolidate duplicate Markdown CSS into `ui/MarkdownBody`.
  #110 only moved existing styles, without cross-page unification.
- [ ] `ws.ts` is a module-level singleton with ten top-level `let` variables,
  blocking two connections in split-screen mode.
- [ ] Test gaps: all three `bin/tmm.rs` tests parse flags; `connection.rs`,
  `fs.rs` and `server/mod.rs` lack tests, as do `AgentsPage`, `Projects`,
  `Settings`, `GitPanel` and `ui/Select`. Some source tests pin implementation text.
- [ ] `list_panes` runs a full `ps -axo` every time; a `hub_post` triggers
  at least two calls. Treat `child_cmd` as a detection clue and measure first.
- [ ] Structural clippy findings: too many parameters in `handle_connection`(9),
  `handle_connection_ws`(11) and `prepare_codex`(9); introduce context structures.
  `Outbound::InitCipher` has a large_enum_variant finding.
- [ ] Frontend backend lists: `AgentsPage.svelte` and `TeamTemplates.svelte`
  each define `BACKENDS`, with five implicit `?? 'kiro'` defaults.
  Source these from the server's `SPAWNABLE_BACKENDS` as part of section A.
- [ ] Arbitrary absolute paths in `fs_*`/`/dl` and git push/commit in the
  allowlist are deliberate (`token = shell access`). Clarify the documentation
  or naming so the allowlist does not imply a stronger restriction.
- [ ] npm is aliased to pnpm and `package-lock.json` is stale;
  inspect `npm_config_user_agent` during preflight.
- [ ] Files Markdown escapes inline HTML, turning README badges into text.
  Accept this cost of one safe renderer or design an allowlist.

## D. Five-Backend Inventory

For section A's second item; recorded from the code on 2026-09-09.
In this inventory, `backend` was `&str` and the five literals were scattered:

| File | Lines Mentioning Backend Names | Responsibility |
|---|---|---|
| `projects/spawn.rs` | 218 | Five `render_*`, five `*_hooks`, resume syntax, refresh detection |
| `agent_notifications.rs` | 92 | Hook payload normalization (`normalize` + `is_user_prompt_submit`) |
| `projects/agents.rs` | 89 | `KNOWN` detection table including resume strings, `SPAWNABLE_BACKENDS` |
| `projects/vitals.rs` | 83 | Four `sniff_*` functions |
| `projects/store.rs` | 80 | Seed definitions, ordering `CASE WHEN`, default models |
| `projects/models.rs` | 36 | Effort values, model lists |
| `team/backends.rs` | 37 | MCP, launch-script and trust-marker helpers borrowed by `projects/` |
| Frontend `hub.ts`/`core/agents.ts`/`AgentsPage`/`TeamTemplates` | 24+24+11+2 | Icons, colors, command palette, two lists, five defaults |

**Not a problem, per the owner:** the five CLIs genuinely differ in configuration,
hook dialects and status lines. Separate implementations are necessary.
The recorded problems were their scattered ownership:

1. No `Backend` type; one backend's knowledge spread across at least twelve
   match/if branches. Adding omp on 2026-09-07 missed `registry_save`.
2. Two tables for the same fact: resume dialects in `agents.rs::KNOWN` and
   `spawn.rs::resume_command`; frontend regexes mirror `find_word`.
3. Each hook contract spans two files: installation in spawn.rs and reading
   in agent_notifications.rs.
4. `refresh_hooks` detects backend-specific paths even though `launch.json`
   records the backend, with an extra kiro backfill case.
5. Five `render_*` functions duplicate the same skeleton: load notification
   center -> helper -> mcp_defs -> prompt file -> hooks -> model/effort ->
   `Rendered`. Three effort-delivery formats are scattered across functions.
6. The recorded `projects/` dependency on the then-to-be-deleted `team/`
   belongs to section A's first item.
7. Pane inspection is legitimate observation (section B), but backend-specific
   `sniff_*` functions belong beside the other backend knowledge.
8. The frontend maintains its own backend lists, icons, colors and command
   palettes (section C).
9. Generic tmux code contains codex's 200ms interval and kiro file-picker
   detection. These measured, screen-triggered adaptations are acceptable,
   but they remain backend knowledge in a generic module.

## E. Open Board Work

Draft snapshot, 2026-09-09; all listed in review:

#73 CLAUDE.md reduction and docs organization · #74 launch whole Agent Teams ·
#75 discard false waiting from `idle_prompt` · #76 terminal button prefers the
current recipient's window · #77 sidebar close/remove menu plus confirmation ·
#78 long-message read acknowledgment · #79 complete dispatch delivery ·
#80-#84 security review fixes (CSP still needs a real device) ·
#88 optional header paths/double-click copy · #89 restart in the card menu ·
#90 To all · #91 card selection follows through to the terminal drawer ·
#92 sidebar shows agent windows only · #94 desktop Agents three-column layout ·
#95 Board slider radii · #97 Chinese glyphs · #98 clickable confirmation style ·
#99 path links open in the Files drawer.

## F. Recorded Limitations (P3)

Emoji width is two cells in tmux versus one in xterm · bookmarks/recents use
cross-client last-writer-wins · iOS target · helper-textarea listeners after
xterm font-size changes · `newWindow` depends on `listPanes` ordering ·
the window switcher can show a non-active pane ·
`slow_rpc_does_not_block_fast_rpc` proves concurrency in only one direction ·
Team-related issues disappear with Team removal.
