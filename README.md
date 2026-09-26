<p align="center">
  <img src="assets/icon.svg" width="128" alt="tmuxmobile"><br>
  <img src="assets/logo.svg" width="220" alt="tmuxmobile">
</p>

<p align="center">
  <strong>Your coding agents, running for real in tmux — watched, directed and handed back from your phone or your desk.</strong><br>
  <sub>A shell around tmux and the agent CLIs you already use. Nothing is wrapped, nothing is re-implemented, and you can always walk back to the terminal and take over.</sub>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri_2-Rust-orange?style=flat-square" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Svelte_5-Frontend-ff3e00?style=flat-square" alt="Svelte 5">
  <img src="https://img.shields.io/badge/xterm.js_6-Terminal-00d4ff?style=flat-square" alt="xterm.js 6">
</p>

---

## What this is

You run Kiro CLI, Claude Code, Codex, Grok, OMP or Kimi Code inside tmux on
your Mac or Linux box. tmux-mobile is the **shell** around that: it connects those real
sessions to a phone or a desktop window, gives every project a chat room where
you and your agents talk, and lets agents spawn, brief and review one another.

It is deliberately **not an engine**. tmux keeps owning the processes and
sessions. Each agent CLI keeps owning its own intelligence and its own
harness: the TUI, the approval flow, the tool loop, session resume. MCP servers
and skills keep owning the tools. tmux-mobile adds exactly four things and
nothing else:

| It adds | Meaning |
|---|---|
| **A connection** | WebSocket JSON-RPC with token auth and optional end-to-end encryption; several saved servers |
| **A room** | One chat room per project, a task board, and agent status *derived* from what the agents actually do |
| **An identity** | A registry of agent definitions and teams, each spawned into its own isolated home |
| **A window** | A phone-first *and* desktop-first UI: terminal, files, sessions, hub |

## Why it is built this way

**The real CLI, in a real pane.** Every agent is an ordinary CLI process in an
ordinary tmux window. tmux-mobile never puts a protocol wrapper around it and
never changes how it behaves; it only reaches the CLI through the doors the CLI
documents — environment, config files, hooks, launch flags. Walk back to your
desk and the window looks exactly as if you had started it by hand. Keep
typing, press Escape, scroll the history. Nothing is hidden from you.

**No central node.** Sessions live in tmux, not in a server. If the
tmux-mobile server is stopped, restarted or was never installed on a machine,
every agent keeps working and every human keeps working. The server is an
observer and a messenger, never the runtime.

**One bridge, three primitives.** Humans and agents talk through the same
three things: text typed into a pane, hooks observed from outside, and the
`tmm` command an agent runs itself. There is no second protocol for
agent-to-agent traffic, so every conversation is visible in the room and a
person can step into any side of it at any time. Humans and agents read the
same record.

**Agents can manage agents.** Everything the UI can do — read the room, see
who is working and who is waiting, spawn a teammate with a brief, move a board
issue, start or stop a project — exists as a `tmm` command. A lead agent can
set up a project, define a new kind of agent, hire it and review its work
without a human in the loop.

**Workspaces you can close.** A project is a declaration: a directory, a
session name, the windows and agents it should hold. The running tmux session
is a disposable projection of it. Close a workspace and reopen it a week later,
or after a reboot, and it comes back the way you declared it.

**Two screens, one standard.** The phone and the desktop each get a complete
form of the same design language: one type scale, one colour vocabulary where
*at rest is grey* and *in motion glows*, one motion grammar that tells you
where you are going. Beauty here is a function: you should see at a glance who
is running and who needs you.

The full tenets, with the reasoning and the incidents behind each one, are in
[docs/tenet.md](docs/tenet.md).

## What you can do

- **Hub** — one room per project. Talk to an agent with `@name`, to a whole
  team by its name, or to everyone with `@all`. The live agents are a tab
  strip above the input, teams grouped like browser tab groups; double-click
  one to read only its messages. Spawn agents from the registry (Kiro / Claude
  Code / Codex / Grok / OMP / Kimi) or a whole configured team; watch their
  derived status live; open any agent's terminal, the project's files or the
  task board in a drawer without leaving the conversation.
- **Task board** — todo / doing / review / done per project; assigning an issue
  notifies the agent, moving it to review notifies the reporter; agents work
  the board with `tmm board`.
- **Agents & teams** — define agents (backend, model, effort, persona, skills,
  MCP servers) and teams (members, roles, nested sub-teams) once; every spawn
  gets an isolated home so agents never share or leak configuration. An
  app-wide instructions file leads every agent's prompt.
- **Terminal** — the same tmux pane you have at your desk, with touch scrolling
  and momentum, shortcut keys and a one-shot Ctrl modifier, a keyboard that
  overlays agent TUIs instead of resizing them, a window switcher with agent
  icons, and a back-to-tail control that also shows new output.
- **Files** — browse, preview (Markdown, code, images, PDF, CSV, HTML, PPTX)
  and edit, with a full-screen reading mode on the phone; path references in
  chat or in a preview open in place with a working Back; stage, diff, commit
  and push with the built-in git panel.
