# UI Unification — one sidebar, one vocabulary

## Context (the mess, inventoried 2026-08-01)

The desktop shell grew page by page and each page rolled its own left column:

| page | left column | width rule |
|---|---|---|
| Hub | projects list | `220px` fixed |
| Agents | definitions list | `minmax(280px, 0.9fr)` — proportional |
| Sessions | (the page IS a list) | full width |
| Files / Terminal | none | full bleed |

Two pages, two widths, two resize behaviors (none and none-but-different).
Switching rail tabs makes the left region *jump*. On top of that, visual
primitives are copy-pasted per component and have already drifted:
`.chip-btn`, `.ava`, `.side-h`, `.icon-btn` each exist twice (Hub,
AgentsPage) with subtle differences waiting to happen.

Owner direction: the rail's pages must share ONE sidebar region — same
geometry, manually adjustable — and the UI vocabulary must stop forking.

## Decisions

### 1. Sidebar geometry is owned by the shell, not the page

One CSS variable, set on `:root` by App and persisted:

```
--sidebar-w        default 240px, clamped 180–420px
localStorage key   tmux_sidebar_w
```

Every desktop page with a list + detail structure uses `var(--sidebar-w)`
for its left column: today that is **Hub** (projects) and **Agents**
(definitions). Pages that are genuinely full-bleed (Terminal, Sessions,
Files) do NOT get a fake sidebar — the rule is "wherever a sidebar exists,
it is THE sidebar", not "every page must have one".

Rationale for a var over a layout component: the pages keep their internal
grid (compact behavior, drawer columns, `:has(.editor)` collapse) and only
delegate the *width*. A shared layout component would have to absorb every
page's special cases; a var absorbs none and still guarantees the geometry
can never disagree.

### 2. One resize affordance

## One "New Project" surface

`CreateProjectDialog.svelte` (src/lib/projects/) is THE way a project is
born, wherever the button lives. The Chat sidebar and the Terminal sidebar
used to grow their own — the dialog vs an inline form with a SECOND directory
picker and raw kiro/claude/codex presets — which read as two different apps
(owner, 2026-08-19). The dialog survived because it carries the rules: the
required NAME that names project and session both, `DirPicker` over the same
`fs_list` the file browser uses, and registry agents (which seed real slots)
instead of bare backend names. It is self-contained — loads the registry,
runs create → up → spawn, and hands the created project back; the caller only
navigates (Chat selects the conversation, Terminal jumps into the first
pane). Sessions.svelte lost ~120 lines of duplicate form/picker with it.

`src/lib/files/DirPicker.svelte` is THE directory picker, for the same reason.
Team's "new team" workspace field had grown its own (`team/DirPicker.svelte`,
camelCase `onPick/onNavigate/onClose`, a breadcrumb path, no race guard, but a
new-folder affordance) beside the file browser's (`onpick/oncancel`, the
newest-tap-wins `seq` guard, list kept on screen while loading). Merged
2026-09-03 (review): one TypeScript component in `files/` with the guard AND
the new-folder input AND an `onnavigate` step callback, one lowercase prop
dialect; `CreateProjectDialog` and `Team` both use it.
`DirPicker.source.test.ts` pins that exactly one exists.

## Every sidebar speaks the same language

The Chat sidebar set the house style and the others follow it (owner,
2026-08-19: "所有的侧边栏风格尽量保持一致"): `--bg2` surface with a right
border, ONE uppercase mono section header per group (`.side-h`, `--fs-meta`,
`--text3`), borderless rows at 9px radius that only take a background when
they are active or hovered, and 8px gutters.

