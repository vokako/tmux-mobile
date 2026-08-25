<script>
  // A directory picker over the SAME `fs_list` RPC the file browser uses —
  // typing an absolute path from memory was the only way to start a project
  // (owner: "路径选择复用下文件选择器"). Deliberately directories-only and
  // read-only: this is a chooser, not a second file manager, so it does not
  // grow preview/edit/upload. Callers get an absolute path back.
  import Icon from '../ui/Icon.svelte';
  import { fsList } from '../core/ws.ts';
  import { t } from '../core/i18n.svelte.ts';

  let { start = '~', onpick, oncancel } = $props();

  let cwd = $state('');
  let dirs = $state([]);
  let loading = $state(false);
  let error = $state('');

  async function open(path) {
    loading = true;
    error = '';
    try {
      const r = await fsList(path, false);
      cwd = r.path ?? path;
      dirs = (r.entries ?? []).filter((e) => e.type === 'dir');
    } catch (e) {
      error = e.message ?? String(e);
    }
    loading = false;
  }

  // The parent of an absolute path; '/' is its own parent (no climb past root).
  function parentOf(path) {
    const trimmed = path.replace(/\/+$/, '');
    const cut = trimmed.lastIndexOf('/');
    if (cut <= 0) return '/';
    return trimmed.slice(0, cut);
  }

  $effect(() => { open(start); });
</script>

<div class="picker">
  <div class="pk-head">
    <button class="icon-btn" title={t('dirPickerUp')} disabled={cwd === '/'} onclick={() => open(parentOf(cwd))}>
      <Icon name="chevron-up" size={13} />
    </button>
    <span class="pk-path" title={cwd}>{cwd || '…'}</span>
    <button class="icon-btn" title={t('dirPickerHome')} onclick={() => open('~')}><Icon name="home" size={13} /></button>
  </div>

  {#if error}
    <div class="pk-err">{error}</div>
  {/if}

  <div class="pk-list subtle-scroll">
    {#if loading}
      <div class="pk-empty">…</div>
    {:else if !dirs.length}
      <div class="pk-empty">{t('dirPickerEmpty')}</div>
    {:else}
      {#each dirs as d (d.path)}
        <button class="pk-row" onclick={() => open(d.path)}>
          <Icon name="folder" size={13} />
          <span class="pk-name">{d.name}</span>
          <Icon name="chevron-right" size={12} />
        </button>
      {/each}
    {/if}
  </div>

  <div class="pk-foot">
    <button class="chip-btn" onclick={() => oncancel?.()}>{t('cancel')}</button>
    <button class="chip-btn primary" disabled={!cwd} onclick={() => onpick?.(cwd)}>{t('dirPickerUse')}</button>
  </div>
</div>

<style>
  .picker { display: flex; flex-direction: column; min-height: 0; gap: 8px; }
  .pk-head { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .pk-path {
    flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--fs-sub);
    color: var(--text2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    direction: rtl; text-align: left;   /* keep the TAIL of a long path visible */
  }
  .pk-err { color: var(--danger); font-size: var(--fs-meta); }
  .pk-list {
    flex: 1; min-height: 140px; max-height: calc(42vh / var(--ui-zoom, 1)); overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--ui-radius-control); background: var(--input-bg);
  }
  .pk-row {
    display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
    /* 40px rows: this is a phone-first list of tap targets. */
    min-height: 40px; padding: 6px 10px; border: none; border-bottom: 1px solid var(--border2);
    background: none; color: var(--text2); font-size: var(--fs-ui); cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .pk-row:last-child { border-bottom: none; }
  .pk-row:hover { color: var(--accent); }
  .pk-name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pk-empty { padding: 16px; text-align: center; color: var(--text3); font-size: var(--fs-ui); }
  .pk-foot { display: flex; justify-content: flex-end; gap: 6px; }
</style>