- **Sessions & projects** — every tmux session is a project. Bring a project
  up, take it down, archive it; rename it by name, never by folder. Open
  projects list first, newest activity on top.
- **Settings** — Appearance (light / dark / auto, English / 中文, layout,
  interface scale on desktop, the content, interface and terminal fonts,
  terminal size and line spacing), Chat detail, notifications, desktop
  shortcuts, and several named servers with LAN, Tailscale and WAN addresses
  that fail over.
- **Notifications & vitals** — an away client can play a cue or post a system
  notification when an agent needs you; low-frequency CPU, memory and disk
  readings sit in the sidebar.
- **`tmm`** — the agent's hands: `tmm send`, `tmm log --grep`, `tmm board`,
  `tmm spawn`, `tmm project up|down`, `tmm registry`, `tmm teams`, background
  `tmm task`s that survive the server, and more. Fail-soft in ~20 ms when the
  server is away; an agent never blocks on it.

The server runs on your Mac or Linux machine. The UI runs in any browser, as a
PWA, or as a native app on macOS and Android.

## Quick start

```bash
npm install

# Browser development stack (recommended): one public endpoint on :5173.
# Vite serves the UI and proxies /ws + /dl to the loopback-only Rust server;
# Rust changes rebuild and restart automatically.
npm run dev:all

# Desktop app (Tauri window + server, connection auto-filled)
npm run tauri:dev

# Server + tmm only, no webview needed (Linux hosts, CI, containers)
npm run build:server
```

On first launch a token is generated and saved to
`~/.config/tmux-mobile/config.toml`. With `npm run dev:all`, open
`http://<your-machine-ip>:5173`; the connection field is pre-filled with
`ws://<your-machine-ip>:5173/ws`, so only port 5173 needs to be reachable.

Point your agents at the shell: `tmm` is built next to the server binary and is
on every managed agent's PATH. Spawn one from the Hub, or from any shell:

```bash
tmm project create ~/work/my-app --name "My app"
tmm spawn claude --brief "Read docs/tenet.md, then fix the failing test in src/lib/core."
tmm log -f
```

## Configuration

`~/.config/tmux-mobile/config.toml`:

```toml
token = "auto-generated-uuid"
host = "0.0.0.0"    # optional
port = 9899          # optional
tmux_socket = ""     # optional, -S path
tls_cert = ""        # optional, PEM cert for wss://
tls_key = ""         # optional, PEM private key for wss://
disconnect_grace_secs = 600  # optional; how long to keep the tmux window size
                             # after the last client drops (0 = restore at once)
```

Environment variables override the file: `TOKEN`, `HOST`, `PORT`, `TMUX_SOCKET`,
`TLS_CERT`, `TLS_KEY`, `DISCONNECT_GRACE_SECS`. Every key is documented in
[docs/reference/config.md](docs/reference/config.md).

## Commands

| Script | What it does |
|--------|--------------|
| `npm run dev:all` | Vite on :5173 + watched Rust server (browser dev loop) |
| `npm run tauri:dev` | Desktop app + server |
| `npm run build:server` | `server` + `tmm`, release, no webview |
| `npm run build:mac` | macOS `.app` + `.dmg` |
| `npm run build:android` | Android APK (aarch64) |
| `npm test` | Frontend + script tests (`node --test`) |
| `npm run check` | `svelte-check`, the type check |
| `npm run test:rust` | Rust tests, sequential, needs a running tmux |

## Build targets

**macOS** — `npm run build:mac` → `src-tauri/target/release/bundle/dmg/`.

**Android** — `rustup target add aarch64-linux-android && npm run build:android`.
Needs Android SDK, NDK 28+, Java 17+. Release signing reads
`src-tauri/gen/android/key.properties` (gitignored; copy the `.example`);
without it the build succeeds unsigned.

**Linux server only** — `npm run build:server` needs no WebKitGTK; it is all
the browser UI and the phone talk to.

**iOS** — not implemented; tracked in [docs/todo.md](docs/todo.md).

## Tailscale

```bash
tailscale serve --bg 5173
# UI:        https://your-machine.tailnet-name.ts.net/
# WebSocket: wss://your-machine.tailnet-name.ts.net/ws
```

Port 9899 stays loopback-only; it needs no serve rule.

## Prerequisites

- macOS or Linux with tmux
- Rust toolchain + Node.js (≥ 22.22)
- A WebKit webview for the desktop app (macOS has one; Linux needs
  `webkit2gtk-4.1`) — or skip it with the server-only build
- Recommended: `set-option -g history-limit 50000` in your tmux config

## Documentation

- [Tenets](docs/tenet.md) — why the project exists and what every decision answers to
- [Guidance](docs/guidance/) — per-dimension rules and review checklists for contributors and agents
- [Requirements, design docs, conventions, reference](docs/) — the map is in [CLAUDE.md](CLAUDE.md)
- [WebSocket RPC API](docs/requirements/api-contracts/websocket-rpc.md)

## License

MIT
