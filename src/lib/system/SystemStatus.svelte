<script lang="ts">
  // Server system vitals corner — board #56 「服务端系统状态显示」.
  //
  // DELIBERATELY DUMB about transport: the `load` callback is INJECTED by
  // whoever mounts this (App wires it to the `system_status` RPC in its own
  // two-line integration — this file never imports ws.ts, so the #55/#57
  // territories stay untouched and the component tests need no socket).
  //
  // Tempo: one load per SYS_POLL_MS (20s — the owner asked for a low refresh,
  // "大概看个数就行"), floored at SYS_POLL_MIN_MS so no caller can wire a hot
  // loop; the SERVER computes CPU% over this very interval, so the tempo is
  // also the measurement window. While `visible` is false the timer stops —
  // a hidden corner reading the CPU is the hidden-terminal mistake in
  // miniature. On re-show it reads immediately (the old reading is stale by
  // up to a whole interval).
  //
  // Fail-soft both ways: a failed/null load KEEPS the last reading ("I could
  // not ask" is not "there is nothing", the roster's lesson), and nothing at
  // all renders until the first successful reading (the verdict rule — no
  // flash of zeros).
  import { sysParts, SYS_POLL_MS, SYS_POLL_MIN_MS, type SystemStatus } from './system.ts';

  let {
    load,
    interval = SYS_POLL_MS,
    visible = true,
  }: {
    load: () => Promise<SystemStatus | null>;
    interval?: number;
    visible?: boolean;
  } = $props();

  let status = $state<SystemStatus | null>(null);
  const parts = $derived(sysParts(status));
  const tempo = $derived(Math.max(SYS_POLL_MIN_MS, interval));

  async function tick() {
    try {
      const r = await load();
      if (r) status = r; // a null/failed answer keeps the last reading
    } catch {
      /* fail-soft: telemetry may never break the page that carries it */
    }
  }

  // ONE effect owns the timer: visible → an immediate read (the old reading
  // is up to a whole interval stale) + the interval; hidden → cleanup stops
  // it. Re-runs when `visible` or the clamped tempo change.
  $effect(() => {
    if (!visible) return;
    void tick();
    const timer = setInterval(tick, tempo);
    return () => clearInterval(timer);
  });
</script>

{#if parts.length}
  <!-- One compact strip inside the desktop PRIMARY sidebar. Each metric gets
       a categorical hue, while the number itself uses full-strength ink: the
       previous all-text3 row was too grey to read in either theme. The title
       keeps the same complete reading for hover/assistive inspection. -->
  <div class="sysvitals appear" title={parts.map((p) => `${p.k} ${p.v}`).join(' · ')}>
    {#each parts as p (p.k)}
      <span class="sv" class:cpu={p.k === 'CPU'} class:mem={p.k === 'MEM'} class:disk={p.k === 'DISK'}>
        <span class="sv-k">{p.k}</span><span class="sv-v">{p.v}</span>
      </span>
    {/each}
  </div>
{/if}

<style>
  .sysvitals {
    width: 100%;
    min-width: 0;
    display: flex;
    gap: 4px;
    align-items: center;
    font-size: var(--fs-micro);
    line-height: 1;
    user-select: none;
    white-space: nowrap;
  }
  .sv {
    --sv-hue: var(--accent);
    flex: 1 1 0;
    min-width: 0;
    display: inline-flex;
    justify-content: center;
    gap: 4px;
    align-items: baseline;
    padding: 4px 5px;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--sv-hue) 24%, transparent);
    border-radius: var(--ui-radius-control);
    background: color-mix(in srgb, var(--sv-hue) 9%, transparent);
  }
  .sv.mem { --sv-hue: var(--status-purple); }
  .sv.disk { --sv-hue: var(--status-ok); }
  .sv-k {
    flex: none;
    color: var(--sv-hue);
    font-weight: 650;
    letter-spacing: 0.04em;
  }
  .sv-v {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
    font-family: var(--font-mono);
    font-weight: 550;
    font-variant-numeric: tabular-nums;
  }
</style>
