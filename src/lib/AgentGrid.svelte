<script>
  // Desktop-only agent grid for the Team tab. Tiles one read-only-by-default
  // terminal preview per crew employee, laid out in a near-square grid
  // (w = ceil(√n), h = ceil(n/w)). Click a cell to activate it (focus + full
  // interaction, like an active split cell); inactive cells stay read-only.
  // Unlike SplitView there is NO switcher chrome — each cell is pinned to its
  // agent's pane (window_name == agent name in the per-workspace crew session).
  import Terminal from './Terminal.svelte';
  import Icon from './Icon.svelte';
  import { listSessionsWithPanes } from './ws.js';
  import { paneAgent } from './agents.js';

  let {
    crewSession = '',     // tmm-crew-<slug>: the session whose windows are agents
    employees = [],       // [{ name, state, ... }] desired roster (all employees)
    fontSize = 14,        // standard size; cells render two notches smaller
    visible = false,
  } = $props();

  // Cell font: two notches below the app's standard size (agent previews are
  // glanceable, not the primary editing surface), clamped to a legible floor.
  let cellFont = $derived(Math.max(8, fontSize - 2));

  // name -> pane ({session,window,pane,current_command,window_name,...}).
  // Refreshed on a poll so a cell starts showing output as soon as its agent's
  // window exists (agents are launched a few seconds after Start team).
  let panesByName = $state({});
  let activeName = $state(null);

  async function loadPanes() {
    try {
      const { panes } = await listSessionsWithPanes();
      const map = {};
      for (const p of panes || []) {
        if (p.session === crewSession && p.window_name) map[p.window_name] = p;
      }
      panesByName = map;
    } catch { /* transient; next tick retries */ }
  }

  const POLL_MS = 2000;
  $effect(() => {
    if (!visible) return;
    loadPanes();
    const id = setInterval(loadPanes, POLL_MS);
    return () => clearInterval(id);
  });

  // Stable cell order: employees in roster order (manager/worker/reviewer/…).
  // Offline / not-yet-launched agents still get a cell (placeholder until their
  // window appears), so the grid shape is stable as the crew comes up.
  let cells = $derived(employees.map(e => ({ name: e.name, state: e.state, pane: panesByName[e.name] || null })));

  // Near-square grid: columns = ceil(√n), rows = ceil(n / columns).
  let cols = $derived(Math.max(1, Math.ceil(Math.sqrt(cells.length || 1))));
  let rows = $derived(Math.max(1, Math.ceil((cells.length || 1) / cols)));

  // Default the active cell to the first one with a live pane.
  $effect(() => {
    if (activeName && cells.some(c => c.name === activeName)) return;
    const first = cells.find(c => c.pane) || cells[0];
    activeName = first?.name ?? null;
  });

  function targetOf(p) { return `${p.session}:${p.window}.${p.pane}`; }
</script>

<div
  class="agent-grid"
  style="grid-template-columns: repeat({cols}, 1fr); grid-template-rows: repeat({rows}, 1fr);"
>
  {#each cells as cell (cell.name)}
    {@const agent = cell.pane ? paneAgent(cell.pane) : null}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="ag-cell"
      class:active={cell.name === activeName}
      onmousedowncapture={() => activeName = cell.name}
    >
      <div class="ag-head">
        {#if agent}
          <img class="ag-icon" class:claude={agent.tag === 'Claude'} src={agent.icon} alt={agent.tag} />
        {:else}
          <Icon name="bot" size={11} />
        {/if}
        <span class="ag-name">{cell.name}</span>
        {#if !cell.pane}<span class="ag-pending">…</span>{/if}
      </div>
      <div class="ag-body">
        {#if cell.pane}
          {#key targetOf(cell.pane)}
            <Terminal
              target={targetOf(cell.pane)}
              session={cell.pane.session}
              command={cell.pane.current_command || ''}
              viewMode="terminal"
              embedded={true}
              chromeless={true}
              active={cell.name === activeName}
              fontSize={cellFont}
            />
          {/key}
        {:else}
          <div class="ag-empty">
            <span class="reconnect-spinner-sm"></span>
          </div>
        {/if}
      </div>
    </div>
  {/each}
</div>

<style>
  .agent-grid {
    display: grid;
    gap: 4px;
    height: 100%;
    width: 100%;
    padding: 4px;
    box-sizing: border-box;
    background: var(--bg);
  }
  .ag-cell {
    display: flex;
    flex-direction: column;
    /* min-* let grid children shrink so xterm's ResizeObserver sees the real box. */
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    background: var(--bg);
  }
  .ag-cell.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent), 0 0 10px var(--accent-glow);
  }
  .ag-head {
    display: flex; align-items: center; gap: 6px;
    padding: 4px 8px; flex-shrink: 0;
    border-bottom: 1px solid var(--border2);
    background: var(--surface);
    font-size: 11px; font-weight: 600; color: var(--text2);
  }
  .ag-cell.active .ag-head { color: var(--accent); }
  .ag-icon { width: 13px; height: 13px; flex-shrink: 0; }
  .ag-icon.claude { width: 15px; height: 15px; }
  .ag-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ag-pending { color: var(--text3); }
  .ag-body { flex: 1; min-width: 0; min-height: 0; position: relative; }
  .ag-empty {
    width: 100%; height: 100%;
    display: flex; align-items: center; justify-content: center;
    color: var(--text3);
  }
  .reconnect-spinner-sm {
    width: 14px; height: 14px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%;
    animation: ag-spin 0.6s linear infinite;
  }
  @keyframes ag-spin { to { transform: rotate(360deg); } }
</style>
