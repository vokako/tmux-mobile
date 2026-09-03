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
  <!-- Quiet chrome, not a message: micro type, meta ink, mono numbers. The
       title carries the same words for hover/assistive reading. -->
  <div class="sysvitals appear" title={parts.map((p) => `${p.k} ${p.v}`).join(' · ')}>
    {#each parts as p (p.k)}
      <span class="sv"><span class="sv-k">{p.k}</span><span class="sv-v">{p.v}</span></span>
    {/each}
  </div>
{/if}

<style>
  .sysvitals {
    display: flex;
    gap: 10px;
    align-items: baseline;
    font-size: var(--fs-micro);
    color: var(--text3);
    line-height: 1;
    user-select: none;
    white-space: nowrap;
  }
  .sv {
    display: inline-flex;
    gap: 4px;
    align-items: baseline;
  }
  .sv-k {
    letter-spacing: 0.04em;
    opacity: 0.75;
  }
  .sv-v {
    font-family: var(--mono, ui-monospace, monospace);
    font-variant-numeric: tabular-nums;
  }
</style>
