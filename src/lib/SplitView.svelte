<script>
  // Desktop split-screen: tiles N independent Terminal instances in a CSS
  // grid, each bound to any session:window.pane. Mounted only when
  // App.svelte decides split is active (desktop + wide). Each cell owns its
  // own Terminal (own subscription, own xterm, own resize) — the ws.js
  // per-target listener registry is what lets them coexist.
  import Terminal from './Terminal.svelte';
  import AgentChip from './AgentChip.svelte';
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';
  import { listSessionsWithPanes } from './ws.js';
  import { paneAgent } from './agents.js';

  let {
    cells,            // [{ id, target, session, command }]
    layout,           // 2 | 3 | 4 | 6
    activeCellId,
    fontSize = 14,
    onActivate = () => {},   // (cellId)
    onAssign = () => {},     // (cellId, target, session, command)
    onCloseCell = () => {},  // (cellId)
    onPaneExit = () => {},   // (cellId)
  } = $props();

  // Pane picker popover state. `pickerCellId` is the cell currently choosing
  // a pane (null = closed). Pane data is fetched on open, mirroring
  // Terminal.svelte's loadOtherAgentSessions pattern.
  let pickerCellId = $state(null);
  let pickerSessions = $state([]); // [{ name, panes: [pane] }]
  let pickerLoading = $state(false);

  async function openPicker(cellId) {
    pickerCellId = cellId;
    pickerLoading = true;
    try {
      const { sessions, panes } = await listSessionsWithPanes();
      const bySession = new Map();
      for (const p of panes) {
        const arr = bySession.get(p.session);
        if (arr) arr.push(p); else bySession.set(p.session, [p]);
      }
      // Keep tmux's session order; only include sessions that have panes.
      pickerSessions = sessions
        .map(s => ({ name: s.name, panes: bySession.get(s.name) || [] }))
        .filter(s => s.panes.length);
    } catch {
      pickerSessions = [];
    }
    pickerLoading = false;
  }

  function closePicker() { pickerCellId = null; }

  function pickPane(cellId, p) {
    onAssign(cellId, `${p.session}:${p.window}.${p.pane}`, p.session, p.current_command);
    closePicker();
  }

  function cellLabel(cell) {
    return cell.command || cell.target || '';
  }
</script>

<div class="split-grid layout-{layout}">
  {#each cells as cell (cell.id)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="cell"
      class:active={cell.id === activeCellId}
      onmousedowncapture={() => onActivate(cell.id)}
    >
      <div class="cell-header">
        <AgentChip
          agent={cell.target ? paneAgent(cell) : null}
          label={cell.target ? cellLabel(cell) : t('pickPane')}
          variant={cell.id === activeCellId ? 'active' : 'default'}
          iconName={cell.target ? '' : 'plus'}
          title={cell.target || t('pickPane')}
          onclick={(e) => { e.stopPropagation(); pickerCellId === cell.id ? closePicker() : openPicker(cell.id); }}
        />
        <div class="cell-spacer"></div>
        {#if cell.target}
          <button class="cell-close" title={t('close')} onclick={(e) => { e.stopPropagation(); onCloseCell(cell.id); }}>
            <Icon name="x" size={12} />
          </button>
        {/if}
      </div>

      <div class="cell-body">
        {#if cell.target}
          {#key cell.target}
            <Terminal
              target={cell.target}
              session={cell.session}
              command={cell.command}
              viewMode="terminal"
              {fontSize}
              onSwitchPane={(t2, cmd) => onAssign(cell.id, t2, t2.split(':')[0], cmd)}
              onPaneExit={() => onPaneExit(cell.id)}
            />
          {/key}
        {:else}
          <button class="cell-empty" onclick={(e) => { e.stopPropagation(); openPicker(cell.id); }}>
            <Icon name="plus" size={20} />
            <span>{t('pickPane')}</span>
          </button>
        {/if}
      </div>

      {#if pickerCellId === cell.id}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="picker-backdrop" onclick={(e) => { e.stopPropagation(); closePicker(); }}></div>
        <div class="picker">
          {#if pickerLoading}
            <div class="picker-empty">…</div>
          {:else if pickerSessions.length === 0}
            <div class="picker-empty">{t('noSessions')}</div>
          {:else}
            {#each pickerSessions as s}
              <div class="picker-session">{s.name}</div>
              <div class="picker-panes">
                {#each s.panes as p}
                  {@const isCur = cell.target === `${p.session}:${p.window}.${p.pane}`}
                  <AgentChip
                    agent={paneAgent(p)}
                    label={p.current_command || p.window_name || `${p.window}.${p.pane}`}
                    variant={isCur ? 'active' : 'default'}
                    title={`${p.session}:${p.window}.${p.pane}`}
                    onclick={(e) => { e.stopPropagation(); pickPane(cell.id, p); }}
                  />
                {/each}
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .split-grid {
    display: grid;
    gap: 4px;
    height: 100%;
    width: 100%;
    padding: 4px;
    box-sizing: border-box;
    background: var(--bg);
  }
  .layout-2 { grid-template-columns: 1fr 1fr; }
  .layout-3 { grid-template-columns: 1fr 1fr 1fr; }
  .layout-4 { grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr; }
  .layout-6 { grid-template-columns: 1fr 1fr 1fr; grid-template-rows: 1fr 1fr; }

  .cell {
    display: flex;
    flex-direction: column;
    /* min-* are essential: grid children otherwise refuse to shrink and
       xterm's ResizeObserver never sees the real (smaller) box. */
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    position: relative;
    background: var(--bg);
  }
  .cell.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent), 0 0 12px var(--accent-glow);
  }

  .cell-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border2);
    background: var(--surface);
    flex-shrink: 0;
  }
  .cell-spacer { flex: 1; min-width: 0; }
  .cell-close {
    flex-shrink: 0;
    width: 22px; height: 22px;
    padding: 0; border: none; border-radius: 6px;
    background: transparent; color: var(--text3);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .cell-close:hover { color: var(--danger); background: var(--surface2); }

  .cell-body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    position: relative;
  }

  .cell-empty {
    width: 100%; height: 100%;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 8px;
    border: none; background: transparent;
    color: var(--text3); font-size: 13px;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .cell-empty:hover { color: var(--accent); }

  /* Pane picker popover */
  .picker-backdrop {
    position: fixed; inset: 0; z-index: 30;
  }
  .picker {
    position: absolute;
    top: 36px; left: 6px;
    z-index: 31;
    max-width: calc(100% - 12px);
    max-height: 60%;
    overflow-y: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.4);
    padding: 6px;
  }
  .picker-session {
    font-size: 10px; font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px;
    padding: 6px 6px 2px;
  }
  .picker-panes {
    display: flex; flex-wrap: wrap; gap: 4px;
    padding: 0 4px 6px;
  }
  .picker-empty {
    padding: 16px; text-align: center; color: var(--text3); font-size: 13px;
  }
</style>
