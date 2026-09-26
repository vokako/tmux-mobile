# Settings Page

## Purpose
Configure server connection, appearance, language, and app behavior.

The gear button opens Settings as a full-window workspace below the app nav,
not a small popover. The workspace uses the shared left-aligned 860px
configuration canvas and one header, not centered cards. Preference rows,
fields and commands follow the configuration control contract in
[design-language.md](../../design-docs/features/design-language.md): 32px
pointer / 44px touch controls, labelled values and shared spacing. The category list
reads as TWO labelled groups where both exist (owner, 2026-09-05: app-level
settings, then Agent-level settings; a lone group hides its label — desktop,
where Agents is a page of its own, reads as one plain list):

**App** — about the application itself; Connection ends the group as its way out
- **Appearance** — how the app looks: theme, language, responsive layout, interface scale, the three font roles (content, display, terminal family), then the terminal's size and line spacing (owner, 2026-09-24: the terminal's style settings belong to Appearance; the one-row Terminal category is retired)
- **Chat** — how much of the conversation the feed shows: chat detail and tool rows (moved out of Appearance 2026-09-25: they set what is said, not how it looks)
- **Notifications** — message notifications On/Off, the level (Finished / Replies / Everything) and a test row (its own category, owner 2026-09-02)
- **Shortcuts** — configurable desktop navigation and Terminal window bindings (desktop only)
- **Connection** — current server/addresses, optimize/share/disconnect (the global debug switch and its floating log panel were retired 2026-09-26)

**Agent** (phone only, where Agents is not a page of its own)
- **Agents / Teams / Skills / MCP servers** — four second-level pages, each the real AgentsPage narrowed to one section (owner, 2026-09-02)

