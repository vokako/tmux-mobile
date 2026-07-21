<script>
  // Desktop split-screen: tiles N independent Terminal instances in a CSS
  // grid, each bound to any session:window.pane. Mounted only when
  // App.svelte decides split is active (desktop + wide). Each cell owns its
  // own Terminal (own subscription, own xterm, own resize) — the ws.js
  // per-target listener registry is what lets them coexist.
  import Terminal from '../terminal/Terminal.svelte';
  import PanePicker from './PanePicker.svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';

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

  // Which cell's pane picker is open (null = none). PanePicker (shared with
  // Terminal's single-pane switcher) fetches + renders the list.
  let pickerCellId = $state(null);
  function openPicker(cellId) { pickerCellId = cellId; }
  function closePicker() { pickerCellId = null; }
  function pickPane(cellId, p) {
    onAssign(cellId, `${p.session}:${p.window}.${p.pane}`, p.session, p.current_command);
    closePicker();
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
      <div class="cell-body">
        {#if cell.target}
          <!-- The Terminal's own window-switcher bar IS the cell header
               (same form as the single-pane view). Its session badge opens
               the cross-session pane picker; onClose closes the cell. -->
          {#key cell.target}
            <Terminal
              target={cell.target}
              session={cell.session}
              command={cell.command}
              embedded={true}
              active={cell.id === activeCellId}
              {fontSize}
              onSwitchPane={(t2, cmd) => onAssign(cell.id, t2, t2.split(':')[0], cmd)}
              onPaneExit={() => onPaneExit(cell.id)}
              onClose={() => onCloseCell(cell.id)}
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
        <PanePicker
          currentTarget={cell.target}
          onPick={(p) => pickPane(cell.id, p)}
          onClose={closePicker}
        />
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
</style>
