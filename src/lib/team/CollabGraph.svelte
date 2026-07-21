<script lang="ts">
  // Ring collaboration graph: every participant (the human + live agents) sits on
  // a circle, coloured by work status. Each new directed message lays down an arc
  // from sender → addressee that slowly fades over ~1 min, while a glowing point
  // streams along it. Driven by `agents` (roster) and `event` (latest message).
  import { t } from '../core/i18n.svelte.ts';

  type GraphAgent = { name: string; status?: string };
  type GraphEvent = { id?: string | number; kind?: string; from?: string; body?: string };
  type Node = { name: string; status: string; x: number; y: number; lx: number; ly: number; anchor: string };

  let { agents = [], event = null }: { agents?: GraphAgent[]; event?: GraphEvent | null } = $props();

  const CX = 100, CY = 100, R = 58, LR = 73;
  // Connection decay is COUNT-based: the glowing dot makes ARC_PASSES trips along
  // the line, each dimmer than the last, then the arc is removed.
  const ARC_PASSES = 10;
  const PASS_MS = 1600;
  const ARC_LIFE_MS = ARC_PASSES * PASS_MS;

  // Nodes: human pinned to the top, agents spread evenly clockwise.
  let nodes = $derived.by(() => {
    const list = [{ name: 'human', status: 'human' },
      ...agents.map(a => ({ name: a.name, status: a.status || 'online' }))];
    const n = list.length;
    return list.map((node, i) => {
      const ang = (-90 + (i * 360) / n) * Math.PI / 180;
      const cos = Math.cos(ang), sin = Math.sin(ang);
      return {
        ...node,
        x: CX + R * cos,
        y: CY + R * sin,
        lx: CX + LR * cos,
        ly: CY + LR * sin,
        anchor: cos > 0.35 ? 'start' : cos < -0.35 ? 'end' : 'middle',
      };
    });
  });

  function nodeByName(name: string | undefined | null): Node | null {
    if (!name) return null;
    const lc = name.toLowerCase();
    return nodes.find(nd => nd.name.toLowerCase() === lc) || null;
  }

  let arcs = $state<{ id: number; d: string }[]>([]);
  let pulses = $state<Set<string>>(new Set());
  let seq = 0;

  function pulse(name: string | undefined) {
    const nd = nodeByName(name);
    if (!nd) return;
    const next = new Set(pulses); next.add(nd.name); pulses = next;
    setTimeout(() => { const s = new Set(pulses); s.delete(nd.name); pulses = s; }, 700);
  }

  function arcPath(a: Node, b: Node): string {
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
    const qx = mx + (CX - mx) * 0.5, qy = my + (CY - my) * 0.5;
    return `M${a.x.toFixed(1)} ${a.y.toFixed(1)} Q${qx.toFixed(1)} ${qy.toFixed(1)} ${b.x.toFixed(1)} ${b.y.toFixed(1)}`;
  }

  function spawnArc(fromName: string | undefined, toName: string) {
    const a = nodeByName(fromName), b = nodeByName(toName);
    if (!a || !b || a.name === b.name) return;
    const id = ++seq;
    arcs = [...arcs.slice(-50), { id, d: arcPath(a, b) }];
    pulse(a.name); pulse(b.name);
    setTimeout(() => { arcs = arcs.filter(x => x.id !== id); }, ARC_LIFE_MS);
  }

  function targetsOf(body: string | undefined) {
    const out: string[] = [];
    const re = /@([a-z0-9_*-]+)/gi;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body || '')) !== null) out.push(m[1]!.toLowerCase());
    return out;
  }

  let seenId: string | number | null = null;
  $effect(() => {
    const m = event;
    if (!m || !m.id || m.id === seenId) return;
    seenId = m.id;
    if (m.kind && m.kind !== 'msg') return;
    const from = m.from;
    const tokens = targetsOf(m.body);
    if (tokens.some(t => t === 'all' || t === '*')) {
      for (const nd of nodes) if (nd.name.toLowerCase() !== (from || '').toLowerCase()) spawnArc(from, nd.name);
    } else if (tokens.length) {
      for (const tk of tokens) spawnArc(from, tk);
    } else {
      pulse(from);
    }
  });

  const legend = [
    { cls: 'idle',        key: 'collabIdle' },
    { cls: 'thinking',    key: 'collabThinking' },
    { cls: 'working',     key: 'collabWorking' },
    { cls: 'hardworking', key: 'collabHardworking' },
    { cls: 'stalled',     key: 'collabStalled' },
    { cls: 'sleeping',    key: 'collabSleeping' },
  ];
</script>

