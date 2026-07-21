<script>
  // Desktop-only agent grid for the Team tab. Tiles one read-only-by-default
  // terminal preview per team employee, laid out in a near-square grid
  // (w = ceil(√n), h = ceil(n/w)). Click a cell to activate it (focus + full
  // interaction, like an active split cell); inactive cells stay read-only.
  // Unlike SplitView there is NO switcher chrome — each cell is pinned to its
  // agent's pane (window_name == agent name in the per-workspace team session).
  import Terminal from '../terminal/Terminal.svelte';
  import Icon from '../ui/Icon.svelte';
  import CollabGraph from './CollabGraph.svelte';
  import { t } from '../core/i18n.svelte.js';
  import { listSessionsWithPanes } from '../core/ws.ts';
  import { paneAgent } from '../core/agents.js';

  let {
    teamSession = '',     // tmm-team-<slug>: the session whose windows are agents
    employees = [],       // [{ name, state, ... }] desired roster (all employees)
    fontSize = 14,        // standard size; cells render two notches smaller
    visible = false,
    collab = false,       // show the collaboration graph as the first cell
    collabAgents = [],    // roster for the graph
    collabEvent = null,   // latest live message driving the graph's arcs
  } = $props();

  // Any cell (agent terminal OR the graph) can be maximized to fill the pane.
  let expandedName = $state(null);

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
        if (p.session === teamSession && p.window_name) map[p.window_name] = p;
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
  // window appears), so the grid shape is stable as the team comes up. FIRED
  // (disabled) employees are dropped — their window is killed, so a cell for
  // them would spin forever waiting for a pane that never returns.
  let cells = $derived([
    ...(collab ? [{ name: '__collab__', collab: true }] : []),
    ...employees
      .filter(e => e.state !== 'disabled')
      .map(e => ({ name: e.name, state: e.state, pane: panesByName[e.name] || null })),
  ]);

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

  // The currently-maximized cell, if any; cleared if it leaves the grid.
  let expandedCell = $derived(cells.find(c => c.name === expandedName) || null);
  $effect(() => {
    if (expandedName && !cells.some(c => c.name === expandedName)) expandedName = null;
  });
</script>

<div
  class="agent-grid"
  style="grid-template-columns: repeat({cols}, 1fr); grid-template-rows: repeat({rows}, 1fr);"
>
  {#each cells as cell (cell.name)}
    {@const agent = (!cell.collab && cell.pane) ? paneAgent(cell.pane) : null}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="ag-cell"
      class:active={!cell.collab && cell.name === activeName}
      class:ag-collab={cell.collab}
      onmousedowncapture={() => activeName = cell.name}
    >
      <div class="ag-head">
        {#if cell.collab}
          <Icon name="collab" size={11} />
        {:else if agent}
          <img class="ag-icon" src={agent.icon} alt={agent.tag} />
        {:else}
          <Icon name="bot" size={11} />
        {/if}
        <span class="ag-name">{cell.collab ? t('teamCollab') : cell.name}</span>
        {#if !cell.collab && !cell.pane}<span class="ag-pending">…</span>{/if}
        <span class="ag-head-actions">
          <button class="ag-head-btn" title="Expand" aria-label="Expand" onclick={() => expandedName = cell.name}>
            <Icon name="maximize" size={12} />
          </button>
        </span>
      </div>
      <div class="ag-body">
        {#if expandedName === cell.name}
          <div class="ag-empty"><Icon name="maximize" size={16} /></div>
        {:else if cell.collab}
          <CollabGraph agents={collabAgents} event={collabEvent} />
        {:else if cell.pane}
          {#key targetOf(cell.pane)}
            <Terminal
              target={targetOf(cell.pane)}
              session={cell.pane.session}
              command={cell.pane.current_command || ''}
              embedded={true}
              chromeless={true}
              active={cell.name === activeName}
              fontSize={cellFont}
            />
          {/key}
        {:else}
          <div class="ag-empty"><span class="reconnect-spinner-sm"></span></div>
        {/if}
      </div>
    </div>
  {/each}
</div>

{#if expandedCell}
  <!-- A maximized cell takes over the whole preview pane (graph or any agent). -->
  <div class="ag-expanded">
    <div class="ag-head">
      {#if expandedCell.collab}
        <Icon name="collab" size={12} />
      {:else if expandedCell.pane}
        {@const a2 = paneAgent(expandedCell.pane)}
        {#if a2}<img class="ag-icon" src={a2.icon} alt={a2.tag} />{:else}<Icon name="bot" size={12} />{/if}
      {:else}
        <Icon name="bot" size={12} />
      {/if}
      <span class="ag-name">{expandedCell.collab ? t('teamCollab') : expandedCell.name}</span>
      <span class="ag-head-actions">
        <button class="ag-head-btn" title="Collapse" aria-label="Collapse" onclick={() => expandedName = null}>
          <Icon name="minimize" size={13} />
        </button>
      </span>
    </div>
    <div class="ag-body">
      {#if expandedCell.collab}
        <CollabGraph agents={collabAgents} event={collabEvent} />
      {:else if expandedCell.pane}
        {#key targetOf(expandedCell.pane)}
          <Terminal
            target={targetOf(expandedCell.pane)}
            session={expandedCell.pane.session}
            command={expandedCell.pane.current_command || ''}
            embedded={true}
            chromeless={true}
            active={true}
            fontSize={fontSize}
          />
        {/key}
      {:else}
        <div class="ag-empty"><span class="reconnect-spinner-sm"></span></div>
      {/if}
    </div>
  </div>
{/if}

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
    /* Isolate each cell into its own stacking context so the embedded Terminal's
       z-indexed bits (scrollbar slider, floating buttons up to z-index 20) stay
       trapped inside the cell and can't paint over the .ag-expanded overlay. */
    position: relative;
    isolation: isolate;
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
  .ag-head-actions { margin-left: auto; display: flex; gap: 2px; }
  .ag-head-btn {
    background: none; border: none; color: var(--text3);
    padding: 2px; display: inline-flex; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .ag-head-btn:active { color: var(--accent); }
  .ag-collab .ag-body { background: var(--surface); }
  .ag-expanded {
    position: absolute; inset: 0; z-index: 10;
    display: flex; flex-direction: column;
    background: var(--bg); border: 1px solid var(--accent); border-radius: 8px;
    overflow: hidden;
  }
  .ag-icon { width: 13px; height: 13px; flex-shrink: 0; }
  .ag-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ag-pending { color: var(--text3); }
  /* Flex column so the embedded <Terminal> (.terminal { flex: 1 }) is
     constrained to the cell's height. Without display:flex here, .terminal
     ignores its flex basis and grows to xterm's natural row count, rendering
     ~47 rows inside a ~22-row box — the live TUI chrome (status/input lines)
     ends up clipped below the fold and the cell shows only stale scrollback. */
  .ag-body { flex: 1; min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }
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
