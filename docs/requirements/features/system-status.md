# System Status (server vitals) — requirements

Board #56. Owner (2026-09-01): 「可以在系统端左下角 显示系统 cpu mem disk容量
状态 可以不用很高刷新率 大概看个数就行」.

## What

The connected client shows three numbers about the MACHINE THE SERVER RUNS ON
(not the phone or browser process). They are always present in the desktop
primary sidebar and appear at the bottom of an open primary side drawer on a
phone:

- **CPU** — whole-machine usage percent.
- **MEM** — used/total, one shared unit (e.g. `3.4/16G`).
- **DISK** — used/total of the root filesystem, one shared unit.

## Behaviour

- **Sidebar-only on both layouts.** On a connected desktop the reading
  occupies reserved space at the bottom of the primary sidebar
  (`--sidebar-w`) after the icon rail. On touch it stays hidden until the
  current page's shared side drawer opens, then occupies that drawer's bottom
  row above the safe area. It never draws or reserves a row under the main
  terminal/content area.
- **Compact, quiet monitor treatment.** The whole strip is 24px high. CPU,
  MEM, and DISK keep their natural widths and flow left — never three equal
  columns. They form one unboxed readout with only restrained 4px
  accent/purple/green dots, muted labels, and clear mono values. There are no
  per-metric coloured cards, borders, or fills competing with project work.
- **Low refresh.** One reading every ~20 seconds. This is a glanceable
  number, not a monitor; the tempo is also the CPU measurement window (see
  design doc).
- **Fail-soft, both directions.** A failed poll keeps the last reading; a
  server that cannot answer (mobile server, older server without the RPC)
  means the corner simply never appears. The corner must never affect the
  connection, the terminal, or any other feature.
- **First sample has no CPU.** CPU% needs a delta window, so the first-ever
  reading shows MEM/DISK only and CPU appears on the next tick — never a
  fabricated 0%.
- **Nothing renders before the first successful reading** (the app-wide
  verdict rule: no flash of zeros).

## API

`system_status` (no params) →

```json
{ "cpu_pct": 12.5, "mem_used": 0, "mem_total": 0, "disk_used": 0, "disk_total": 0 }
```

Bytes everywhere; `cpu_pct` is `null` on the server's first sample. The
method is desktop-only (Android/iOS report method-not-found, same contract as
`project_*`).