## Components
- Connection setup command opens the existing address/token form.
- Current server addresses allow quick switching; tokens are never displayed.
- Multi-server registry (board #55): named server configs persist in
  `tmux_servers` (id, name, address, token, optional socket + machineId);
  identity is the MACHINE, so LAN/Tailscale/WAN alternates of one server are
  ONE entry (the failover map `tmux_machines` stays the address authority). A
  successful connect upserts by machineId; the desktop rail carries a
  switcher above the configure group, and on the phone a row at the top of
  the Settings category list (swap icon + authenticated hostname, falling back
  to the host portion of the address before auth) opens the same switcher —
  current entry marked, click switches
  (full socket teardown + reload through the one boot path, per-server
  `tmux_state`/`tmux_machine_id` parked and restored so restore targets never
  cross servers), double-click renames, non-current rows removable, `+` row
  opens this page as the add flow. Migration from the single-server keys is
  one-time, idempotent, and never loses the current user.
- Server info: hostname, machine ID, address; the address list marks the
  current address and shows a connecting cue on a tapped address until the
  switch settles. An alternate address is removable (the active one is not —
  it is the live connection), and a row's grip drag (or Arrow keys on it)
  reorders the list; the order IS the failover priority the reconnect
  round-robin walks (board #222)
- Language selector: EN / 中文 (`ui/Segmented`, the travelling pill)
- Theme selector: Auto / Light / Dark (`ui/Segmented`)
- Message notifications (own category): `ui/Switch`, persisted immediately to localStorage `tmux_notify`; turning it on is the user gesture that previews the cue and requests system-notification permission (Android's runtime prompt inside the app). The caption says when only sound can play. The separate test command plays the cue and attempts a notification; it cannot race an outstanding permission request. Moved here from the Hub header (board #72).
- Desktop interface scale (60%–180%, persisted to localStorage `tmux_ui_zoom`; Cmd/Ctrl `+`, `-`, and `0` use the same value)
- Terminal font size control (`ui/Stepper`, 6-40px), independent from interface scale; tool-row count and interface scale use the same Stepper with their own bounds and localized action names
- Terminal font family (in Appearance with the other two font roles, followed by the terminal's size and line spacing; the list offers only families the DEVICE resolves — the suggestion pool is probed with the same registry check the validator uses, so nothing offered can fail on pick; another family may still be typed; only a valid local font is applied and persisted to localStorage `tmux_font`; empty = system default)
- Terminal line spacing (0.40–1.60, persisted to localStorage `tmux_line_height`; applies live to every normal, split, and Team terminal)
- Line spacing uses `ui/Slider`, with its native range semantics, visible numeric value and named reset command
- Desktop shortcuts default to Cmd+U / Cmd+I for previous/next page, Option+U / Option+I for previous/next Terminal window, Cmd+T for Terminal, and Cmd+F for Files
- Shortcut bindings can be recorded, cleared with Delete/Backspace, reset to defaults, or disabled; duplicate bindings are rejected
- Disconnect button

## Interactions
- Enter address + token → connect
- Tap history entry → auto-fill and connect
- Switch language → immediate UI text update, persisted to localStorage
- Switch theme → immediate CSS variable transition
- Switch settings tab → remember the last tab in localStorage and restore it on the next open
- Rest a pointer on a category row (desktop) → the shared hover card shows the category's name and a one-line description of what is inside (`settings*Hint`); on an address row → the address and its state (current / connecting / alternate). Touch shows no card; the rows' `aria-label`s and visible text are unchanged
- Adjust interface scale → updates the complete Tauri desktop WebView; terminal grid refits after the native zoom settles
- Adjust terminal font size → updates Terminal view without changing the surrounding UI
- Choose or enter a font → validate it against the device font registry, then apply and remember it; the row is pending during validation, and failure restores the confirmed family with a visible error
- Tap an alternate address' × → it leaves the machine's failover set; drag a row's grip (or focus it and press Arrow keys) → the set's order changes; both persist to localStorage `tmux_machines` through the one writer (`servers.ts` `saveMachineAddresses`)
- Tap disconnect → `doDisconnect()`

## API Calls
- `auth(token)` or encrypted auth (client_nonce + proof) — authenticate on connect (returns machine_id, hostname)
- `set_socket(socket)` — set tmux socket path at runtime

## State Management
- Connection state (disconnected/connecting/connected)
- Address history (localStorage)
- Locale / language preference (localStorage `tmux_locale`)
- Interface scale, terminal font size, theme, terminal font name preference (localStorage)
- Server info (hostname, machine_id)
- State restore on reload (page, session, view mode)

## Configuration Contract (#156, 2026-09-10)

- Independently valid local preferences remain immediate; there is no page-wide
  Save or Cancel. Async commands use shared CommandButton pending/error states,
  synchronously prevent duplicate activation and permit retry after failure.
  Hook results belong to the captured connection generation; a late response
  cannot overwrite a newer server's status. Failed updates retain known status.
- Preferences keeps one embedded AgentsPage for the four Agent categories.
  A category change, including a capability correction or external request,
  passes its action through the child's `onGuardExit` registration before
  changing the visible category or unmounting that child. The child owns
  pending-save queuing and dirty-draft confirmation, not a second Settings queue.
- An external `{tab, n}` request is consumed only when its category action
  applies. Its Agent edit request is forwarded only after that accepted jump,
  and only to the Agents section. Superseded callbacks cannot redirect a newer
  category intent. Back still delegates to the child's dialog/editor chain
  before returning to categories; no new global history handler is installed.
- Settings observes its actual container and requested sidebar width. Below
  the sidebar plus 480px editor budget, it presents the existing category drill
  layout without navigating or discarding state. The 760px compact form remains;
  control sizes follow input capability. The child's own layout decides how its
  object list and editor fit.
- The shortcut recorder remains a native keyboard-input target, now using shared
  field geometry. Capture, conflict rejection, Escape, clear and reset keep their
  existing meanings; ordinary commands no longer share its private button skin.

Mounted tests execute the Settings handlers; the request-arbitration cases use
a child stub only for that boundary. `preferences-agent.mount.test.ts` also
mounts the real Preferences and AgentsPage together, proving a single discard
confirmation and a host exit held until Save completes. Global-tab preservation
is tested at the retained-component boundary, not through a complete App mount.
Chromium checks geometry and interaction in isolated real-component fixtures;
native WebView/IME behavior and the owner's actual build remain separate gates.

## Edge Cases
- Auto ws/wss detection based on address format
- Multi-address reconnect: server machine_id tracks alternate addresses, auto-failover on disconnect
- Tauri desktop auto-fills config from local `~/.config/tmux-mobile/config.toml`
- Language auto-detected from `navigator.language` on first visit (zh → Chinese, else English)

## Motion
State changes on this page move on the app's one vocabulary
([design-docs/features/motion.md](../../design-docs/features/motion.md)): the
address-history arrow TURNS 180° (`.flip`) instead of swapping glyphs, the
history list rises in and its rows `animate:flip` when an entry is removed,
every error line fades in (`.appear`, opacity only — never height), address
rows / the shortcut recorder cross-fade their selected clothes on `--t-fast`,
every segmented row is `ui/Segmented` whose accent pill GLIDES to the chosen
option on `--t-move` (the buttons only cross-fade their ink), and the server
row's 180°-symmetric swap glyph uses `.quarter-turn` to rotate 90° while its
popover is open (a 180° turn would look unchanged). The server popover grows
from its anchor (`.pop-layer`). The category list unfolds on first paint and a
switched category's pane is keyed on the accepted category so its sections rise in
staggered (`.reveal`) rather than swapping in as a finished wall — on the
phone the unfold plays inside the drill slide only on the way in; a re-shown
list does not replay it. On a pointer device the category and address rows
open the shared hover card (name + description / address + state).
