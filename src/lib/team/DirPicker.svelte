<script lang="ts">
  // Inline directory browser, extracted from the Sessions new-session picker so
  // the Team "new team" workspace field gets the same folder-browse UX.
  // Breadcrumb path + up button + folder list; choosing a folder navigates into
  // it, the ✓ confirms the current directory. Calls onPick(path) on confirm.
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { fsList, fsMkdir } from '../core/ws.ts';

  type DirEntry = { name: string; type: string; path: string };

  let {
    start = '~',           // initial directory to open at
    onPick = () => {},     // confirmed directory (✓ button)
    onNavigate = () => {}, // fires on every browse step, so the caller's field
                           // tracks where the picker is before the user confirms
    onClose = () => {},
  }: {
    start?: string;
    onPick?: (path: string) => void;
    onNavigate?: (path: string) => void;
    onClose?: () => void;
  } = $props();

  let path = $state('');
  let entries = $state<DirEntry[]>([]);
  let pathEl = $state<HTMLElement | null>(null);
  // New-folder inline input.
  let creating = $state(false);
  let newName = $state('');
  let createErr = $state('');

  async function createFolder() {
    const name = newName.trim();
    if (!name) return;
    const dir = `${path.replace(/\/$/, '')}/${name}`;
    try {
      await fsMkdir(dir);
      creating = false;
      newName = '';
      createErr = '';
      await load(dir); // navigate into the new folder → it becomes the selection
    } catch (e) {
      createErr = (e as Error)?.message || 'failed';
    }
  }

  async function load(p: string) {
    try {
      const r = await fsList(p, false);
      path = r.path || p;   // server resolves ~/relative → absolute
      entries = (r.entries || []).filter((e: DirEntry) => e.type === 'dir').sort((a: DirEntry, b: DirEntry) => a.name.localeCompare(b.name));
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
    <button class="picker-btn" class:on={creating} onclick={() => { creating = !creating; createErr = ''; }} aria-label={t('newFolder')} title={t('newFolder')}><Icon name="folder-plus" size={13} /></button>
    <button class="picker-btn pick-ok" onclick={() => onPick(path)} aria-label="Select"><Icon name="check" size={13} /></button>
  </div>
  {#if creating}
    <div class="picker-new">
      <Icon name="folder-plus" size={13} />
      <input class="picker-new-input" bind:value={newName} placeholder={t('newFolderName')}
        autocapitalize="off" autocomplete="off"
        onkeydown={(e) => { if (e.key === 'Enter') createFolder(); else if (e.key === 'Escape') { creating = false; newName = ''; } }} />
      <button class="picker-btn pick-ok" onclick={createFolder} disabled={!newName.trim()} aria-label={t('create')}><Icon name="check" size={13} /></button>
    </div>
    {#if createErr}<div class="picker-err">{createErr}</div>{/if}
  {/if}
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
  .picker-btn.on { background: var(--accent-bg); color: var(--accent); }
  .pick-ok { background: var(--accent-bg); color: var(--accent); }
  .pick-ok:disabled { opacity: 0.4; cursor: default; }
  .picker-new {
    display: flex; align-items: center; gap: 6px;
    padding: 6px 8px; border-bottom: 1px solid var(--border2); color: var(--text3);
  }
  .picker-new-input {
    flex: 1; min-width: 0; padding: 5px 8px;
    border: 1px solid var(--input-border); border-radius: 6px;
    background: var(--bg); color: var(--text); font-size: 12px;
    font-family: var(--font-mono); outline: none;
  }
  .picker-new-input:focus { border-color: var(--accent); }
  .picker-err { padding: 4px 10px; color: var(--danger); font-size: 11px; border-bottom: 1px solid var(--border2); }
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
