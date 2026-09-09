# TODO: Gaps Against the Tenets

> Draft · 2026-09-09. This document is the ONE entry point for open problems
> and remaining work, not decisions. Decisions belong in `tenet.md`; rules
> belong in `guidance/`. (`docs/unresolved.md` was folded in here on
> 2026-09-09, board #104 — its resolved entries live on in the design docs'
> dated rules and in git history.)
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
- [x] **Consolidate `docs/unresolved.md` and this document** (board #104,
  2026-09-09): unresolved.md's surviving details are folded into the matching
  items here and the file is gone; resolved and deleted-feature entries were
  dropped (their record is the design docs' dated rules and git history).
- [ ] **Review process:** define how agents review separate disciplines,
  with one reviewer per dimension, the corresponding guidance checklist,
  and conclusions recorded in board notes.

## B. Correctness (P1)

- [x] Registry definition edits reach every already-spawned agent on restart
  (board #113, 2026-09-09): provenance lives in `launch.json` (`agent_def` /
  `team`+`member` — the recipe is the declaration, no slots column), and
  `refresh_agent` resolves the CURRENT def through it, so uniquified windows
  and team members re-materialize too; a deleted def degrades soft.
- [x] `is_managed_in` no longer re-arms merely because kiro recreates the
  `KIRO_HOME` subtree after `agent_remove`. It now requires `launch.json`
  or a pre-recipe `agents/<name>.json` (board #112, 2026-09-09).
- [ ] The roughly 1300-line touch-gesture state machine is embedded in
  `Terminal.svelte`'s effect closure with no tests; `kbLocked` invariants
  (e.g. "endTouchScroll must never change kbLocked") rely only on
  documentation. Plan: terminal-gestures.md already specifies the state
  machine — test the pure geometry first, then extract behind an interface;
  needs an on-device regression pass.
- [x] #108: a bounded stateful filter covers fragmented/coalesced `onData`
  replies. The source of the original `?62;22;52c` report still requires
  measurement: installed xterm.js 6.0.0 emits DA1 as ONE complete callback
  even for fragmented queries, so the old "naturally split replies"
  attribution was an unverified hypothesis — if the text recurs, capture the
  `onData` payloads and pane output first; never strip bare printable
  `?62;22;52c` on assumption (see terminal-rendering.md § xterm DA filtering).
- [x] #109: full snapshots restore history. The false-tail event, lost news
  flag and repeated redraw caused by synchronous `clear()` are fixed by
  in-frame `CSI 3J`.
- [x] Telemetry keyed by window INDEX (board #120, 2026-09-09): every store
  (turn edges, deliveries, activity, vitals, recovery) now keys on the window
  NAME, resolved once at ingest (`resolve_pane_id` returns `#{window_name}`);
  state.db v20 migrates readable; the rename-between-hook-and-consume post
  loss went with the index → name round-trip. Still open from the same
  cluster: identical bodies confuse receipts; delivery lacks backpressure;
  `SPAWN_CAP` includes windows we do not own.
- [ ] Backend parity, blocked on measurement rather than effort: claude's
  `/` palette is not transcribed (mechanical once captured — transcribe the
  popup with pinned captures like codex's); claude/codex/grok have no
  auto-continue (their transient-error paints are uncaptured, and a guessed
  pattern would type `continue` into a working agent — capture each verbatim
  into a test first); codex has no StopFailure hook event (binary checked),
  so a codex `failed` state has nothing to wire until the CLI grows one.
- [ ] Smoke-test CSP on a real device with Markdown, PDF, Mermaid and Hub.
  Browser/PWA responses have no CSP header.
- [ ] The Android signing key was in git history before 60992d4 and has not
  been rotated, by owner decision. Keep this recorded.
- [ ] Isolate the flaky `adopt_then_down_then_up_restores_the_workspace` test
  on its own tmux socket (`-S`): `pick_workspace` votes over ALL windows and
  the test's two windows have no majority, so anything another test leaves in
  the shared tmux can tip which directory wins (seen once, 2026-08-05).
- [ ] Consolidate three shell quoters: `agent_notifications.rs` (the only one
  compiled on Android, pinned by hook files on disk), `tasks.rs::sh_quote`,
  and `projects/backends/shared.rs::shell_quote`.
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
- [ ] Structural clippy findings (deferred 2026-07-22; fixing them changes
  signatures, which the mechanical-move discipline forbade in that pass):
  `handle_connection`(9) / `handle_connection_ws`(11) want a `ConnContext`
  struct. `Outbound::InitCipher` is ~700 bytes vs 24 for `Plain`
  (large_enum_variant) — boxing is trivial but touches the hot send funnel,
  so do it with a connection-path regression run, not blind.
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

- Emoji width: tmux measures 2 cells, xterm's UnicodeV6 table 1 — a joined
  (`capture -J`) line with emoji can re-wrap differently and shear pane rows.
  Fix = `@xterm/addon-unicode11` AND the same table in
  `terminal/cursor-layout.ts` `cellWidth`; verify against tmux's wcwidth first.
- Bookmarks/recents are cross-client last-writer-wins: the client guards its
  own races (generation counter, see file-browser.md), but phone + desktop
  writing from parallel snapshots still clobber each other. Deeper fix:
  server-side add/remove RPCs (`bookmark_toggle`, `fs_add_recent`) or merge
  semantics in `set_prefs`, after which the client guards go.
- iOS target: not implemented (Xcode + xcodegen + Apple Developer account;
  `rustup target add aarch64-apple-ios aarch64-apple-ios-sim && npx tauri ios
  init && npx tauri ios dev`).
- xterm helper-textarea listeners after a font-size change: if xterm rebuilds
  its hidden textarea (unverified), the `kbTa` reference and blur/focus
  listeners go stale and break the keyboard-lock guard — confirm with a test
  before re-binding per font change (`Terminal.svelte` fontSize $effect).
- `newWindow` relies on `listPanes` returning the new pane last; have the
  `new_window` RPC return the new `{session, window, pane}` directly.
- The window switcher dedupes by window id with the FIRST pane it meets, so
  command/title/AI badge can come from a background pane — prefer
  `pane_active`.
- `slow_rpc_does_not_block_fast_rpc` proves concurrency in one direction only
  (measured 2026-08-20: the 5.3 MB download frame sits ahead of the pings in
  the socket buffer, so a serial server can look concurrent). It prints
  `inconclusive` instead of crying wolf; closing the gap needs an RPC whose
  server work is slow while its response stays small — every current one
  couples the two.
