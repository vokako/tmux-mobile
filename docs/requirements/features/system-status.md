# System Status (server vitals) — requirements

Board #56. Owner (2026-09-01): 「可以在系统端左下角 显示系统 cpu mem disk容量
状态 可以不用很高刷新率 大概看个数就行」.

## What

The desktop client shows, in its bottom-left corner, three numbers about the
MACHINE THE SERVER RUNS ON (not the phone, not the browser):

- **CPU** — whole-machine usage percent.
- **MEM** — used/total, one shared unit (e.g. `3.4/16G`).
- **DISK** — used/total of the root filesystem, one shared unit.

## Behaviour

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
