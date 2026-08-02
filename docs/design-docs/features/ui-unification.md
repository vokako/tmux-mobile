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

`SideHandle.svelte` — a 5px grab strip on the sidebar's right edge:

- drag → live-updates `--sidebar-w`, persists on release
- double-click → reset to the 240px default
- keyboard: arrow keys nudge ±16px when focused (a11y floor)
- hidden in compact/mobile layouts (they have no sidebar)

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

## Out of scope (recorded, not forgotten)

- Sessions/Files as sidebar+detail pages (a deeper IA change; Sessions may
  eventually fold into the Hub sidebar entirely).
- A theme-token audit beyond the four utilities.
