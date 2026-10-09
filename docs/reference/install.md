# Installing tmux-mobile

Two pieces run on the machine whose tmux you want to reach:

- **the gateway** (`tmm gateway`): the server, as a per-user background service. Phones, browsers and the desktop app all connect to it.
- **the desktop app** (macOS, optional): a window onto the same server.

## macOS from the DMG

You need tmux (`brew install tmux`). The app is not signed or notarized yet: the first time, macOS refuses to open it. Right-click it in Finder → **Open** → **Open**, or run `xattr -dr com.apple.quarantine /Applications/TmuxMobile.app`.

Install the gateway first, then open the app. The other way round, the app starts its own server on the gateway's port, and `tmm gateway` refuses to start next to it.

1. Open the DMG and drag **TmuxMobile.app** into `/Applications` (or `~/Applications`). Use it from there: the gateway service records the path of the `tmm` inside the app, and a path on the mounted DMG, or a temporary copy macOS made on a first launch from the DMG, disappears.
2. Make the bundled `tmm` reachable:

   ```bash
   mkdir -p ~/.local/bin
   ln -s /Applications/TmuxMobile.app/Contents/MacOS/tmm ~/.local/bin/tmm   # ~/.local/bin must be on your PATH
   ```

3. Start the gateway:

   ```bash
   tmm gateway
   ```

   With no `~/.config/tmux-mobile/config.toml` yet it asks a few questions (port, listen address, tmux socket, optional TLS) and writes it; `tmm setup` runs the same questions again later. It then installs and starts the launchd agent `cc.voka.tmux-mobile`, which starts at login. `tmm gateway status` shows it and prints the connect hint; `tmm ui --open` opens the web UI the gateway serves.

4. Open the app. At start it checks for this machine's gateway (the same check `tmm gateway status` makes: a login with your token, answered with this machine's id):
   - the gateway answers → the app uses it and starts no server of its own;
   - nothing is listening → the app starts its own server, which stops when you quit the app;
   - something else holds the port → the app starts nothing and says why on the connect page.

   The server card on the left rail shows which, on its **This computer** line. Quitting the app never stops the gateway.

**If you opened the app first**, it is running its own server on that port. Quit the app (⌘Q; its server stops with it), then run `tmm gateway`, then open the app again. Its first launch already wrote `config.toml` with a token, so `tmm gateway` asks nothing; run `tmm setup` first if you want to change the port or listen address.

**What was tested.** The reuse and the embedded fallback were checked on an Apple-silicon Mac (macOS 27) by launching the app binary with an explicit environment. A double-click from Finder starts the app with launchd's PATH, not your shell's; the app itself needs none, but tmux has to be reachable for the server, and the gateway service sets its own PATH. Starting from Finder has not been verified end to end.

After moving the app, run `tmm gateway install --replace` with the new `tmm`: the installed service names the old path, so to the new `tmm` it is not its own and a plain `tmm gateway` refuses it.

## Linux

Build the server and `tmm` (`npm run build:server`: the web UI is embedded), put `src-tauri/target/release/tmm` on your PATH and run `tmm gateway`. It installs a systemd user unit, `tmux-mobile-gateway.service`. On a headless machine, `loginctl enable-linger <user>` lets it start at boot.

## Removing

```bash
tmm gateway uninstall      # stops and removes the service
```

The configuration stays in `~/.config/tmux-mobile/`. The design behind every step is in [tmm-cli.md § The gateway](../design-docs/features/tmm-cli.md#the-gateway-tmm-gateway-board-323).
