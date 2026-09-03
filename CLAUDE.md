# AGENTS.md — tmux-mobile

This file is the MAP, not the manual. It says what the project is, the dozen
rules every change must respect, and where everything else is written down.
A rule belongs next to the design it protects (`docs/design-docs/…`), never
here: when you verify a change, update that design doc and commit together.
(`AGENTS.md` is a symlink to this file — they are one document.)

## What this is

Tauri 2 cross-platform app (Rust + Svelte 5) for monitoring and controlling
tmux sessions from a phone, and — through the **Hub** — for running and
talking to AI coding agents (kiro, codex, claude, grok) inside those sessions.
WebSocket JSON-RPC with token auth + optional E2E encryption. Targets: Android
(primary), macOS desktop, browser/PWA.

- **Frontend**: Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`), Vite 6, TypeScript (migration in progress), xterm.js v6
- **Backend**: Rust (Tauri 2), tokio, tokio-tungstenite, rusqlite (`state.db`), vendored `agora` bus (`team.db`)
- **Preview / rendering**: highlight.js, marked (+ KaTeX in chat), mermaid, pdfjs-dist
- **Agent side**: the `tmm` CLI (`src-tauri/src/bin/tmm.rs`) + hooks telemetry; no MCP for the integration itself
- **Layout**: `src/lib/{app,core,files,hub,projects,sessions,system,team,terminal,ui}` · `src-tauri/src/{server,projects,team,bin}` (backend map: [docs/reference/backend-map.md](docs/reference/backend-map.md))

## Commands

```bash
npm run dev:all          # Vite :5173 + watched Rust server (browser dev loop)
npm run tauri:dev        # desktop app + server
npm run build:server     # server + tmm, no webview needed
npm run build:android    # APK (aarch64) — ends with the build-dir postflight
npm test                 # frontend + script tests (node --test, no tmux)
npm run check            # svelte-check (the ONLY type check)
npm run test:rust        # Rust tests, sequential, needs a running tmux
```

Everything else about running and building — the supervised server on this
host (**do not restart it**), the `gui` Cargo feature and headless build,
`incremental = false`, port preflight, the Android build-dir postflight,
`pnpx` — is in [docs/conventions/development.md](docs/conventions/development.md).

## Non-negotiables

Each links to the doc that holds the reason and the details.

1. **The design language is a contract** — six type steps, three font roles, the radius scale, two hover families, ONE popover mechanism, `--t-fast/--t-move`, 760px compact, 44px touch. A new visual species is a regression; source tests enforce it. → [design-language.md](docs/design-docs/features/design-language.md)
2. **TypeScript rules**: explicit `.ts` in relative imports, erasable syntax only, `npm run check` is the type check, convert `.js` file-by-file with no logic change in the same commit. → [conventions/frontend.md](docs/conventions/frontend.md)
3. **Platform checks**: `isAndroid` before `isTauri`; `await tauriReady` before any plugin; Android opens files through `AndroidFileOpener`, never `tauri-plugin-opener`. → [conventions/frontend.md](docs/conventions/frontend.md)
4. **Commit after every verified change** — tested, docs updated, one logical change per commit. Never commit `agent-team-page/` or unrelated in-progress work. → [conventions/development.md](docs/conventions/development.md)
5. **Tests**: `npm test` for the frontend, `npm run test:rust` (sequential, shared tmux) for Rust; source-contract tests (`*.source.test.ts`) pin markup and CSS on purpose — read the test before "fixing" it. → [conventions/testing.md](docs/conventions/testing.md)
6. **One mechanism per UI job**: `ui/Select` for every dropdown, `ui/ContextMenu` + `ui/longpress` for right-click/long-press, `menuPlacement` for every fixed popover, `.to-tail` for every back-to-tail control, `.live-dot`/`stateDotColor` for the one status colour language (at rest is achromatic). → [design-language.md](docs/design-docs/features/design-language.md), [hub-feed.md](docs/design-docs/features/hub-feed.md)
7. **Terminal key encoding**: Ctrl keys go to tmux as NAMED keys (`extended-keys on` drops raw C0 bytes); device-attribute responses are filtered before forwarding; a hidden terminal records frames and never renders them. → [terminal-rendering.md](docs/design-docs/pages/terminal-rendering.md)
8. **The keyboard is an overlay for agent TUIs** (`.keep-rows`), a resize for everything else; printable keys bypass xterm's keydown so CJK IMEs work; only `unlockKeyboard()` and friends may touch `kbLocked`. → [terminal-keyboard.md](docs/design-docs/pages/terminal-keyboard.md)
9. **Projects declare, tmux projects**: every session is a project, `up` matches windows by name, only agent slots are relaunched, migrations run with `PRAGMA foreign_keys=OFF`. A project is named by its NAME, never its folder. → [projects.md](docs/design-docs/features/projects.md)
10. **A managed agent is defined by its isolated home** (`<ws>/.tmm/agents/<name>/`, `projects::managed_home`); its model and effort live in its CONFIG, never on the launch line; the launch recipe (env + PATH + identity) is replayed on restart. → [agents-overview.md](docs/design-docs/features/agents-overview.md)
11. **Hook-sourced posts obey four invariants** (same-turn dedup, record-only, managed-only, reply budget) and status is DERIVED from turn edges — pane activity is not work, Claude's `idle_prompt` is not an ask. → [agent-status.md](docs/design-docs/features/agent-status.md)
12. **Delivery is typing into a pane**: `@name` types into one agent, `@all` into every managed agent, no recipient records only; a `/command` goes verbatim to the CLI; `Escape` is the only interrupt. → [hub-composer.md](docs/design-docs/features/hub-composer.md)
13. **Chat markdown escapes `&` and `<`, never `>`**; images are references, never bytes; messages are not deletable in the UI. → [conventions/frontend.md](docs/conventions/frontend.md), [hub-feed.md](docs/design-docs/features/hub-feed.md)
14. **CLI/UI parity**: every project/agent/board verb exists as a `tmm` command, because an agent that can only be managed by a human cannot manage a teammate. → [tmm-cli.md](docs/design-docs/features/tmm-cli.md)
15. **Team is desktop-only and JSON-gated**: `server/` never names an agora type; mobile passes `None`. → [team.md](docs/design-docs/features/team.md)

## Documentation map

`docs/` is divided by the QUESTION a document answers:

| directory | answers | when to read |
|---|---|---|
| `docs/requirements/` | WHAT the product does (pages, features, API contracts, backend services) | before changing behaviour a user sees |
| `docs/design-docs/` | WHY it is built this way and HOW — `features/` cross-cutting, `pages/` per screen; each ends with **Rules and their reasons** | before touching that area |
| `docs/conventions/` | how we WORK: development loop, frontend rules, testing | first day, and whenever a build misbehaves |
| `docs/reference/` | FACTS to look up: configuration keys, the backend module map | when configuring or deploying |
| `docs/exec-plans/` | HISTORY: dated plans and prototypes that led here | to understand a past decision |
| `docs/unresolved.md` | known open problems | before filing a duplicate |

### Requirements (the WHAT)
- Pages: [Terminal](docs/requirements/pages/terminal.md) · [File Browser](docs/requirements/pages/file-browser.md) · [Sessions](docs/requirements/pages/sessions.md) · [Settings](docs/requirements/pages/settings.md) · [Hub (chat, agents, board)](docs/requirements/pages/hub.md) · [Team](docs/requirements/pages/team.md)
- Features: [i18n](docs/requirements/features/i18n.md) · [Message notifications](docs/requirements/features/notifications.md) · [System status](docs/requirements/features/system-status.md)
- Contracts: [WebSocket RPC API](docs/requirements/api-contracts/websocket-rpc.md) · [WebSocket server](docs/requirements/backend/services/websocket-server.md) · [tmux wrapper](docs/requirements/backend/services/tmux-wrapper.md) · [Filesystem service](docs/requirements/backend/services/filesystem.md)

### Design — shell, platform, visual language
- [**Design language** (normative)](docs/design-docs/features/design-language.md) · [Fonts](docs/design-docs/features/fonts.md) · [UI unification](docs/design-docs/features/ui-unification.md) · [App shell (rail, bottom bar, back gesture, restore)](docs/design-docs/features/app-shell.md) · [Desktop split-screen](docs/design-docs/features/split-screen.md) · [Sessions density](docs/design-docs/pages/sessions-density.md)
- [Android platform](docs/design-docs/features/android-platform.md) · [PWA install](docs/design-docs/features/pwa-install.md) · [File handling & security](docs/design-docs/features/file-handling.md) · [Message notifications](docs/design-docs/features/notifications.md) · [System status](docs/design-docs/features/system-status.md)
- [WebSocket client robustness + server registry](docs/design-docs/features/websocket-client.md) · [Concurrent WS RPC](docs/design-docs/features/concurrent-ws-rpc.md) · [Disconnect grace](docs/design-docs/features/disconnect-grace.md)

### Design — terminal
- [Touch handling](docs/design-docs/pages/terminal-touch.md) · [Gesture state machine](docs/design-docs/pages/terminal-gestures.md) · [Keyboard](docs/design-docs/pages/terminal-keyboard.md) · [Rendering, tail, key encoding](docs/design-docs/pages/terminal-rendering.md) · [Sizing (cols × rows)](docs/design-docs/pages/terminal-sizing.md) · [Cursor layout](docs/design-docs/pages/terminal-cursor-layout.md) · [Color adaptation](docs/design-docs/features/color-adaptation.md) · [xterm build-time patch](docs/design-docs/features/xterm-patch.md)

### Design — projects, agents, hub
- [Projects (declarative workspaces)](docs/design-docs/features/projects.md) · [Agents overview (CLI substrate, hooks, isolated homes, registry)](docs/design-docs/features/agents-overview.md) · [Agent status (derived state, deliveries, vitals, recovery)](docs/design-docs/features/agent-status.md) · [Agent lifecycle](docs/design-docs/features/agent-lifecycle.md) ([中文](docs/design-docs/features/agent-lifecycle.zh.md)) · [Agent notifications (hooks)](docs/design-docs/features/agent-notifications.md)
- [tmm CLI (the agent's hands — exhaustive)](docs/design-docs/features/tmm-cli.md) · [Agent teams (§ in agents overview)](docs/design-docs/features/agents-overview.md#agent-teams-board-74) · [Hub feed](docs/design-docs/features/hub-feed.md) · [Hub composer](docs/design-docs/features/hub-composer.md) · [Task board](docs/design-docs/features/board.md) · [Team (multi-agent bus)](docs/design-docs/features/team.md)

### Conventions, reference, history
- [Development (commands, dev loop, build gotchas)](docs/conventions/development.md) · [Frontend conventions](docs/conventions/frontend.md) · [Testing](docs/conventions/testing.md)
- [Configuration reference](docs/reference/config.md) · [Backend module map](docs/reference/backend-map.md)
- [Execution plans (history)](docs/exec-plans/) · [Unresolved issues](docs/unresolved.md)

## One entry point

This is the ONLY memory file in the repository (owner, 2026-09-02: source
folders carry no `CLAUDE.md`/`AGENTS.md` — a second entry point is a second
place to look). Everything else is under `docs/`. `temp/` is gitignored
scratch and `.tmm/agents/*/` are generated agent homes; files inside them are
not project memory.
