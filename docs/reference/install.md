# Installing tmux-mobile

Two pieces run on the machine whose tmux you want to reach:

- **the gateway** (`tmm gateway`): the server, as a per-user background service. Phones, browsers and the desktop app all connect to it.
- **the desktop app** (macOS, optional): a window onto the same server.

## macOS from the DMG

1. Open the DMG and drag **TmuxMobile.app** into `/Applications` (or `~/Applications`). Run it from there, not from the mounted DMG: the gateway service records the path of the `tmm` inside the app, and a path on a DMG, or one macOS moved to a temporary location on first launch, disappears.
2. Put the bundled `tmm` on your PATH:

   ```bash
   ln -s /Applications/TmuxMobile.app/Contents/MacOS/tmm ~/.local/bin/tmm   # any directory on PATH
   ```

3. Start the gateway:

   ```bash
   tmm gateway
   ```

   On the first run it asks a few questions (port, listen address, tmux socket, optional TLS) and writes `~/.config/tmux-mobile/config.toml`. It then installs and starts the launchd agent `cc.voka.tmux-mobile`, which starts at login. `tmm gateway status` shows it and prints the connect hint; `tmm ui --open` opens the web UI the gateway serves.

4. Open the app. At start it checks for this machine's gateway (the same check `tmm gateway status` makes: a login with your token, answered with this machine's id):
   - the gateway answers → the app uses it and starts no server of its own;
   - nothing is listening → the app starts its own server, which stops when you quit the app;
   - something else holds the port → the app starts nothing and says why on the connect page.

   The server card on the left rail shows which, on its **This computer** line. Quitting the app never stops the gateway.

After moving the app, run `tmm gateway install --replace` with the new `tmm`: the installed service names the old path, so to the new `tmm` it is not its own and a plain `tmm gateway` refuses it.

## Linux

Build the server and `tmm` (`npm run build:server`: the web UI is embedded), put `src-tauri/target/release/tmm` on your PATH and run `tmm gateway`. It installs a systemd user unit, `tmux-mobile-gateway.service`. On a headless machine, `loginctl enable-linger <user>` lets it start at boot.

## Removing

```bash
tmm gateway uninstall      # stops and removes the service
```

The configuration stays in `~/.config/tmux-mobile/`. The design behind every step is in [tmm-cli.md § The gateway](../design-docs/features/tmm-cli.md#the-gateway-tmm-gateway-board-323).
