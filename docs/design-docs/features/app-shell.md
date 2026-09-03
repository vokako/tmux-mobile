# App Shell — one chrome per context

## Context

Until agents-v2 the app had ONE shell on every platform: a top bar with four
nav pills (Sessions / Terminal / Team / Files) plus a gear. The Hub made it
five, and the top bar started working against both platforms at once:

- On desktop the Hub already contains the projects list and an embedded
  terminal, so the top pills mostly duplicated what the sidebar shows — while
  eating ~45px of vertical space on the page where vertical space is the
  product (a terminal).
- On a phone the pills sit in the hardest-to-reach part of the screen, and
  every added page shrank the tap targets.

## Decision

Three shell shapes, chosen by context, never mixed:

| context | chrome | why |
|---|---|---|
| connected desktop | **left icon rail** (46px, fixed): Hub / Sessions / Terminal / Agents / Files, gear at the bottom | the VSCode/Slack pattern — switching stays one always-visible click, the top edge belongs to content, and the rail composes with the Hub's own sidebar instead of stacking a second horizontal bar over it |
| connected mobile | **bottom tab bar** (icons + labels, safe-area padded) | thumb reach; the top edge goes back to content. Hidden under `html.keyboard-open` so immersive typing (terminal, editor) costs nothing — the ONLY writer of that class is App's viewport handler, so there is no second source of truth |
| disconnected | the old **top brand bar** with the gear (both platforms) | before auth there is nothing to navigate; brand + settings is the whole story |

Desktop lands on the **Hub by default** (fresh state); a guard falls back to
Sessions once the bus probe answers negative (`hub`/`agents` need the bus).

The Hub is CHAT-FIRST (owner-directed rework 2026-08-01): a project's default
view is its conversation, full width; the terminal is a DRAWER behind a
button — terminal is terminal, project is project, never parallel equals.
Agent definitions live on their own Agents page (rail icon / bottom-bar tab).
The Team tab is retired — the Hub's per-project chat + spawn + telemetry
replaced it; the team_* backend stays (the bus IS the hub substrate) and
legacy tmm-team-* sessions appear as plain sessions in the list.

## Mechanics worth recording

- The rail is `position: fixed`; content clears it via `main.with-rail
  { padding-left: 46px }` rather than a flex re-nest, so the page-layer
  keep-alive structure (Team/Files/Terminal/Hub stay mounted) is untouched.
- Every nav item — rail icons, the rail's server control, the phone's tab
  bar, the gear — is in the Tab order and shows the global
  `button:focus-visible` ring (accent outline, 2px offset); the current page
  is `aria-current="page"`. They all carried `tabindex="-1"` from v0.3.0 with
  no recorded reason, which left the whole navigation unreachable by keyboard
  (review, 2026-09-03). `App.source.test.ts` pins it.
- Fixed overlays cannot assume a top bar anymore. `Preferences` used
  `inset: calc(49px + var(--sat)) 0 0`; it now reads
  `--shell-top` / `--shell-left`, which App sets per context (49px top when
  disconnected, 46px left on connected desktop, zero on connected mobile).
  Anything else `position: fixed` that wants to clear the chrome should use
  the same two vars.
- The mobile tab bar hides with CSS (`:global(html.keyboard-open) .tabbar
  { display: none }`), not JS — it inherits exactly the keyboard lifecycle
  the terminal already fights hard to get right, including the Android
  native-height path.
- Tab order for swipe navigation is unchanged (`tabs()`), and the gear is a
  toggle, not a page — it never participates in swipes.

## The rail has two groups: where you work, and what you configure

Top: Chat, Terminal, Files — the places work happens. Bottom, after the
spacer: Agents (agent/skill/MCP definitions) and Settings. Agents moved down
there on 2026-08-19 (owner ask): it is a configuration surface, visited when
setting something up rather than while working, and pairing it with the gear
says that without a label. Order within the pair puts Agents above the gear,
which stays the last item in the rail as the app-wide convention.

## The tab slide belongs to the swipe, not to the app

`switchTab` plays a one-shot `slide-in-left/right` on the page layer — but only
on a TOUCH layout (`layout.isTouchDevice`, 2026-08-19). It is the visual half
of the swipe gesture: content follows the finger, so the direction carries
meaning. A desktop switches tabs by clicking a rail button, where nothing moved
horizontally and the slide just made the page lurch (owner report) — most
visibly on the wide three-column pages, where a whole workspace slid. The
animation is not deleted, because on a phone removing it would leave a swipe
whose content does not follow the thumb.

## Verified

