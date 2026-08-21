<script>
  // THE "New Project" surface — one dialog, wherever a project can be born.
  // The Chat sidebar and the Terminal sidebar used to grow their own (an
  // inline form with a second directory picker and raw backend presets vs.
  // this dialog), which read as two different apps (owner: "所有页面像是统一
  // 设计出来的"). The dialog is the survivor because it already carries the
  // required-name rule and the registry agent picks that seed real slots.
  //
  // Self-contained on purpose: it loads the registry itself and runs the whole
  // create → up → spawn orchestration (each step an observable RPC), then
  // hands the created project to the caller, whose only job is navigation.
  import Icon from '../ui/Icon.svelte';
  import DirPicker from '../files/DirPicker.svelte';
  import { projectCreate, projectUp, hubSpawn, registryList } from '../core/ws.ts';
  import { backendColor } from '../hub/hub.ts';
  import { backendIcon } from '../core/agents.ts';
  import { t } from '../core/i18n.svelte.ts';

  let { compact = false, oncreated, oncancel } = $props();

  let name = $state('');
  let path = $state('');
  let agents = $state([]);          // selected registry names
  let registry = $state([]);
  let pickerOpen = $state(false);
  let creating = $state(false);
  let error = $state('');

  $effect(() => {
    registryList().then((r) => { registry = r.agents ?? []; }).catch(() => {});
  });

  function toggle(n) {
    agents = agents.includes(n) ? agents.filter((x) => x !== n) : [...agents, n];
  }

  async function create() {
    const p = path.trim();
    const n = name.trim();
    // The NAME names the project AND its session; the folder name is what was
    // wrong when this was optional (projects called "src-tauri").
    if (!p || !n || creating) return;
    creating = true;
    error = '';
    try {
      const r = await projectCreate(p, { name: n, session: n });
      const proj = r.project ?? r;
      await projectUp(proj.id);
      for (const a of agents) {
        try { await hubSpawn(proj.session, a); } catch (e) { console.warn('spawn failed', a, e); }
      }
      oncreated?.(proj);
    } catch (e) {
      error = e.message ?? String(e);
    } finally {
      creating = false;
    }
  }
</script>

<div class="dlg-backdrop" onclick={() => oncancel?.()} role="presentation"></div>
<div class="dlg" class:sheet={compact}>
  <h2>{t('projectNew')}</h2>
  {#if error}<p class="err">{error}</p>{/if}
  {#if pickerOpen}
    <DirPicker start={path.trim() || '~'}
      onpick={(p) => { path = p; pickerOpen = false; }}
      oncancel={() => pickerOpen = false} />
  {:else}
    <input placeholder={t('hubCreateName')} bind:value={name} />
    <div class="path-row">
      <input placeholder={t('hubCreatePath')} bind:value={path} />
      <button class="chip-btn" onclick={() => pickerOpen = true}>
        <Icon name="folder" size={13} />{t('dirPickerOpen')}
      </button>
    </div>
    <div class="dlg-h">{t('hubCreateAgents')}</div>
    <div class="dlg-agents">
      {#each registry as r (r.name)}
        <button class="agent-pick" class:sel={agents.includes(r.name)} onclick={() => toggle(r.name)}>
          {#if backendIcon(r.backend)}<img class="ava" src={backendIcon(r.backend)} alt={r.backend} />{:else}<span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>{/if}
          {r.name} · {r.backend}
          {#if agents.includes(r.name)}<Icon name="check" size={13} />{/if}
        </button>
      {/each}
      {#if !registry.length}
        <p class="dlg-note">{t('hubCreateNoRegistry')}</p>
      {/if}
    </div>
    <div class="dlg-actions">
      <button class="chip-btn" onclick={() => oncancel?.()}>{t('cancel')}</button>
      <button class="chip-btn primary" disabled={!path.trim() || !name.trim() || creating} onclick={create}>
        {creating ? '…' : t('hubCreateGo')}
      </button>
    </div>
  {/if}
</div>

<style>
  .dlg-backdrop { position: fixed; inset: 0; z-index: 30; background: rgba(0,0,0,0.45); }
  .dlg {
    position: fixed; z-index: 31; top: 50%; left: 50%; transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 32px)); max-height: min(80vh, 640px); overflow-y: auto;
    background: var(--bg2); border: 1px solid var(--border); border-radius: 18px;
    padding: 18px; display: flex; flex-direction: column; gap: 9px;
    box-shadow: 0 18px 48px rgba(0,0,0,0.35);
  }
  .dlg h2 { margin: 0 0 4px; font-size: var(--fs-title); }
  /* Phone: the dialog becomes a bottom sheet — reachable with a thumb. */
  .dlg.sheet {
    top: auto; left: 0; right: 0; bottom: 0; transform: none; width: auto;
    border-radius: 18px 18px 0 0; padding-bottom: calc(18px + env(safe-area-inset-bottom));
  }
  .dlg.sheet .dlg-agents { max-height: 46vh; overflow-y: auto; }
  .dlg.sheet .agent-pick, .dlg.sheet input, .dlg.sheet .dlg-actions button { min-height: 44px; }
  .dlg input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: var(--ui-radius-control); color: var(--text); padding: 8px 12px; font-size: var(--fs-ui); outline: none; }
  .dlg input:focus { border-color: var(--accent); }
  .dlg-h { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-meta); text-transform: uppercase; letter-spacing: 1.4px; color: var(--text3); margin-top: 4px; }
  .dlg-agents { display: flex; flex-direction: column; gap: 5px; }
  .dlg-note { margin: 0; color: var(--text3); font-size: var(--fs-ui); }
  .err { margin: 0; color: var(--danger); font-size: var(--fs-ui); }
  .path-row { display: flex; gap: 6px; align-items: stretch; }
  .path-row input { flex: 1; min-width: 0; }
  .path-row :global(.chip-btn) { flex: none; }
  .agent-pick { display: flex; align-items: center; gap: 8px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--ui-radius-control); color: var(--text2); padding: 8px 11px; font-size: var(--fs-ui); cursor: pointer; text-align: left; }
  .agent-pick.sel { border-color: var(--accent-line); background: var(--accent-bg); color: var(--text); }
  .agent-pick :global(svg) { margin-left: auto; color: var(--accent); }
  .ava { width: 20px; height: 20px; border-radius: 6px; display: grid; place-items: center; color: white; font-size: var(--fs-meta); font-weight: 700; flex: none; }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
</style>