The outlier was the Terminal sidebar, because it renders `Sessions` →
`Projects`, which was designed as a PAGE of bordered cards. Those components
now take a mode flag instead of being duplicated: `Sessions chips={false}`
(also drops the MRU strip, see terminal.md) sets `.sidebar-mode`, which
tightens the content gutter and passes `dense` to `Projects`. In dense mode a
project card loses its border and surface, the group label becomes a plain
`.side-h`-style LABEL (no chevron: a sidebar section header is not a control,
and Chat's projects never collapsed), the row collapses to ONE line — dot,
name, age, with the path moved into the row's `title` — and its actions
(Close / archive) fade in on hover/focus-within. Windows stay under the row,
because picking one is why this sidebar exists, but as borderless text rather
than a tray of pills; that tray was the last thing still reading as a "card"
(owner had to point at it twice).

Measured after the change, Chat / Terminal / Agents sidebars all report the
same surface `rgb(238,238,240)` (light), 1px right border, 10.5px headers in
`--text3`, and 9px borderless rows.

`SideHandle.svelte` — a 6px grab strip on a panel's edge, and the ONE resize
affordance in the app:

- drag → live-updates a CSS variable on `:root`, persists on release
- double-click → reset to the default
- keyboard: arrow keys nudge ±16px when focused (a11y floor)
- hidden in compact/mobile layouts (they have no resizable columns)

It is PARAMETRIC, not sidebar-specific (2026-08-19): `varName`, `storeKey`,
`min`/`max`/`def`, `edge` (`right` default, `left` inverts the drag delta so a
panel grows when its left edge is dragged left) and `label`. Defaults describe
the shared sidebar (`--sidebar-w`, 180–420, 240), and the Hub's chat/terminal
divider passes `--hub-drawer-w`, 320–900, 520 — a second consumer instead of a
second implementation. The page that owns the variable restores it from
localStorage on mount; SideHandle is the only other writer.

The handle is the only writer of `--sidebar-w` besides the App init read.

### 3. Shared primitives move to `app.css`

The copy-pasted classes become global utilities, deleted from components:

| class | role |
|---|---|
| `.chip-btn` (+ `.primary`, `.danger`) | small bordered action button |
| `.icon-btn` | square icon-only button |
| `.side-h` | sidebar/section heading (mono, uppercase, tracked) |
| `.ava` | agent monogram chip (color set inline per backend) |

Svelte scoped styles keep page-specific *layout* (grids, spacing); shared
*vocabulary* lives in one file. New pages must reach for these before
inventing buttons.

### 4. Rhythm rules (documentation, not framework)

- Page headers: `padding: 10px 16px`, one `<h1>` at 15px mono for
  project-ish titles, borders with `var(--border)`.
- Section headings inside sidebars: `.side-h`.
- Interactive hover transitions: 160ms, color/border only.

These are conventions to follow when touching a page, not a retrofit pass —
retrofitting every page in one commit is churn without user value.

## Execution

1. `app.css`: add the utilities; App reads/persists `--sidebar-w`.
2. `SideHandle.svelte`; mount in Hub and Agents sidebars.
3. Hub `.cols` and Agents `.agents-root` use `var(--sidebar-w)`.
4. Components drop their local `.chip-btn`/`.icon-btn`/`.side-h`/`.ava`.
5. Verify: drag in Hub → switch to Agents → same width; reload → persists;
   compact/mobile unaffected.

## Page skeleton (added 2026-08-02, owner: "reuse the project page's format")

Bar dialects found in the wild: Hub's header (`padding 10px 16px`, ~41px,
`border` color, mono h1) vs the compact `--ui-bar-*` dialect shared by the
Files toolbar and the Terminal win-bar (31px, `surface` bg, `border2`) vs
Agents (no header bar at all, 16px sans h1). Three dialects, one app.

**The Hub page is the reference format.** Two more shared classes:

| class | role |
|---|---|
| `.page-head` | THE page header bar: flex, `min-height 42px`, `padding 6px 16px`, `border-bottom: 1px solid var(--border)`, transparent bg; `h1` inside is mono 15px/600 |
| `.side-row` | sidebar list row (the Hub `.p-row` pattern): flex, `8px 10px`, radius 9px, hover `surface2`, `.open` = `accent-bg`, `.add` = dim |

Adoption:

- **Hub** — donates the pattern; its local `.mid-head`/`.p-row` become the
  shared classes.
- **Agents** — full adopt: left column becomes a real sidebar (bg2,
  `.side-h`, `.side-row` entries), the editor gets a `.page-head` (name +
  actions). Compact keeps list-or-editor.
- **Files toolbar / Terminal win-bar** — adopt `.page-head` geometry and
  border/background (their dense controls stay; only the bar itself is
  standardized). The 31px `--ui-bar-*` tokens remain for the Preferences
  modal tabs, which are a modal, not a page.
- **Files directory browser** (added same day) — the desktop left column IS
  the shared sidebar: `var(--sidebar-w)` + bg2 + `SideHandle`, replacing the
  page-private flex-fraction splitter (`tmux_files_frac` retired). Mobile
  single-pane chain unchanged.
