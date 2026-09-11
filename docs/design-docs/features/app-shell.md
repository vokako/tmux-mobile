# App Shell — one chrome per context

**Reading this record (2026-09-10, board #159 / #154):** the original shell decision
and its verification below describe the rollout, including retired tab/bus
names; they are not a current navigation inventory or new runtime acceptance.
Later dated rules supersede that history. Configuration layout/control rules
belong in [design-language.md](design-language.md), with #155/#156 implementation;
motion in [motion.md](motion.md). This update does not reorganize global tabs.

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

Historical rollout; later scoped decisions below supersede its inventory.

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

Original rollout observations only, not current configuration acceptance:

Desktop 1440x900: rail with Hub/Sessions/Terminal/Team/Files/Settings, no
top bar, `main` padding 46px, Hub as the fresh-state default, Files
switching, Preferences opening at `left: 46px`. Mobile 390x844 (touch
emulation): bottom bar with active states, tapping a project window lands in
a rendering terminal with the bar visible, `keyboard-open` computes
`display: none` for it. Disconnected: top brand bar, no tab bar.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

Later scoped decisions supersede explicitly retired descriptions; a historical
measurement is not evidence that a new configuration implementation was tested.

### Tab swipe priority

App-level left/right tab swipe is lowest priority. Suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px).

### The phone's BACK gesture peels layers; it never reads as the browser navigating

