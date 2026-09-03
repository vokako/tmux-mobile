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
8. **Popovers do not animate; navigation has one slide; sheets slide with a
   scrim.** These three are already law in design-language.md §1 and stay so.
   A menu is placed, measured, shown. Page-level slides are touch-only.
   Desktop rails and sidebars have no motion behind them.
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
