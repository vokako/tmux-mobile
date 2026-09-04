# Motion — the app's animation principles and the 2026-09-03 plan

Written on owner request (2026-09-03: "你现在是我们的 ui 动效设计师，你可以帮
我们定一下 app 整体的设计准则，以及当前项目优化实施方案"). This is the
normative motion contract; [design-language.md](design-language.md) §1 keeps
the one-paragraph summary and links here. `ui/motion.source.test.ts` pins the
vocabulary.

## 1 · Principles

1. **Motion explains a cause.** Every animation answers "what just happened
   and where did it go": a caret turns because a section opened, a chip fades
   in because you attached something, a card slides because its status
   changed. Decoration without a cause is banned; so is animating a state the
   user did not touch (a poll refresh that re-sorts silently must not dance).
2. **Two tempos, no third.** `--t-fast` (120ms) is micro feedback: colour,
   border, a glyph turning, a chip appearing. `--t-move` (200ms) is something
   that MOVES or resizes: a drawer, a card changing place, a sheet rising.
   Spinner tempos are semantic (0.6s loading, 2.2s "a turn is open", 1.3–5s
   presence) and are not tokens. A raw `0.15s` / `160ms` in a component is a
   drift to fix, not a third tempo.
3. **Transform and opacity only.** Never animate height, width, top, left,
   margin or padding: layout animation is what makes a UI feel slow, and in
   this app several surfaces (the chat feed, the terminal, the composer) read
   their own geometry mid-frame and would measure a lie. An element that
   appears grows nothing — it fades or rises 6px into a slot that is already
   its size.
4. **A glyph turns, it is never swapped.** An arrow that reads up then down,
   a caret that reads closed then open, is ONE glyph rotated (180° / 90°). Two
   glyphs swapped is a cut, not a movement, and it is the thing the owner
   noticed first ("上箭头变成下箭头 可以用旋转过渡"). Unrelated glyphs (trash →
   caret, copy → check) may still swap: there is no rotation that means it.
5. **Enter with motion, leave with a cut.** Intros animate; exits snap. Exit
   should be faster than enter, and the fastest exit is none — an outro keeps
   a dead element in flow while the list under it has already moved, and it
   doubles a grid cell for a beat. The rule frees every `{#if}` from needing
   a matching `out:`.
6. **Arrive decelerating, spin linear.** `ease-out` for anything that enters
   or settles; `linear` for constant-rate rotation (spinners) and for the
   navigation slide (owner rule). No springs and no bounces: the app's
   surfaces are dense and technical, and overshoot on a 6px rise reads as
   jitter.
7. **The finger is the animation.** While a drag or swipe is in progress the
   element follows the pointer with an inline transform and NO transition; a
   transition only plays on release (spring back on `--t-fast`, commit on the
   navigation slide). Files' edge-swipe and the rail's icon drag are the
   references.
8. **A popover is placed first, then grows from its anchor; navigation has
   one slide; sheets slide with a scrim.** A menu, select, picker or hover
   card is measured invisible, positioned, and only THEN plays a short
   opacity + scale(0.96 → 1) intro from the corner that touches its trigger
   (`.pop-layer`, `popOrigin()`), so it is never seen at the wrong place and
   still feels like it came from where you clicked (owner, 2026-09-04 #86:
   "右键或者长按出来的选项卡出现什么的，都可以加动效" — this replaces the
   earlier "popovers do not animate"). Its exit is a cut. Page-level slides
   are touch-only. Desktop rails and sidebars have no motion behind them —
   but the HIGHLIGHT that marks the chosen tab glides (principle 14).
9. **Never touch the terminal's box.** No transform, transition or animation
   on `.term-wrap`, `.xterm-wrap`, a split `.cell`, or any ancestor of an
   xterm instance: a resting transform turns it into a containing block and
   breaks every fixed popover, and a transitioning size makes the fit run on
   a mid-flight measurement. Chrome AROUND the terminal (bars, chips, toasts,
   the to-tail button) moves freely; an xterm ancestor may fade (opacity) and
   nothing else.
10. **The feed decides its own scroll.** The Hub feed disables scroll
    anchoring and drives `scrollTop` itself; any animation that changes a
    block's height mid-flight (a `slide` on a fold) re-creates the documented
    blink loop. Feed blocks fade/rise in with transform only, and a fold is a
    turned caret plus a cut.
11. **Motion is optional.** Every loop stills and every atom goes inert under
    `prefers-reduced-motion`; a JS duration goes through `moveMs()` /
    `fastMs()` so it becomes 0 there. The static state must read on its own —
    a stilled spinner is still a partial ring, a stilled live-dot still wears
    its halo.
12. **Animate one or two things per view.** When a card is selected, its
    frame cross-fades and nothing else; when a feed block arrives, it rises
    and the to-tail dot pops — that is the budget. A screen in which five
    things move at once is a screen in which nothing is explained.
13. **Rapid changes cancel cleanly.** State correctness never waits for
    `transitionend`/`animationend`; CSS transitions replace themselves, an
    intro class is on the element that mounts so a remount restarts it, and
    a whole list mounting at once (history load, project switch) is gated
    with `class:appear={fresh}` so two hundred rows do not fade in together.
14. **One highlight that travels.** A tab bar, a rail, a segmented control
    has ONE marker for "chosen", and when the choice changes the marker
    glides to the new item (`.slide-ind` / `.slide-pill`, `ui/indicator.ts`)
    instead of one item switching off and another switching on. The items'
    own colour still cross-fades (`.state-ctl`); the travelling marker is
    what makes the change read as a movement (owner, 2026-09-04: "点击不同的
    标签页面，过度的一些选项卡高亮过度的时候，都可以有动效").
15. **A load unfolds, it never flashes.** Content that arrives after a
    switch does not pop in as a finished wall: while it loads, the slot
    shows either the PREVIOUS content dimmed (stale-while-revalidate, Files'
    pattern) or, when there is nothing previous, a skeleton of the right
    shape that appears only after 150ms (`.skel-wrap`); when the data lands
    the rows unfold with a ~30ms stagger (`.reveal` from the top,
    `.reveal-tail` from the newest message at the bottom). Never a blank
    frame followed by everything at once (owner, 2026-09-04: "切换过去然后
    看到东西闪出来 … 像一个东西在你面前展开一样加载").
16. **Hover explains.** On a pointer device, resting on a thing that has
    more to say (an agent card, a project row, a rail icon, a session, a
    file) opens ONE shared hover card (`use:hoverInfo`, `ui/HoverCard`) after
    a short dwell, instantly when hopping between neighbours, with the
    thing's live facts — never a second tooltip species and never a native
    `title` beside it. Touch has no hover: there the long-press menu is the
    "more" gesture, and the card is not shown.

## 2 · Vocabulary (app.css, one copy)

| atom | meaning | tempo |
|---|---|---|
| `.chev` + `.open` | disclosure caret turns 90° (closed ▸ → open ▾) | `--t-move` |
| `.flip` + `.on` | up↔down arrow turns 180° | `--t-move` |
| `.appear` | something enters: fade in | `--t-fast` |
| `.appear-rise` | a block/banner enters: fade + rise 6px | `--t-move` |
| `.appear-pop` | a badge/dot/small chip enters: fade + scale from 0.6 | `--t-fast` |
| `.state-ctl` | a control's selected/active clothes cross-fade (border, background, colour, inset ring, opacity) | `--t-fast` |
| `animate:flip={{ duration: moveMs() }}` | a keyed list reorders — the ONE Svelte directive in use (`ui/motion.ts`) | `--t-move` |
| `.side-scrim` / `.dlg-backdrop` fade-in, `sheet-up` (`translateY(100%) → none`, only for a layer whose resting transform is none) | sheets rise with a scrim (design-language §1) | `--t-move` |
| `drill-in-right` / `drill-in-left` (app.css, one copy), `slideInLeft/Right` (App) | navigation, touch-only | 120ms linear |
| `.pop-layer` + `.ready`, `--pop-origin` from `popOrigin()` | a placed popover grows from its anchor corner (opacity + scale 0.96→1); exit is a cut | `--t-fast` |
| `.slide-ind` (bar) / `.slide-pill` (filled pill) + `use:slideIndicator` | the one travelling highlight of a tab bar / rail / segmented control (`--ind-x/y/w/h` written by the action from client rects ÷ `uiZoom()`, so a nested or zoomed item measures right; `.ready` after the first measure so it is born in place; `hidden: true` collapses it while the container is being rearranged). A segmented row is `ui/Segmented`, which carries its own pill | `--t-move` |
| `.reveal` / `.reveal-tail` on a container | a loaded list unfolds, rows staggered 30ms from the top / from the newest at the bottom; backwards fill only | `--t-move` + stagger |
| `.skel` (+ `.skel-wrap`) | a loading placeholder of the coming shape with a slow shimmer, invisible for the first 150ms | 1.4s loop, stilled |
| `use:hoverInfo={() => info}` + `ui/HoverCard` | the one hover card (title / text / label→value rows / note), 380ms dwell, 60ms hop, pointer + keyboard focus only | `--t-fast` intro |

Usage: wrap the `<Icon>` in `<span class="chev" class:open={x}>`; put `.appear*`
on the element that mounts inside `{#if}`; add `.state-ctl` to a segmented
button / tab / row that has an `.active`/`.on`/`.sel` state; key the `{#each}`
before adding `animate:flip`. `svelte/transition` is not used — intros are the
classes, exits are cuts (principle 5) — and `ui/motion.source.test.ts` forbids
importing it.

## 3 · Implementation plan (2026-09-03)

Three surveys (Hub · shell/Settings/ui · terminal/Sessions/Files/Projects/
Team) found ~120 state or position changes that snap. Ordered by how often a
user meets them; each item is a one-class change unless noted.

**Wave 1 — the vocabulary and the rule violations** (done with this doc):
tokens mirrored in `ui/motion.ts`; atoms in app.css; `.side-row` 160ms and
the scrim/to-tail snaps normalised; six spinner loops without a
reduced-motion still (App, Settings, Hub attach, AgentGrid, Team,
TeamTemplates) guarded; Hub's private `.chev` promoted to the atom.

**Wave 2 — turning glyphs** (principle 4): Hub recipient chip arrow, message
fold/unfold caret, tool-lane caret (already), Settings address-history arrow,
Select trigger chevron, Projects/TeamTemplates/Sessions disclosure carets,
AgentChip's `chevron` prop, the terminal chip-bar collapse chevron, the
server-switcher trigger.

**Wave 3 — things that enter** (`.appear*`): fresh feed blocks (gated on
`ts > openedAt`), attachment / recipient-extra / unread / needs-you chips,
the interrupt pill, `.m-acts` rows, filter pill, error and info lines
everywhere, reconnect and push banners, to-tail buttons (Hub + Terminal),
toasts (Terminal, Files copy/download), selection toolbar, search bar,
new-item / rename / commit rows, drop hint, system vitals first reading,
ConfirmDialog / picker / CreateProjectDialog / template sheets rising with a
scrim (the grammar already promised this).

**Wave 4 — state controls** (`.state-ctl`): tab bar, gear, split toggle,
Preferences segmented controls and address rows, shortcut recording, Board
status slider ring, `.acard` background + ring + opacity, `.to-chip`
variants, `.win-pill.cur`, `.pick.sel`, git tabs, Files tool buttons and
editor actions, Team header buttons, AgentGrid/SplitView active-cell ring,
state dots' background colour (never opacity).

**Wave 5 — reorders** (`animate:flip`, keyed lists first): Projects cards
(the most meaningful one: a project going up/down moves), roster cards,
Board same-column cards, sidebar project rows, Sessions rows and MRU chips,
terminal window chips, git status rows, Team roster chips, rail icons after a
drag, Settings history rows.

**Done 2026-09-03, shell / Settings / ui pass** (App, Preferences, Settings,
SystemStatus, ConfirmDialog, Select, SideHandle, InstallPrompt, Lightbox,
app.css atoms): tab bar, gear (turns 30° while Settings is open), split
toggle, reconnect banner, rail drop line and rail-slot flip after a drag,
server-switcher and Settings server-row swap glyphs, page slides stilled;
Preferences segmented/stepper/address/shortcut/hook controls and error
lines; Settings history arrow, list rise, row flip, error box, eye/share/
history buttons; the sysvitals first reading; ConfirmDialog scrim + sheet
rise (`sheet-up` joined the vocabulary) + desktop fade; Select chevron;
SideHandle opacity reveal; InstallPrompt on `--t-move`; Lightbox settle on
`moveMs()`; `.m-acts button` and `.chip-btn` filter transitions.

**Wave 6 — popovers grow from their anchor** (#86): ContextMenu, Select,
PanePicker, the Hub agent menu, the server menu and the hover card wear
`.pop-layer`; the composer's recipient and command menus and any remaining
`opacity: 0 → .ready` gate join them.

**Wave 7 — the highlight travels** (#86): phone tab bar (bar), desktop rail
(vertical bar), Preferences segmented controls and the Board status slider
(pill), Files/Git tabs ✓, the Hub drawer's view toggle. Done 2026-09-04 in
the terminal/Sessions/Files/Projects/Team pass: GitPanel's Status/Log tabs
and TeamTemplates' template list (both `.slide-pill`). Skipped on purpose:
the Terminal chip strip (scrolling AgentChips), the Sessions MRU chips and
the Team header toggles — none is a segmented control.

**Wave 8 — loads unfold** (#86): Hub room switch (skeleton bubbles + cards
after 150ms for an uncached room, then `.reveal-tail` on the feed and
`.reveal` on the roster), Board columns, Sessions list ✓, Files listing (the
dim-then-reveal) ✓, Projects cards ✓, Agents list, Settings categories on
first paint. Done 2026-09-04: Sessions (four `.skel` rows in a `.skel-wrap`,
then `.reveal` on the first fill), Files and DirPicker (rows keyed by path;
`.reveal` when a DIFFERENT directory lands, nothing on a same-dir refresh),
Projects (first fill), the Team roster strip (once per room, dropped after
the stagger so joiners still pop) and TeamTemplates' list on open.

**Wave 9 — hover explains** (#86): agent cards (state, model, context,
last activity, cwd), sidebar project rows (path, agents, last message),
rail icons and tab bar (name + shortcut), sessions and panes (command, size,
last activity) ✓, file rows (size, modified, kind) ✓, board cards (reporter,
assignee, updated), settings rows where the label is terse. Native `title`
is removed wherever the card takes over; `aria-label` stays. Done
2026-09-04: session and pane rows, Terminal window chips (on the `.win-chip`
wrapper) and its to-tail note, file rows / breadcrumbs / bookmarks / recent
files / git status rows (`files/git-status.ts` puts the porcelain code in
words), project cards (path, "n live · n stopped", state, age) and Team
roster chips (state, role, backend, model, room) — every card through the
row's EXISTING formatter. Open: AgentChip still falls back to its label as a
native `title`, so the Terminal chip shows both until `ui/AgentChip` learns
to go without one.

**Landed 2026-09-04, shell / Settings / ui pass** (App, Preferences,
SystemStatus, ui): wave 6 — the server menu wears `.pop-layer` (done in the
foundation commit, verified); wave 7 — the phone tab bar's bar, the rail's
vertical bar (hidden during an icon drag, re-measured on release — the
action now measures by client rects ÷ `uiZoom()` and re-reads after the move
tempo, `boxFromRects` pure + tested), and every Preferences segmented row
through the new `ui/Segmented` (the pill); wave 8 — the Settings category
list on first paint and the keyed `.pref-content` on a category switch;
wave 9 — rail icons (name + live shortcut), the server switcher (server,
address, state), the gear, the split toggle, Settings category rows (one
`settings*Hint` line each) and address rows (address + state), the vitals
corner (`sysDetail`: full CPU/memory/disk numbers + poll tempo). Not done
here, with reasons: the tab bar gets no hover card (touch-only, no hover);
address rows show no "last used" (no timestamp is stored for an address);
the vitals card has no load average / uptime (not on the `system_status`
wire); the server menu's rows do not `.reveal` (a popover already grows).
Settings has no `.git-tabs`-like tab row — its categories are `.side-row`s.

**Deliberately not done**: `body` and the scrollbar thumb keep their `0.3s`
theme cross-fade (app.css) — that is the THEME changing under every pixel at
once, a whole-screen event with no token of its own, and moving it to
`--t-move` would make a theme switch read as a flash; composer height
(JS-measured per keystroke); fold
bodies and tool-lane bodies via slide (principle 10); desktop sidebar /
rail content swaps (owner rule); PanePicker / Select / ContextMenu / agent
menu open (popover rule — the two that fade on `.ready` are tolerated as the
"invisible until measured" guard, not as a precedent); Team's `swapSides`
remount (a layout bug, not a motion gap — belongs to a Team fix); button
sizes (owner declined).

## Rules and their reasons

- **One vocabulary in app.css, components add a class** — five components had
  five private spinners, Hub had a private caret transition, and the compact
  drill pair was declared in five components; a second copy is a second tempo
  waiting to drift. A component references a global keyframe BY NAME (Svelte
  scopes only keyframes declared locally), so `ui/motion.source.test.ts`
  forbids a component-level `.chev`/`.flip` rule and any local declaration of
  `fade-in`/`rise-in`/`pop-in`/`sheet-up`/`drill-in-*`. `.to-tail` carries its
  own `pop-in` in app.css, so wearers add nothing.
- **No `svelte/transition`** — the codebase was CSS-only before this doc;
  outros conflict with principle 5 and with the feed's scroll ownership; a
  Svelte intro on a keyed list replays for every row on a parent recreate
  unless guarded. The one Svelte directive allowed is `animate:flip`, because
  CSS cannot animate a reorder, and its duration must come from `moveMs()`.
- **Intro keyframes animate transform/opacity only** — pinned by the test;
  the reasons are principles 3, 9 and 10.
- **Every loop stills under reduced motion** — the test walks every
  component for `infinite` and requires a `prefers-reduced-motion` rule in
  the same file.
- **The tokens have a JS mirror** — `T_FAST_MS`/`T_MOVE_MS` in `ui/motion.ts`
  are asserted equal to `--t-fast`/`--t-move` so a token change cannot leave
  `animate:flip` on the old tempo.