- **Terminal** (2026-08-19, owner: "terminal 和 project 还是不统一") — the last
  holdout, and it differed in four measurable ways at once:
  - *No title.* The win-bar had copied the head's geometry but carried no
    `<h1>`, and worse, the bar only existed when the window switcher was
    EXPANDED — which is not the default (`tmux_winswitcher` unset), so a fresh
    desktop install showed a terminal with no header at all (measured: no
    `.win-bar` in the DOM, xterm at `top: 0`). Now the bar wears
    `class="win-bar page-head"` (geometry, border and the mono 15px h1 come
    from the shared class, not a copy), and collapsing hides the window CHIPS
    while the head stays — the current-window chip compresses to the bar's
    right end, which is what "collapsed" always claimed to mean. The identity
    slot is a three-way branch, because the bar plays three roles: a split
    CELL keeps its chip (it opens the cross-session pane picker), a PHONE keeps
    its chip (it opens the session sheet, that layout's sidebar), and the
    DESKTOP shows the title — there the sidebar is already on screen, so a chip
    would be the second door the owner cut. `flex-wrap: nowrap` is required:
    the phone `.page-head` rule wraps head actions, and this head is a scroll
    strip.
  - *A different New Project button.* A full-width bordered `.new-btn` in a
    bottom bar, next to Chat's and Agents' `.side-row.add` rows — the same
    dialog behind two unrelated-looking controls. The sidebar now renders the
    shared row (measured identical to Chat's: 223×35, 12.5px, radius 9, padding
    8px 10px, same ink), the bottom bar keeps only the two list utilities
    (search + refresh, right-aligned), and the `.new-btn` survives solely in
    the page dialect. The row sits in `Sessions`, not in `Projects`, because
    that section hides itself when there is nothing to list — an empty project
    list is exactly when the create row matters most.
  - *A different column width.* A hardcoded 280px with no resize handle. Now
    `var(--sidebar-w)` + `SideHandle`, so switching rail tabs no longer makes
    the left region jump. The 280px exception was justified by project CARDS
    with a wrapping tray of pane pills; dense mode retired both, so the
    exception expired with them.
  - *Two header dialects in ONE column.* The dense PROJECTS label imitated
    `.side-h` while TEAMS/SESSIONS stayed accent-bold — and the imitation had
    already drifted (1.05px tracking vs Chat's 1.4px). Both now WEAR the
    `.side-h` class; the page dialect is qualified (`.projects:not(.dense)`,
    `.sessions:not(.sidebar-mode)`) because a scoped `.group-label` rule
    outranks a shared class (0,2,0 vs 0,1,0) and silently overrode it. A flat
    session list also gained its own header, so no group of rows is unlabelled.

  Measured after the change, Chat / Terminal / Agents / Settings report the
  same head (`min-height 42px`, `padding 6px 16px`, one border colour), the
  same h1 (15px, `ui-monospace`, 600), the same sidebar surface
  (`rgb(238,238,240)`, 1px right border, 240px) and the same section headers
  (10.5px mono, uppercase, 1.4px tracking, `--text3`). Restating a shared
  property in a page's own stylesheet is how each of these drifted; wearing
  the class is the fix.

## Settings as a page (added 2026-08-02, owner: "not a floating window")

The centered modal lasted one day: settings deserve the same skeleton as
every other page. `Preferences` becomes the `prefs` page — shared sidebar
with category rows, main column with a `.page-head` per category. The
categories are recut by SUBJECT instead of by history:

| category | contents |
|---|---|
| Appearance | theme, language, layout mode, UI zoom |
| Terminal | font family, font size, line spacing (previously mixed into Appearance) |
| Shortcuts | desktop-Tauri only (unchanged) |
| Connection | server info, addresses, hooks, share, disconnect, debug toggle |

The gear (rail bottom / mobile tab / disconnected top bar) navigates to the
page; no backdrop, no X. Compact drills category-list → detail like the
Agents page.

## Skills & MCP as first-class assets (same day)

The registry stored skills/MCP inline per agent — editing a shared MCP
server meant editing every agent. Now `reg_skills` (name → ref +
description) and `reg_mcp` (name → def) are central tables (state.db v6)
managed on the Agents page (sidebar sections AGENTS / SKILLS / MCP with the
same editors), exposed over `skills_*`/`mcp_*` RPCs and `tmm skills|mcp
list/save/delete` (self-management parity). Agent defs may reference both
by NAME: spawn resolves a skills entry matching a reg_skills name to its
ref, and an mcp array STRING entry to its reg_mcp def — inline objects keep
working, so nothing migrates.

## Out of scope (recorded, not forgotten)

- Sessions/Files as sidebar+detail pages (a deeper IA change; Sessions may
  eventually fold into the Hub sidebar entirely).
- A theme-token audit beyond the four utilities.
