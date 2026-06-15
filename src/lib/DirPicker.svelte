<script>
  // Inline directory browser, extracted from the Sessions new-session picker so
  // the Team "new team" workspace field gets the same folder-browse UX.
  // Breadcrumb path + up button + folder list; choosing a folder navigates into
  // it, the ✓ confirms the current directory. Calls onPick(path) on confirm.
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';
  import { fsList } from './ws.js';

  let {
    start = '~',           // initial directory to open at
    onPick = () => {},     // (path) — confirmed directory (✓ button)
    onNavigate = () => {}, // (path) — fires on every browse step, so the
                           // caller's field tracks where the picker is even
                           // before the user confirms with ✓
    onClose = () => {},
  } = $props();

  let path = $state('');
  let entries = $state([]);
  let pathEl = $state(null);

  async function load(p) {
    try {
      const r = await fsList(p, false);
      path = r.path || p;   // server resolves ~/relative → absolute
      entries = (r.entries || []).filter(e => e.type === 'dir').sort((a, b) => a.name.localeCompare(b.name));
      onNavigate(path);
    } catch {}
  }
  function up() {
    const parent = path.replace(/\/[^/]+\/?$/, '') || '/';
    load(parent);
  }
  let breadcrumbs = $derived.by(() => {
    if (!path) return [];
    const parts = path.split('/').filter(Boolean);
    return parts.map((name, i) => ({ name, path: '/' + parts.slice(0, i + 1).join('/') }));
  });

  // Keep the breadcrumb scrolled to its tail (most-specific segment visible).
  $effect(() => {
    path;
    setTimeout(() => { if (pathEl) pathEl.scrollLeft = pathEl.scrollWidth; }, 0);
  });

  $effect(() => { load(start || '~'); });
</script>

<div class="picker">
  <div class="picker-header">
    <button class="picker-btn" onclick={up} aria-label="Up"><Icon name="folder-up" size={13} /></button>
    <div class="picker-path" bind:this={pathEl}>
      <button class="picker-seg" onclick={() => load('/')}>/</button>
      {#each breadcrumbs as bc}
        <button class="picker-seg" onclick={() => load(bc.path)}>{bc.name}</button>
        <span class="picker-sep">/</span>
      {/each}
    </div>
    <button class="picker-btn pick-ok" onclick={() => onPick(path)} aria-label="Select"><Icon name="check" size={13} /></button>
  </div>
  <div class="picker-list">
    {#each entries as e}
      <button class="picker-item" onclick={() => load(e.path)}>
        <Icon name="folder" size={13} /> {e.name}
      </button>
    {/each}
    {#if !entries.length}
      <div class="picker-empty">{t('noSubdirs')}</div>
    {/if}
  </div>
</div>

<style>
  .picker {
    border: 1px solid var(--border2);
    border-radius: 10px;
    overflow: hidden;
    background: var(--input-bg);
  }
  .picker-header {
    display: flex; align-items: center; gap: 6px;
    padding: 6px 8px; border-bottom: 1px solid var(--border2);
  }
  .picker-path {
    flex: 1; display: flex; align-items: center; gap: 1px;
    overflow-x: auto; scrollbar-width: none;
    font-family: var(--font-mono);
    font-size: 12px; -webkit-overflow-scrolling: touch;
  }
  .picker-path::-webkit-scrollbar { display: none; }
  .picker-seg {
    padding: 2px 3px; border: none; background: none; color: var(--text2);
    cursor: pointer; white-space: nowrap; font-size: 12px; font-family: inherit;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-seg:last-of-type { color: var(--accent); }
  .picker-seg:active { color: var(--accent); }
  .picker-sep { color: var(--text3); font-size: 11px; }
  .picker-btn {
    padding: 5px; border: none; border-radius: 6px;
    background: var(--surface2); color: var(--text2);
    cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .picker-btn:active { color: var(--accent); }
  .pick-ok { background: var(--accent-bg); color: var(--accent); }
  .picker-list { max-height: 180px; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .picker-item {
    display: flex; align-items: center; gap: 8px; width: 100%;
    padding: 10px 12px; border: none; border-bottom: 1px solid var(--border2);
    background: none; color: var(--accent); font-size: 13px; cursor: pointer;
    text-align: left; -webkit-tap-highlight-color: transparent;
  }
  .picker-item:active { background: var(--accent-bg); }
  .picker-item:last-child { border-bottom: none; }
  .picker-empty { padding: 12px; text-align: center; color: var(--text3); font-size: 12px; }
</style>
