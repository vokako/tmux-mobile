# System Status (server vitals) — design

Board #56. The WHAT is in
[requirements](../../requirements/features/system-status.md); this is why the
pieces are shaped the way they are.

## The client's poll interval IS the CPU measurement window

CPU% is a delta (busy time / total time between two samples). The obvious
implementations both cost something the feature doesn't need: a background
sampler thread ticks forever for a corner nobody may be looking at, and a
sleep-inside-the-request (sample, wait 200ms, sample) stalls every poll.

Instead `system_status.rs` keeps ONE process-wide `sysinfo::System` behind a
`OnceLock<Mutex<…>>` and each RPC call reports usage **since the previous
call**. The client's low tempo (20s) becomes the measurement window — the
lazy path is also the honest one. Two consequences are deliberate:

- The **first** call of a server's life has no window and answers
  `cpu_pct: null`. The client drops that part for a tick rather than showing
  a made-up 0%.
- Two clients polling share the window (each sees usage since ANYONE last
  asked). For a glanceable number this is fine; it is the documented trade.

## Dependency: `sysinfo`, pinned exact, features cut down

`sysinfo = "=0.39.6"`, `default-features = false`, features `system` + `disk`
only (no process table, no network counters, no multithread refresh).

- **Why a crate at all**: the macOS half (host_statistics64 / sysctl FFI) is
  unsafe code this project cannot exercise in CI or on the Linux dev host;
  sysinfo's readers for both platforms are the ecosystem-standard ones. No
  shell-outs (`df`/`top` are slow and parsing them is a dialect per OS), so
  there is no injection surface and no subprocess cost.
- **Why exact-pinned**: the crate renamed its reading APIs across recent
  minors (0.30→0.31→0.33); a caret would let an unrelated `cargo update`
  break the build.
- **Why desktop-gated** (same `cfg` as `projects`/agora): a phone is a client
  of a desktop server and never answers `system_status`; the sampler would be
  dead weight in the mobile shell.

## Which disk is "the disk"

`pick_root_disk` (pure, tested): the exact `/` mount when present with a
non-zero total, else the largest mounted disk. macOS splits the APFS
container across a read-only `/` and a data volume; "largest total" is the
capacity a person means by 「disk容量」. An empty/unreadable disk list yields
0/0, which the client treats as "nothing to say".

## Client: transport injected, tempo floored, failure keeps the reading

`src/lib/system/` is self-contained: `system.ts` (wire type + pure
formatters) and `SystemStatus.svelte` (the corner). The component takes a
`load: () => Promise<SystemStatus | null>` **prop** — it never imports
`ws.ts`, so it tests with a stub and integrates into App with two lines
(mount + wire `load` to the RPC), keeping the App/ws territory free for
concurrent work.

- **Tempo**: `interval` prop defaults to `SYS_POLL_MS` (20s) and is clamped
  to `SYS_POLL_MIN_MS` (5s) — the server computes CPU% over this interval,
  so a hot loop would be cost AND noise. Hidden (`visible === false`) stops
  the timer entirely (the hidden-terminal lesson in miniature); re-show
  reads immediately.
- **Fail-soft**: only a truthy answer overwrites the reading ("I could not
  ask" is not "there is nothing" — the roster lesson), and nothing renders
  until the first success (the verdict rule).
- **Formatting**: `fmtPair` puts used/total in the TOTAL's unit so the pair
  reads as a fraction (`210/473G`), one decimal only under 10. Labels
  CPU/MEM/DISK are universal abbreviations — no i18n entries needed.

## Integration (App, board #56 final phase)

`ws.ts` exports the typed wrapper — `systemStatus(): Promise<SystemStatus |
null>` — whose `catch(() => null)` absorbs EVERY failure including an older
or mobile server's method-not-found: null is the reading the component
already treats as "keep what I have / say nothing", so no error can escape
into the page. App mounts the corner once, gated by ONE derived flag
(`sysMounted = connected && !layout.isTouchDevice` — a phone's server is
remote and its corners belong to the bottom bar), with `visible={connected}`
so a dropped connection stops the timer even before the unmount.

**The corner is a FLOW footer; it never floats over content.** Two earlier
cuts each failed the lead/builder-3 review geometry: a fixed overlay with
`pointer-events: none` visually covered the rail and the sidebar's last row
(pass-through clicks do not un-hide pixels), and a `.page-layer`-only bottom
inset missed `.page`'s DIRECT children — `<Settings>` renders straight into
`.page`, not into a layer. The footer is now the LAST flow child of `main`'s
column flex: `.page` is the `flex: 1` row and shrinks above it, which
shortens the absolute layers (`inset: 0` tracks `.page`'s box) AND every
direct child alike — zero visual intersection on every current and FUTURE
page, by layout rather than by policing. `main.with-rail`'s `padding-left`
starts it right of the rail; no fixed positioning, no z-index, no
pointer-events games, and the hover tooltip survives.

Verifying "no overlap" needs a CLIP-AWARE intersection test:
`getBoundingClientRect` ignores overflow clipping, so a row half-scrolled
out of its `overflow: hidden` scroller has a rect crossing the footer
boundary while painting nothing there — intersect each rect with every
clipping ancestor first, then test overlap.

## Tests

- Rust (`system_status.rs` module tests): first-call-null then bounded
  percent; `pick_root_disk` preference + fallbacks + degenerate cases; the
  serialized wire shape.
- `system.test.ts`: formatter behaviour incl. clamps and verdict-rule
  emptiness; the tempo constants.
- `system.source.test.ts` pins the contracts refactors would silently break:
  no ws/App/hub imports, the interval clamp, hidden-stops-timer, the
  failed-load-keeps-reading shape, tokens-only type.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### Server vitals occupy space; they never overlay it

(board #56): desktop `system_status` samples CPU/MEM/root disk in-process through exact-pinned, desktop-gated `sysinfo`; the 20s client poll is the CPU delta window and first CPU is null. `SystemStatus` keeps the last good reading and stops hidden. App mounts it only on a connected desktop as `main`'s LAST flow footer; `.page` shrinks above it, covering absolute page layers and direct children such as Settings. Fixed overlays, pointer-through claims, and layer-only insets are retired — independent Chromium found each of those can still hide pixels.