Desktop 1440x900: rail with Hub/Sessions/Terminal/Team/Files/Settings, no
top bar, `main` padding 46px, Hub as the fresh-state default, Files
switching, Preferences opening at `left: 46px`. Mobile 390x844 (touch
emulation): bottom bar with active states, tapping a project window lands in
a rendering terminal with the bar visible, `keyboard-open` computes
`display: none` for it. Disconnected: top brand bar, no tab bar.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### Tab swipe priority

App-level left/right tab swipe is lowest priority. Suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px).

### The phone's BACK gesture peels layers; it never reads as the browser navigating

App seeds/re-pushes `{app:true}` history entries and routes `popstate` to the visible page's own `onGoBack` closure — the Files page defined the contract (git panel → editor-with-confirm → preview → directory up → floor) and the owner named it the reference ("chat agent配置等页面 对于返回手势适配不太好 像是网页刷新了。像文件管理页面就很好", 2026-08-24). Hub peels in tap-outside/Escape order: context menu → agent menu → recipient picker → `/` palette → armed interrupt → confirm dialogs → team picker → create dialog → rename → terminal drawer (via `closeDrawer`) → and on compact, a bare conversation lifts the project list (the list is the FLOOR, like Files' `/`: back with it open falls through, so it cannot cycle open/close). Terminal on compact follows the same floor rule (board #58): a bare terminal lifts the session drawer; with it open Back falls through/re-pushes instead of closing it, so no close/open cycle; a Chat-jumped Terminal returns to Chat before the floor lift. AgentsPage: pending delete dialog → whichever editor is open (compact: the editor takes the screen) → floor. Settings: the open category → the category list (its compact layout is the same drill-down — the list is the first screen, a chip row was a third navigation species; owner, 2026-08-25) → floor (leaves the page). A CONSUMED pop re-pushes in App so the next back always has an entry to spend; only an unconsumed one reaches the re-push floor. Pages register via the same `onGoBack={(fn) => xGoBack = fn}` prop pattern Files uses — a page never installs its own `popstate` listener.

**The dance is the phone's** (review, 2026-09-03). Everything above — the seed, the re-push after a consumed pop, the `popstate` router — is gated on `layout.isTouchDevice`, and `navPush()` is a no-op that returns `false` on a desktop layout. A desktop browser has no back gesture to protect, and the unconditional seed + re-push had made the app a page you could not Back out of: the browser's Back button did nothing, forever. On the phone nothing changed. Callers that later spend an entry with `history.back()` (the gear's toggle via `prefsPushed`, Settings' compact drill via `onDrill`'s return value → `drillPushed`) only do so when their push was real, so a narrow desktop window never `history.back()`s out of the app. The gate is reactive: switching the layout mode in Settings installs or removes the router live (entries already pushed before a switch to desktop are simply popped by the browser with nothing listening). `App.source.test.ts` pins the gate. Files' own `navPush` (a page-local push, Files-owned) still pushes on desktop; those entries are consumed silently by Back until the app's real root is reached — harmless, but the one remaining desktop push.

### Where you left off is persisted, and the tab is part of it

`tmux_state` carries `{ page, terminalTarget, terminalSession, terminalCommand, splitLayout, splitCells }` and is written whenever `connected` — it used to be gated on `terminalTarget`, so reading the chat and refreshing dropped you on the device default (owner, 2026-08-19: "每次切换或者刷新都会变"). Restore is `restorePage` (`src/lib/app/nav-state.ts`, pure + tested): an unknown/stale name (a retired tab, an older build) falls back to `defaultPage` — terminal on touch, Hub on desktop — never to a page that no longer renders. The Hub's open project is `tmux_hub_project` via `hubPrefs.setProject`, verified against the current list on load (a project can be deleted between two visits) and only then falling back to the top row. Files is the deliberate exception: its cwd FOLLOWS the tmux session's cwd — but the SESSION it follows is whichever the user touched LAST (a terminal pane or the chat's selected project; it used to be terminal-only, so browsing a project in chat never moved Files — owner, 2026-08-22), and switching projects PARKS the in-Files browse position per session (in-memory, not a preference) and restores it on return, with the follow rule still outranking the parked position when that project's real cwd moved meanwhile. That parked map is MODULE-scoped in Files.svelte (owner, 2026-08-28: "每个 project 自己记录自己的 current路径"): every Files instance shares it and it outlives any one instance — which is what lets the Hub's drawer mount a fresh Files per open and still wake up where that project left off (a new instance restores its session's parked cwd at mount and parks on unmount).
