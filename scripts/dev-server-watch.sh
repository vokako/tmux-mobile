#!/usr/bin/env bash
# Watch-and-supervise loop for the headless WS server.
#
# Why this exists instead of `tauri dev --release`: this host has no
# webkit2gtk-4.1 and no DISPLAY (verified 2026-08-18), so the Tauri shell
# cannot even build here — the headless `--no-default-features` path is the
# supported one (see AGENTS.md "Headless build"). What `tauri dev` would have
# provided for the server half is exactly what this loop does:
#
#   1. build the release server from the current tree,
#   2. run it,
#   3. restart it when it EXITS (crash or clean),
#   4. rebuild + restart when Rust SOURCES CHANGE (mtime poll — no inotify
#      dependency, and NFS-safe),
#   5. if a rebuild fails, keep the last good binary running and retry on the
#      next change.
#
# Run under `tmm task` so the log and state survive in a tmux window:
#   tmm task start server --replace -- scripts/dev-server-watch.sh
set -u
cd "$(dirname "$0")/.."
. "$HOME/.cargo/env" 2>/dev/null || true

BIN=src-tauri/target/release/server
MANIFEST=src-tauri/Cargo.toml
export HOST="${HOST:-127.0.0.1}"

# Newest mtime over the Rust sources that make up the server.
stamp() {
  find src-tauri/src src-tauri/crates src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs \
    -name '*.rs' -o -name 'Cargo.toml' -o -name 'Cargo.lock' 2>/dev/null \
    | xargs stat -c %Y 2>/dev/null | sort -rn | head -1
}

build() {
  echo "[watch] building server ($(date +%H:%M:%S))"
  cargo build --manifest-path "$MANIFEST" --no-default-features --release --bin server
}

last=""
pid=""

stop_server() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  pid=""
}
trap 'stop_server; exit 0' INT TERM

build || echo "[watch] initial build FAILED — will retry on next change"

while :; do
  if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
    if [ -n "$pid" ]; then
      echo "[watch] server exited ($(date +%H:%M:%S)) — restarting in 2s"
      sleep 2
    fi
    if [ -x "$BIN" ]; then
      "$BIN" & pid=$!
      echo "[watch] server up (pid $pid)"
    else
      echo "[watch] no binary yet — waiting for a successful build"
      sleep 5
    fi
  fi
  cur=$(stamp)
  if [ -n "$cur" ] && [ "$cur" != "$last" ]; then
    if [ -n "$last" ]; then
      echo "[watch] sources changed — rebuilding"
      if build; then
        echo "[watch] rebuild ok — restarting server"
        stop_server
      else
        echo "[watch] rebuild FAILED — keeping the running server"
      fi
    fi
    last="$cur"
  fi
  sleep 3
done
