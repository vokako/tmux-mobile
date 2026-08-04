# Unresolved Issues

## Emoji width mismatch: tmux (2 cells) vs xterm UnicodeV6 (1 cell)
- **Priority**: Low
- **Area**: Terminal rendering
- **Details**: tmux measures emoji as 2 cells; xterm.js with no unicode addon
  uses its UnicodeV6 table where emoji are width 1. A joined (`capture -J`)
  line containing emoji can therefore re-wrap at a different point than tmux
  displayed, shearing subsequent pane rows by one. Pre-existing, independent
  of the cursor-layout fix (whose cursor row is measurement-independent when
  the pane is full — see
  `docs/design-docs/pages/terminal-cursor-layout.md`). Fix would be loading
  `@xterm/addon-unicode11` AND switching `cellWidth` in
  `src/lib/terminal/cursor-layout.ts` to the same table; verify against
  tmux's actual wcwidth on macOS/Linux first.

## Prefs/bookmarks: cross-client last-writer-wins
- **Priority**: Low
- **Area**: Filesystem service / Files page
- **Details**: `save_bookmarks` / `set_pref('recentFiles')` persist the whole
  array; the server (`config.rs`) blindly replaces it. The client now guards
  its own races (generation counter + never-write-before-first-load, see
  `docs/requirements/pages/file-browser.md`), but two clients connected at
  once (phone + desktop) can still clobber each other's writes: both load at
  t0, phone stars A, desktop stars B from its pre-A snapshot → A is silently
  dropped. Deeper fix: server-side add/remove RPCs (`bookmark_toggle`,
  `fs_add_recent`) or merge semantics in `set_prefs`, after which the client
  guards become unnecessary.

## Team label collisions on same-basename workspaces
- **Priority**: Low
- **Area**: Sessions / PanePicker / team.svelte.js
- **Details**: `teamLabel()` strips the `-<6hex>` slug suffix for display, so
  `/a/demo` and `/b/demo` both render "demo" (the full room only lives in the
  row `title`, unreachable on touch). Accepted for now (rare case). Fix would
  be joining against `team_status().teams[].workspace` and disambiguating with
  a parent-dir segment when basenames collide.

## Team feature

### Team delivery cursor advances before client acknowledgement
- **Priority**: Medium · **Area**: Team / agora bus
- `Bus::wait` advances an agent's SQLite cursor before the MCP HTTP response is
  delivered. If the connection drops after cursor advancement but before the
  Agent CLI receives the ToolResult, reconnecting `wait` skips that message.
  Longer waits do not create the race, but server restarts and transport loss
  make it relevant. Fix requires an acknowledgement token or at-least-once
  delivery with message-id deduplication; do not treat SQLite persistence alone
  as proof of delivery.

### Launch failures / fired agents not surfaced in the UI
- **Priority**: Low · **Area**: Team
- After `MAX_LAUNCH_FAILURES` (3) failed launches the supervisor stops trying, but the UI shows no
  "failed" state — the agent just never appears as a chip/grid cell. Likewise a
  `fire`d or crashed agent. Fix: include a per-agent state (failed/offline) in
  the teams/roster payload and badge it in the roster + grid.

### Fired/offline agents keep a dead cell + stale pane in the grid
- **Priority**: Low · **Area**: Team / AgentGrid.svelte
- The desktop grid renders one cell per *employee* (all states), so a `fire`d or
  crashed agent leaves a cell pointing at a dead/closed pane (spinner or last
  frame). No visual "offline/failed" treatment. Fix: badge offline cells, or
  drop disabled employees from the grid.

### Missing agent window is skipped while presence is still online
- **Priority**: Medium · **Area**: Team supervisor
- The reconcile loop records an employee as launched when its persisted roster
  presence is still non-offline, even if no tmux window exists. If a window is
  killed shortly before server recovery, the stale `idle` row can therefore
  suppress relaunch. The loop does not reconsider that in-memory entry. Fix:
  require a matching live window before adopting an online employee, or retain
  a retryable state until either the pane appears or presence expires.

### Adopt-by-window-name treats a bare shell as a live agent
- **Priority**: Medium · **Area**: Team supervisor (reconcile.rs)
- `find_window_by_name` adoption checks only the window's NAME. A window whose
  agent CLI died (or, before the launch-script fix, never received its launch
  line) is adopted as if running: `launched` gets an entry, the loop never
  retries, and no error surfaces anywhere — this is what made the
  kiro-cli-term input-swallow failure (see team.md §5 launch-script note)
  invisible: the team looked started while builder/planner sat at bare
  prompts. Fix: at adopt time require `pane_current_command != shell` (or a
  recent heartbeat) before marking launched; otherwise kill the bare window
  and relaunch through the normal failure-counting path.