<div class="collab">
  {#if nodes.length <= 1}
    <div class="collab-empty">…</div>
  {:else}
    <svg viewBox="0 0 200 200" class="collab-svg" preserveAspectRatio="xMidYMid meet">
      <circle class="ring" cx={CX} cy={CY} r={R} />
      {#each arcs as arc (arc.id)}
        <g class="arc" style="--life:{ARC_LIFE_MS}ms; --pass:{PASS_MS}ms; --passes:{ARC_PASSES}">
          <path class="arc-base" d={arc.d} />
          <path class="arc-comet" d={arc.d} pathLength="1" />
        </g>
      {/each}
      {#each nodes as nd (nd.name)}
        <g class="node" class:pulse={pulses.has(nd.name)}>
          <circle class="dot status-{nd.status}" cx={nd.x} cy={nd.y} r="7" />
          <text class="lbl" x={nd.lx} y={nd.ly} text-anchor={nd.anchor} dominant-baseline="middle">{nd.name}</text>
        </g>
      {/each}
    </svg>
    <div class="legend">
      {#each legend as l}
        <span class="leg-item"><i class="leg-dot status-{l.cls}"></i>{t(l.key)}</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .collab { position: relative; width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; padding: 6px 14px 22px; box-sizing: border-box; }
  .collab-empty { color: var(--text3); font-size: 12px; }
  .collab-svg { width: 100%; height: 100%; overflow: visible; }

  .ring { fill: none; stroke: var(--border2); stroke-width: 0.75; opacity: 0.6; }

  .dot {
    stroke: var(--bg); stroke-width: 1.5;
    transform-box: fill-box; transform-origin: center;
    transition: transform 0.25s ease;
  }
  .dot.status-idle        { fill: var(--status-ok); }
  .dot.status-thinking    { fill: var(--accent); }
  .dot.status-working     { fill: var(--status-warn); }
  .dot.status-hardworking { fill: var(--status-hot); }
  .dot.status-stalled     { fill: var(--status-danger); }
  .dot.status-sleeping    { fill: var(--status-sleep); }
  .dot.status-online      { fill: var(--accent); }
  .dot.status-offline     { fill: var(--text3); }
  .dot.status-human       { fill: var(--bg); stroke: var(--accent); stroke-width: 2.5; }
  .node.pulse .dot { transform: scale(1.55); }

  /* Every present dot "breathes" continuously (a soft in-place scale loop) so
     the graph is always alive — resting states (idle/online/you) breathe gently
     and slowly, active states more and faster. Sleeping breathes slowest of all
     and dims (a parked, napping team). Only stalled/offline stay still
     (stuck / gone). The running animation owns `transform`, so it supersedes the
     discrete .node.pulse scale — the comet arc still signals message events. */
  @keyframes breathe {
    0%, 100% { transform: scale(1); }
    50%      { transform: scale(1.22); }
  }
  @keyframes breathe-soft {
    0%, 100% { transform: scale(1); }
    50%      { transform: scale(1.1); }
  }
  .dot.status-idle        { animation: breathe-soft 3.4s ease-in-out infinite; }
  .dot.status-online      { animation: breathe-soft 3.4s ease-in-out infinite; }
  .dot.status-human       { animation: breathe-soft 3.4s ease-in-out infinite; }
  .dot.status-thinking    { animation: breathe 2s   ease-in-out infinite; }
  .dot.status-working     { animation: breathe 1.3s ease-in-out infinite; }
  .dot.status-hardworking { animation: breathe 2.8s ease-in-out infinite; }
  .dot.status-sleeping    { animation: breathe-soft 5s ease-in-out infinite; opacity: 0.5; }

  .lbl { fill: var(--text2); font-size: 8px; font-weight: 600; }

  /* Connection decay is count-based: ARC_PASSES trips of the glowing dot, each
     a step dimmer than the last, then the arc is removed. */
  .arc {
    animation-name: arc-steps;
    animation-duration: var(--life, 8000ms);
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }
  /* Ten discrete levels (one per pass) — keep in sync with ARC_PASSES = 10. */
  @keyframes arc-steps {
    0%, 10% { opacity: 1; }
    10.01%, 20% { opacity: 0.9; }
    20.01%, 30% { opacity: 0.8; }
    30.01%, 40% { opacity: 0.7; }
    40.01%, 50% { opacity: 0.6; }
    50.01%, 60% { opacity: 0.5; }
    60.01%, 70% { opacity: 0.4; }
    70.01%, 80% { opacity: 0.3; }
    80.01%, 90% { opacity: 0.2; }
    90.01%, 100% { opacity: 0.1; }
  }
  .arc-base { fill: none; stroke: var(--accent); stroke-width: 1.5; opacity: 0.22; }
  .arc-comet {
    fill: none; stroke: var(--accent); stroke-width: 2.5; stroke-linecap: round;
    stroke-dasharray: 0.05 0.95; stroke-dashoffset: 1;
    animation-name: comet;
    animation-duration: var(--pass, 1600ms);
    animation-timing-function: linear;
    animation-iteration-count: var(--passes, 5);
    filter: drop-shadow(0 0 3px var(--accent));
  }
  @keyframes comet { from { stroke-dashoffset: 1; } to { stroke-dashoffset: 0; } }

  .legend {
    position: absolute; left: 10px; bottom: 4px;
    display: flex; flex-wrap: wrap; gap: 4px 10px;
    font-size: 10px; color: var(--text3);
  }
  .leg-item { display: inline-flex; align-items: center; gap: 4px; }
  .leg-dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
  .leg-dot.status-idle        { background: var(--status-ok); }
  .leg-dot.status-thinking    { background: var(--accent); }
  .leg-dot.status-working     { background: var(--status-warn); }
  .leg-dot.status-hardworking { background: var(--status-hot); }
  .leg-dot.status-stalled     { background: var(--status-danger); }
  .leg-dot.status-sleeping    { background: var(--status-sleep); }
  .leg-dot.status-human       { background: var(--bg); border: 2px solid var(--accent); box-sizing: border-box; }

  @media (prefers-reduced-motion: reduce) {
    .arc-comet { animation: none; stroke-dasharray: none; }
    .dot { transition: none; animation: none; }
  }
</style>
