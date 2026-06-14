<script>
  // Team roster template editor (modal). Lists named templates; the selected
  // one's agents are editable in full (name, backend, role, goal, backstory,
  // model, manage). Add/remove agents, add/rename/delete templates, save.
  // Persists via onSave(name, agents) / onDelete(name) which call the team_*
  // template RPCs and re-fetch.
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';

  let {
    templates = [],        // [{ name, agents:[…] }]
    onSave = () => {},     // (name, agents)
    onDelete = () => {},   // (name)
    onClose = () => {},
  } = $props();

  const BACKENDS = ['kiro', 'claude', 'codex'];

  // Local editable copy so edits aren't lost on the 1s status poll re-render.
  let drafts = $state(structuredClone(templates));
  let selIdx = $state(0);
  let dirty = $state(false);
  let saving = $state(false);

  let sel = $derived(drafts[selIdx] || null);

  function markDirty() { dirty = true; }

  function addAgent() {
    if (!sel) return;
    sel.agents = [...sel.agents, {
      name: `agent${sel.agents.length + 1}`, backend: 'kiro',
      role: '', goal: '', backstory: '', model: '', manage: false,
    }];
    markDirty();
  }
  function removeAgent(i) {
    if (!sel) return;
    sel.agents = sel.agents.filter((_, idx) => idx !== i);
    markDirty();
  }

  function addTemplate() {
    // Unique default name.
    let n = 1, name = 'team-a';
    const taken = new Set(drafts.map(d => d.name));
    const letters = 'abcdefghijklmnop';
    while (taken.has(name)) { name = `team-${letters[n] || n}`; n++; }
    drafts = [...drafts, { name, agents: [] }];
    selIdx = drafts.length - 1;
    dirty = true;
  }

  async function saveCurrent() {
    if (!sel || saving) return;
    saving = true;
    try {
      await onSave(sel.name, sel.agents);
      dirty = false;
    } catch {} finally { saving = false; }
  }

  async function deleteCurrent() {
    if (!sel) return;
    const name = sel.name;
    if (name === 'default') return; // protected
    try {
      await onDelete(name);
      drafts = drafts.filter(d => d.name !== name);
      selIdx = Math.max(0, selIdx - 1);
    } catch {}
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="tpl-overlay" onclick={onClose}></div>
<div class="tpl-modal">
  <div class="tpl-head">
    <span class="tpl-title">{t('teamTemplates')}</span>
    <button class="tpl-x" onclick={onClose} aria-label={t('close')}><Icon name="x" size={15} /></button>
  </div>

  <div class="tpl-body">
    <!-- Left: template list -->
    <div class="tpl-list">
      {#each drafts as d, i}
        <button class="tpl-item" class:active={i === selIdx} onclick={() => selIdx = i}>
          {d.name} <span class="tpl-count">{d.agents?.length ?? 0}</span>
        </button>
      {/each}
      <button class="tpl-add" onclick={addTemplate}><Icon name="plus" size={12} /> {t('teamNewTemplate')}</button>
    </div>

    <!-- Right: selected template's agents -->
    <div class="tpl-edit">
      {#if sel}
        <div class="tpl-name-row">
          <input class="tpl-name-input" bind:value={sel.name} oninput={markDirty}
            disabled={sel.name === 'default'} placeholder="template name" />
        </div>

        {#each sel.agents as ag, i}
          <div class="agent-card">
            <div class="agent-card-head">
              <input class="ag-field ag-name" bind:value={ag.name} oninput={markDirty} placeholder="name" />
              <select class="ag-field ag-backend" bind:value={ag.backend} onchange={markDirty}>
                {#each BACKENDS as b}<option value={b}>{b}</option>{/each}
              </select>
              <label class="ag-manage" title="manager (can hire/fire)">
                <input type="checkbox" bind:checked={ag.manage} onchange={markDirty} /> mgr
              </label>
              <button class="ag-del" onclick={() => removeAgent(i)} aria-label={t('teamRemoveAgent')}><Icon name="trash" size={12} /></button>
            </div>
            <input class="ag-field" bind:value={ag.role} oninput={markDirty} placeholder={t('teamRole')} />
            <textarea class="ag-field ag-area" bind:value={ag.goal} oninput={markDirty} placeholder={t('teamGoal')} rows="2"></textarea>
            <textarea class="ag-field ag-area" bind:value={ag.backstory} oninput={markDirty} placeholder={t('teamBackstory')} rows="2"></textarea>
            <input class="ag-field" bind:value={ag.model} oninput={markDirty} placeholder={t('teamModel')} />
          </div>
        {/each}

        <button class="agent-add" onclick={addAgent}><Icon name="plus" size={13} /> {t('teamAddAgent')}</button>
      {/if}
    </div>
  </div>

  <div class="tpl-foot">
    {#if sel && sel.name !== 'default'}
      <button class="tpl-delete" onclick={deleteCurrent}>{t('teamDeleteTemplate')}</button>
    {/if}
    <div class="tpl-foot-right">
      <button class="tpl-cancel" onclick={onClose}>{t('close')}</button>
      <button class="tpl-save" disabled={!dirty || saving} onclick={saveCurrent}>
        {#if saving}<span class="tpl-spin"></span>{/if}{t('teamSaveTemplate')}
      </button>
    </div>
  </div>
</div>

<style>
  .tpl-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.45); z-index: 40; border: none; }
  .tpl-modal {
    position: fixed; z-index: 41;
    top: 50%; left: 50%; transform: translate(-50%, -50%);
    width: min(720px, 94vw); height: min(80vh, 720px);
    display: flex; flex-direction: column;
    background: var(--bg); border: 1px solid var(--border); border-radius: 14px;
    box-shadow: 0 20px 60px rgba(0,0,0,0.5); overflow: hidden;
  }
  .tpl-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 14px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .tpl-title { font-size: 14px; font-weight: 600; color: var(--text); }
  .tpl-x { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; }
  .tpl-x:active { color: var(--accent); }

  .tpl-body { flex: 1; min-height: 0; display: flex; }
  .tpl-list {
    width: 160px; flex-shrink: 0; border-right: 1px solid var(--border);
    padding: 8px; display: flex; flex-direction: column; gap: 4px; overflow-y: auto;
  }
  .tpl-item {
    display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding: 7px 9px; border: 1px solid var(--border2); border-radius: 8px;
    background: var(--input-bg); color: var(--text2); font-size: 12px; cursor: pointer;
    text-align: left; -webkit-tap-highlight-color: transparent;
    font-family: 'Maple Mono NF CN','Maple Mono','SF Mono',Menlo,monospace;
  }
  .tpl-item.active { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .tpl-count { color: var(--text3); font-size: 11px; }
  .tpl-add {
    padding: 7px 9px; border: 1px dashed var(--border2); border-radius: 8px;
    background: transparent; color: var(--text3); font-size: 12px; cursor: pointer;
    display: flex; align-items: center; gap: 5px; -webkit-tap-highlight-color: transparent;
  }
  .tpl-add:active { color: var(--accent); border-color: var(--accent); }

  .tpl-edit { flex: 1; min-width: 0; padding: 12px; overflow-y: auto; display: flex; flex-direction: column; gap: 10px; }
  .tpl-name-input {
    width: 100%; padding: 8px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: 14px; font-weight: 600; outline: none;
    font-family: 'Maple Mono NF CN','Maple Mono','SF Mono',Menlo,monospace;
  }
  .tpl-name-input:focus { border-color: var(--accent); }
  .tpl-name-input:disabled { opacity: 0.6; }

  .agent-card {
    display: flex; flex-direction: column; gap: 6px;
    padding: 10px; border: 1px solid var(--border2); border-radius: 10px; background: var(--surface);
  }
  .agent-card-head { display: flex; align-items: center; gap: 6px; }
  .ag-field {
    padding: 6px 9px; border: 1px solid var(--input-border); border-radius: 7px;
    background: var(--input-bg); color: var(--text); font-size: 12px; font-family: inherit; outline: none;
  }
  .ag-field:focus { border-color: var(--accent); }
  .ag-name { flex: 1; min-width: 0; font-weight: 600; }
  .ag-backend { flex-shrink: 0; }
  .ag-manage { display: flex; align-items: center; gap: 3px; font-size: 11px; color: var(--text3); white-space: nowrap; }
  .ag-del { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; flex-shrink: 0; padding: 4px; }
  .ag-del:active { color: var(--danger); }
  .ag-area { resize: vertical; line-height: 1.4; }

  .agent-add {
    padding: 9px; border: 1px dashed var(--border2); border-radius: 10px;
    background: transparent; color: var(--text3); font-size: 13px; cursor: pointer;
    display: flex; align-items: center; justify-content: center; gap: 6px;
    -webkit-tap-highlight-color: transparent;
  }
  .agent-add:active { color: var(--accent); border-color: var(--accent); }

  .tpl-foot {
    display: flex; align-items: center; justify-content: space-between;
    padding: 10px 14px; border-top: 1px solid var(--border); flex-shrink: 0;
  }
  .tpl-foot-right { display: flex; gap: 8px; margin-left: auto; }
  .tpl-delete {
    padding: 7px 12px; border: 1px solid var(--danger); border-radius: 8px;
    background: none; color: var(--danger); font-size: 12px; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .tpl-cancel {
    padding: 7px 12px; border: 1px solid var(--border2); border-radius: 8px;
    background: none; color: var(--text2); font-size: 12px; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .tpl-save {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 7px 14px; border: 1px solid var(--accent); border-radius: 8px;
    background: var(--accent-bg); color: var(--accent); font-size: 12px; font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .tpl-save:disabled { opacity: 0.5; cursor: default; }
  .tpl-spin {
    width: 11px; height: 11px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%; animation: tpl-spin 0.6s linear infinite;
  }
  @keyframes tpl-spin { to { transform: rotate(360deg); } }
</style>