### Manager hire() launches on a hardcoded backend, no model/x-room nuance
- **Priority**: Low · **Area**: Team / agora hire + supervisor
- `agora::bus::hire` seeds an employee with `backend` absent; our supervisor
  defaults hires to "kiro" (the recovery/seed path) — a hire can't pick
  claude/codex, and the hired spec has no `model`. Acceptable for now; revisit
  if runtime hiring is used heavily.

### Template edits don't affect already-running teams
- **Priority**: Low · **Area**: Team
- The selected template is folded into employee specs and inline prompts at
  launch. Editing it only affects teams started afterward; running agents must
  be restarted to pick up changes. By design, but not surfaced to the user.

### Stale Chinese default.json on existing installs
- **Priority**: Low · **Area**: Team / templates
- `ensure_templates_seeded` only writes `default.json` when absent, so installs
  created before the English rewrite keep the Chinese roster. The English roster
  ships as `default-en.json` (added to the user config) but `default.json` is
  not migrated. Fix: offer a "reset to built-in" action, or version the builtin.

## iOS Target
- **Priority**: Low
- **Area**: Build / Platform
- **Details**: iOS build not yet implemented. Needs Xcode + xcodegen + Apple Developer account. Basic setup: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim && npx tauri ios init && npx tauri ios dev`

## Terminal scrollback reset on full rewrite
- **Priority**: Medium
- **Area**: Terminal
- **Details**: `writeToXterm` calls `term.clear()` when `buf.baseY > 0`, dropping all xterm-side scrollback on each full rewrite. tmux only returns ~500 lines per capture, so scrollback effectively caps at the server snapshot length, and any extra history xterm had built up is lost after each push. Fix needs delta-based content updates instead of full rewrite, or server-side scrollback streaming. Affects: `src/lib/terminal/Terminal.svelte:writeToXterm`.

## ~~Chat parser runs on every pane output~~
- Resolved 2026-07 by deletion: the chat feature (ChatView + parsers.js and
  the `waitingForInput`/`statusInfo` deriveds) was removed entirely, so no
  parser runs on pane output anymore.

## xterm helper textarea listeners on font change
- **Priority**: Low
- **Area**: Terminal / Keyboard
- **Details**: If xterm.js rebuilds its hidden helper textarea on font-size change (unverified), our `kbTa` reference and the `blur`/`focus` listeners become stale, breaking the keyboard lock guard. Needs confirmation with a test; if true, re-bind listeners after each font change. Affects: `src/lib/terminal/Terminal.svelte` fontSize $effect.

## newWindow picks last pane by listPanes order
- **Priority**: Low
- **Area**: tmux RPC / Window switcher
- **Details**: `newWindow` in `Terminal.svelte` relies on `listPanes(session)` returning the new pane at the end, which is not guaranteed to be race-free. Have the `new_window` RPC return the new `{session, window, pane}` directly so the UI can switch without re-querying.

## Window switcher may show non-active pane info
- **Priority**: Low
- **Area**: Window switcher
- **Details**: The `windows` derived in `Terminal.svelte` dedupes by window id using the first pane it encounters, not the active one. `current_command` / `pane_title` / AI icon badge may therefore come from a background pane. Prefer `pane_active` when choosing the representative pane.

## DA-response leakage: `?62;22;52c` appears as text in the prompt
- **Priority**: Medium
- **Area**: Terminal / xterm input filter
- **Details**: Occasionally the literal string `?62;22;52c` shows up in the terminal as if the user typed it. This is xterm.js's reply to a DA1 (Primary Device Attributes) query — full sequence `\x1b[?62;22;52c`. The query reaches xterm because `tmux capture-pane -e` re-emits ANSI sequences captured from programs that printed `\x1b[c`. xterm replies via `term.onData(...)`, our existing filter at `Terminal.svelte:605` drops `^\x1b\[[\?>=]?[\d;]*c$` — but the reply can be split across two `onData` invocations (e.g. `\x1b[?62;22` then `;52c`), defeating the regex. Fix candidates: (a) accumulate `onData` chunks across a short window and re-test the joined string before forwarding; (b) strip DA-related sequences server-side before pushing the snapshot, since they're never useful as visible content; (c) configure xterm to not auto-reply DA queries at all. Need a real-world repro to pick the cleanest path. Affects: `src/lib/terminal/Terminal.svelte:onData`.

## Frontend structural debt: fat components, flat src/lib, closure-locked gestures
- **Priority**: Medium (High for the gesture part)
- **Area**: Frontend architecture
- **Details**: 2026-07 audit findings, to be paid down in phases:
  - `Terminal.svelte` (2374 lines after 2026-07 slices): top-level pure
    logic (selection model, cursor-layout math) extracted + tested; the
    touch-gesture state machine (~1300
    lines, `touchToCell` / `recomputeSelUI` / `applySelectionToXterm` / ...)
    is nested inside an effect closure — zero test coverage, invariants
    (e.g. "endTouchScroll must never change kbLocked") guarded only by
    docs. Extraction plan: docs/design-docs/pages/terminal-gestures.md
    already specifies the state machine; add tests for pure geometry first,
    then extract behind an interface. Requires on-device regression.
  - `Files.svelte`: ~~the git client~~ (GitPanel.svelte) and
    ~~bookmarks/recents race guards~~ (persisted-list.ts, unit-tested) —
    done 2026-07; Files 2150 -> 1825 lines. Still worthwhile: the preview
    renderers (PDF/CSV/HTML/MD).
  - `App.svelte`: ~~reconnect state machine~~ — done 2026-07, extracted to
    `app/reconnect.ts` (framework-free, DI'd, 9 unit tests incl. cancel
    races and the watchdog). ~~Global design tokens + resets in one
    component's style block~~ — done 2026-07: moved to `src/app.css`.
  - ~~`src/lib/` is a single flat directory of 60+ files~~ — done 2026-07:
    reorganized into `core/ app/ terminal/ files/ sessions/ team/ ui/`.
  - Duplicated styles: light-theme `.scroll-btn` override copied in
    ChatView + Terminal; markdown-render CSS copied in Files + ChatView +
    Team. Fix via shared `ui/` primitives (e.g. MarkdownBody), not a
    shared stylesheet.
  - `ws.ts` is a mutable module-level singleton (10 top-level `let`s);
    fine today, but a wall if split-screen ever needs two simultaneous
    server connections.

## ~~ChatView is dead code pending a keep-or-delete decision~~
- Resolved 2026-07: owner chose delete. ChatView.svelte, parsers.js, the
  viewMode plumbing, the chat i18n keys, and the ws.js send_command wrapper
  are all removed (recoverable from git history). The `send_command` RPC
  still exists server-side.

## Backend clippy: structural findings deferred (2026-07-22)

Left as-is during the team/server module split (fixing them means
signature/behavior changes, which the mechanical-move discipline forbids
in the same pass):

- `too_many_arguments`: `server/connection.rs` `handle_connection` (9)
  and `handle_connection_ws` (11), `team/backends.rs` `prepare_codex` (9).
  The connection pair wants a `ConnContext` struct (token/machine_id/
  trackers/grace/team/notifications travel together everywhere); the
  backends one wants the existing `Extras` to absorb its loose params.
- `large_enum_variant`: `server::Outbound` — `InitCipher([u8;16], HalfCipher)`
  is ~700 bytes vs 24 for `Plain(String)`. One-shot variant per connection;
  boxing `HalfCipher` is trivial but touches the hot send funnel, so do it
  with a connection-path regression run, not blind.

All are pre-existing smells surfaced (not introduced) by the split;
everything mechanical from the same clippy run was fixed in-tree.

## `projects::tests::adopt_then_down_then_up_restores_the_workspace` is flaky (2026-08-05)

Seen failing once in the full `cargo test --lib -- --test-threads=1` run with
`api["cwd"]` empty instead of `"api"` ("cwd is stored relative to the
project"), then passing on its own and on a full re-run of the identical
binary — so it is order/state dependent, not a code defect. The suspect is
shared tmux state: `pick_workspace` chooses the workspace over ALL windows,
and with the test's two windows (`editor` at the root, `api` one level down)
there is no majority, so anything another test leaves behind in tmux can tip
which directory wins and make the stored relative cwd empty. Surfaced while
adding `tmm task`, which does not touch `projects`; left alone rather than
fixed blind. A fix means giving the test its own tmux socket (`-S`) so it
cannot see other sessions.