App seeds/re-pushes `{app:true}` history entries and routes `popstate` to the visible page's own `onGoBack` closure — the Files page defined the contract (git panel → editor-with-confirm → preview → directory up → floor) and the owner named it the reference ("chat agent配置等页面 对于返回手势适配不太好 像是网页刷新了。像文件管理页面就很好", 2026-08-24). Hub peels its fixed local priority sequence, listed below; this is separate from its Escape/pointerdown listeners. On compact, a bare conversation lifts the project list (the list is the FLOOR, like Files' `/`: back with it open falls through, so it cannot cycle open/close). Terminal on compact follows the same floor rule (board #58): a bare terminal lifts the session drawer; with it open Back falls through/re-pushes instead of closing it, so no close/open cycle; a Chat-jumped Terminal returns to Chat before the floor lift. AgentsPage: pending delete dialog → whichever editor is open (compact: the editor takes the screen) → floor. Settings: the open category → the category list (its compact layout is the same drill-down — the list is the first screen, a chip row was a third navigation species; owner, 2026-08-25) → floor (leaves the page). A CONSUMED pop re-pushes in App so the next back always has an entry to spend; only an unconsumed one reaches the re-push floor. Pages register via the same `onGoBack={(fn) => xGoBack = fn}` prop pattern Files uses — a page never installs its own `popstate` listener.

**Hub dispatch ownership** (board #119, 2026-09-09): one per-Hub
`hub-back.ts` registry replaces the closure-wide flag chain. Its fixed order
is lightbox -> ContextMenu -> palette ->
non-busy action confirmation -> trash confirmation ->
agent picker -> create project -> rename -> agent filter -> Files delegate ->
drawer close -> compact project-list floor. This is not registration order
or a chronological stack. Each callback reads its owner's live state;
the registry keeps no duplicate open flags. Missing callbacks fall through.
Each disposer belongs to one registration, so an old owner cannot remove
its replacement even when both register the same function.

Hub still publishes one callback through `onGoBack`; cleanup removes each
owner's registrations. Board #168 (2026-09-11) removes the agent tap menu,
recipient popup and send-arm mechanism whole, including their Back slots
and the obsolete card-menu capture effect. Composer registers only palette;
Hub owns the other eleven slots. All surviving guards and their relative
order are unchanged. The original
guards remain: a truthy palette consumes even
without items, busy action confirmation falls through, and Files is called
only for an open Files partition and consumes only on a true return.
Message actions/raw view are not Back layers; Board delegation is not added.
The Escape/pointerdown listeners and their capture order stay in Hub,
including the terminal/Files/Board focus territories. The registry neither
installs listeners nor calls browser history. Desktop still uses in-pane
Files Back/breadcrumbs; a narrow desktop is not a reason to trap browser Back.
Unit traces cover every layer and overlap; a mounted Hub test exercises
ContextMenu/palette priority, the compact floor and registration cleanup.
Since board #134 (2026-09-09), Feed owns Copy/Raw state and registers live
`isOpen`/`outside`/`escape` callbacks with Hub's existing capture listener.
No flags are copied and no new listener or Back layer is installed.
Board #136 (2026-09-09) moves the Drawer view without moving those listeners
or its parent-owned state. Its controlled Files directory and Back callback
preserve the existing dispatcher path; Terminal stays keyed by target and
hidden partitions retain the original visibility gates.

**The dance is the phone's** (review, 2026-09-03). Everything above — the seed, the re-push after a consumed pop, the `popstate` router — is gated on `layout.isTouchDevice`, and `navPush()` is a no-op that returns `false` on a desktop layout. A desktop browser has no back gesture to protect, and the unconditional seed + re-push had made the app a page you could not Back out of: the browser's Back button did nothing, forever. On the phone nothing changed. Callers that later spend an entry with `history.back()` (the gear's toggle via `prefsPushed`, Settings' compact drill via `onDrill`'s return value → `drillPushed`) only do so when their push was real, so a narrow desktop window never `history.back()`s out of the app. The gate is reactive: switching the layout mode in Settings installs or removes the router live (entries already pushed before a switch to desktop are simply popped by the browser with nothing listening). `App.source.test.ts` pins the gate. Files' own `navPush` (a page-local push, Files-owned) still pushes on desktop; those entries are consumed silently by Back until the app's real root is reached — harmless, but the one remaining desktop push.

### Where you left off is persisted, and the tab is part of it

`tmux_state` carries `{ page, terminalTarget, terminalSession, terminalCommand, splitLayout, splitCells }` and is written whenever `connected` — it used to be gated on `terminalTarget`, so reading the chat and refreshing dropped you on the device default (owner, 2026-08-19: "每次切换或者刷新都会变"). Restore is `restorePage` (`src/lib/app/nav-state.ts`, pure + tested): an unknown/stale name (a retired tab, an older build) falls back to `defaultPage` — terminal on touch, Hub on desktop — never to a page that no longer renders. The Hub's open project is `tmux_hub_project` via `hubPrefs.setProject`, verified against the current list on load (a project can be deleted between two visits) and only then falling back to the top row. Files is the deliberate exception: its cwd FOLLOWS the tmux session's cwd — but the SESSION it follows is whichever the user touched LAST (a terminal pane or the chat's selected project; it used to be terminal-only, so browsing a project in chat never moved Files — owner, 2026-08-22), and switching projects PARKS the in-Files browse position per session (in-memory, not a preference) and restores it on return, with the follow rule still outranking the parked position when that project's real cwd moved meanwhile. That parked map is MODULE-scoped in Files.svelte (owner, 2026-08-28: "每个 project 自己记录自己的 current路径"): every Files instance shares it and it outlives any one instance — which is what lets the Hub's drawer mount a fresh Files per open and still wake up where that project left off (a new instance restores its session's parked cwd at mount and parks on unmount).

### Shell motion follows the one vocabulary (2026-09-03)

Every state change in the shell moves on the tempo tokens and nothing else
([motion.md](motion.md)): the tab bar and gear cross-fade colour on `--t-fast`
and the gear TURNS 30° while Settings is open (a state is a movement, not a
swap); the reconnect banner, the rail's insertion line and the vitals sidebar strip's
first reading `.appear` (opacity only — never height, so the banner cannot
push the page); the split toggle is a `.state-ctl`; the server switcher's
180°-symmetric swap glyph is a `.quarter-turn` that turns 90° while its
popover is open (180° would look unchanged; the popover itself uses the
shared popover intro); the page slides still under
`prefers-reduced-motion` like the compact drill pair. The rail's icons sit in
a per-slot `.rail-slot` wrapper because Svelte's `animate:flip` must be the
keyed each's only child: the wrapper carries the dragged icon's inline
transform (no transition — the finger is the animation), so on release the
flip measures from under the pointer and the icon SETTLES into its new slot
on `moveMs()` instead of jumping back and sliding. `transform` is deliberately
absent from `.rail-btn`'s transition list for the same reason.

### The desktop Hub's project sidebar collapses, from ONE control that turns (2026-09-11, board #174)

Owner: "现在左侧侧边栏可以加一个折叠展开的按钮，在左侧边栏右上角的位置，折叠展开最好是有动画，不是直接跳". The Hub's project sidebar (desktop only — the phone has its sheet + scrim) collapses to a 0 track and expands again, both on `--t-move` by the REVEAL technique motion.md principle 8 describes (content pinned at its final width, the grid track's `@property` factor moves; `hub/reveal.ts`). ONE control: a `CommandButton` (`variant="icon"`, `chevron-right`, `expanded={!collapsed}`, `inside`) defined once in Hub as the `sideToggle` snippet and rendered in the sidebar head's right end while open and at the header's left edge — where the phone's menu button stands — while collapsed; the glyph is one chevron that turns 180° (principle 4), never two icons. `inside` keeps the control at rest in both states: standing inside the region it discloses, the visible region is the whole signal, so it does not wear the engaged wash the roster's disclosure (content elsewhere) does — at rest is achromatic. The state is app-wide (`hubPrefs.sidebarCollapsed`, `tmux_hub_sidebar`), not per project: where the project list is, is a property of the window. Collapsed at rest the content is UNREACHABLE, not narrowed — `visibility: hidden` takes it out of sight, tab order and assistive tech; it is visible only while the move uncovers or withdraws it. Measured at 1440: collapse 240→2.9px over 12 moving frames, `.sidebar` box {240} throughout, hidden at rest; expand 0→237px, {240}; with the drawer open the chat column ends exactly where it rests (920 = 920). Guards: `Hub.source.test.ts` (rest factor, hidden rule, pin/anchor/move order both ways, one snippet rendered in two slots), `Sidebar.source.test.ts` (the head carries the handed-in control, no second button species), `hub-prefs.test.ts`, `CommandButton.mount.test.ts` (horizontal chevrons turn; `inside` = no wash).

### The chosen tab is marked by ONE highlight that travels (2026-09-04, #86)

**Current scope:** the desktop rail retains its travelling wash. The phone
tab bar's wash described below was retired on 2026-09-05; phone tabs select
by foreground ink alone, with no background or marker (motion principle 14).
The original 2026-09-04 description and owner quote remain historical evidence,
not permission to restore the phone wash. Configuration Segmented controls
still use a travelling marker; the phone-tab exception is not a global rule.

The tab bar and the rail each hold a single `.slide-pill` (motion.md §1.14,
`ui/indicator.ts`): the accent WASH behind the active phone tab (inset so it
hugs icon + label) and behind the active rail icon. When the page changes the
wash GLIDES to the new item on `--t-move` while the buttons' own ink still
cross-fades on `--t-fast` — the change reads as a movement rather than one
icon switching off and another on. The buttons themselves paint no active
background any more; the pill carries it. There is no bar: the first cut drew
a 2px line under the tab / beside the icon, and the owner retired it the same
day ("线条都去掉，直接用 icon 上面的背景阴影来滑动"). Two mechanics matter:

- **Measured by layout offsets, never client rects.** The first cut read
  `getBoundingClientRect()` and divided by `uiZoom()`; on the phone the marker
  "乱滑" — it slid somewhere wrong, then corrected itself. A rect includes
  every transform on the way: the tab's press `scale(0.95)` while the finger is
  still down, a `.rail-slot` mid-`animate:flip`, the root's CSS `zoom`. The
  action now walks `offsetParent` from the item and from the container to
  their common root and subtracts (`boxFromOffsets`, pure and tested): the
  layout box is where the item WILL rest, so the pill goes straight there.
- **Hidden while the rail is being rearranged.** `hidden: !!railDrag`
  collapses the pill during an icon drag and re-measures on release, with a
  second read after `moveMs()` for a layout that is still settling.
### The rail explains itself on hover (2026-09-04, #86)

A rail icon, the server switcher, the gear and the split toggle wear
`use:hoverInfo` (motion.md §1.16): resting a pointer opens the ONE shared
card — a page's name with its shortcut as the note (read live from
`shortcuts`, so a rebinding shows at once; only Terminal and Files have one),
the switcher's current server with its address and connection state, the
gear's and the split toggle's names. Their native `title`s were removed: a
browser tooltip next to the card is a second tooltip species, which is the
thing the rule forbids. `aria-label`s stay — the card is pointer/keyboard-
focus only and the label is for everyone; on touch there is no card and
nothing was lost, since touch never showed a title either. The phone's tab
bar gets no card for the same reason.
