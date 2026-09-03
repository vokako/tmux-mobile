# Development — commands, the dev loop, and build gotchas

Everything about RUNNING and BUILDING the project. Testing conventions are in `testing.md`; frontend coding rules in `frontend.md`; runtime configuration in `../reference/config.md`.

## Commands

```bash
npm run dev:all          # one public :5173; proxies /ws + /dl to watched Rust
npm run dev              # Vite dev server only (web UI on 0.0.0.0:5173)
npm run dev:server:watch # standalone WS server; rebuild/restart on Rust changes
npm run tauri:dev        # Desktop app + WS server (dev mode)
npm run tauri:dev:release # Release-mode desktop app + WS server
npm run build:mac        # macOS .app + .dmg
npm run build:android    # Android APK (aarch64)
npm test                 # Frontend + dev-script tests (node --test; see docs/conventions/testing.md)
npm run check            # svelte-check: type-checks .ts/.svelte.ts/.svelte files
npm run build:server     # server + tmm, no webview needed (release)
npm run dev:server       # standalone WS server (pair with `npm run dev`)
npm run test:rust        # Rust tests, sequential (needs tmux running)


Run Tauri through these project scripts. Do not use `pnpx tauri`: `pnpx` is
`pnpm dlx` and downloads the unrelated `tauri` package instead of invoking the
installed `@tauri-apps/cli`. Release mode is the `--release` option, not a
positional `release` argument.

On a memory-constrained machine, lower Cargo concurrency per invocation without
changing the project default:
`CARGO_BUILD_JOBS=2 npm run tauri:dev:release`.

### Headless build (server + CLI, no webview)

```bash
npm run build:server     # server + tmm, release
npm run dev:all          # Vite + watched server (recommended browser dev loop)
npm run dev:server:watch # watched server only; pair with `npm run dev`
npm run dev:server       # one-shot server only; pair with `npm run dev`
npm run test:rust        # the Rust tests
```

**On this dev host the server is already supervised — do not tell the owner to
restart it.** `scripts/dev-server-watch.sh` runs under `tmm task` (window
`tmm-tasks:server`, started with
`tmm task start server --replace -- scripts/dev-server-watch.sh`) and is what
`tauri dev` would have given us for the server half, which cannot build here (no
webkit2gtk-4.1, no DISPLAY): it restarts the server when it EXITS (2 s backoff),
polls Rust source mtimes every 3 s (no inotify — NFS-safe, ~4 ms per scan),
rebuilds release after the tree has been quiet for one interval, and restarts
ONLY when the rebuilt binary's BYTES differ — cargo fingerprints by mtime, so a
`touch`, a checkout, or a comment-only edit relinks an identical binary and
dropping every client for that would be noise. A failed rebuild keeps the last
good binary running and retries on the next change. So an edit is live within
seconds (`[watch] binary changed — restarting server` in that window's log), and
the frontend needs no restart either — vite serves from disk, so a browser
reload is enough. `tmm task list` shows both. Owner correction, 2026-08-19.
These pass `--no-default-features`, which turns off the `gui` Cargo feature —
`tauri` plus its four plugins (dialog, fs, opener, notification). What remains is what the WebSocket server and
`tmm` actually use, and the webview leaves the dependency graph entirely. Use
this wherever there is no WebKit webview to link against (a Linux host with no
WebKitGTK development package, a CI runner, a container): on Linux `tauri`
reaches gtk3 and webkit2gtk-4.1 through wry, so a `--bin server` build that
needs none of it fails in `glib-sys`'s build script. It is also the faster way
to run the Rust tests anywhere, because none of them touch the Tauri shell.

Two things the gate has to preserve, both easy to break:
- `build.rs` emits the `desktop` / `mobile` cfg aliases itself when `gui` is
  off. They are not cosmetic — `mod team_bridge`, `mod team` and the `Config`
  import are gated on `desktop`. It cannot just call `tauri_build::build()`,
  which panics looking for the `cargo:dev` instruction `tauri`'s own build
  script would have emitted.
- The `tmux-mobile` bin still has to compile, so `main.rs` keeps a stub `main`
  that points at `server`.

The release profile has `incremental = false` deliberately. With it on, every
release build after a source edit failed at link time with hundreds of
`Undefined symbols: "_anon.<hash>.llvm.<n>" … referenced from libtmux_mobile.rlib`
and only `cargo clean -p tmux-mobile --release` recovered — the same cost as a
non-incremental build. If you hit that error on a branch that re-enables it,
that is the cause (this crate emits staticlib + cdylib + rlib, and LLVM's
per-CGU anonymous symbols get renamed between incremental runs).

Dev commands run a port preflight (`scripts/preflight.mjs`): if 5173/9899 are
already held they fail fast and print the owning PIDs instead of half-starting
(a second vite instance corrupts the dep-optimizer cache → blank page on every
open client). `build:android` ends with a postflight that re-points this
machine's gradle build-dir symlink at THIS checkout's output and prints the real
APK path. It computes the target rather than guessing: a global gradle init
script redirects `build/` to
`~/.cache/builds/gradle-builds/<dirname>-<md5(gradle root abs path)[:12]>/`, so
the slug is a pure function of the path. Two failure modes, and the second is
the dangerous one — dangling (cache pruned) is loud, but a link pointing at a
**second checkout of the same repo** hashes to a different slug and makes a
green build silently serve that other tree's older APK. Healing only the
dangling case, or picking the newest APK across all `android-*` dirs, is how you
end up shipping the wrong tree.


## Android signing

The release keystore is NOT in the repository. `gen/android/app/build.gradle.kts`
reads `src-tauri/gen/android/key.properties` (gitignored; copy
`key.properties.example` and fill in `storeFile`, `storePassword`, `keyAlias`,
`keyPassword`; `storeFile` is relative to `gen/android/`). Without the file a
release build still succeeds but the APK is unsigned, and gradle says so.

Why: until 2026-09-03 `keystore.jks` and its passwords were committed in the
gradle script. Anyone with the repository could sign an update that installed
copies of `com.tmuxmobile.dev` would accept. The file was untracked that day; the
key itself is still the one installed apps trust, so rotating it means users
reinstall — an owner decision, not a build step. Until it is rotated, treat the
history as containing a live signing key.

## Workflow

- **Commit after every verified change** (owner's standing instruction): once a fix/feature is tested and its docs are updated, commit it right away — one logical change per commit. Don't let verified work sit uncommitted in the tree. Never commit `agent-team-page/` or other unrelated in-progress work without being asked.

## Testing

```bash
tmux new-session -d -s test
npm run test:rust
```
Rust tests are sequential (shared tmux state), spread across `src-tauri/src/*.rs`
(`main.rs`, `tmux.rs`, `team_bridge.rs`, and the `server/` and `team/`
modules — unit tests live in the submodule they test) plus
`src-tauri/crates/agora/tests/`. Frontend tests: `npm test` (node --test,
`src/**/*.test.{js,ts}`, no tmux needed; conventions in
