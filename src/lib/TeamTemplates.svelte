<script>
  // Team roster template editor (modal). Lists named templates; the selected
  // one's agents are editable in full (name, backend, role, goal, model,
  // manage). Add/remove agents, add/rename/delete templates, save.
  // Persists via onSave(name, agents) / onDelete(name) which call the team_*
  // template RPCs and re-fetch.
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';

  let {
    templates = [],        // [{ name, agents:[…] }]
    systemPrompt = '',     // global system prompt (shared across all teams)
    onSave = () => {},     // (name, agents)
    onDelete = () => {},   // (name)
    onSaveSystemPrompt = () => {}, // (text)
    onClose = () => {},
  } = $props();

  const BACKENDS = ['kiro', 'claude', 'codex'];

  // Local editable copy so edits aren't lost on the 1s status poll re-render.
  // NOTE: `templates` is a Svelte $state proxy — structuredClone() throws a
  // DataCloneError on proxies, so deep-clone via JSON instead.
  let drafts = $state(JSON.parse(JSON.stringify(templates ?? [])));
  let selIdx = $state(0);
  let dirty = $state(false);
  let saving = $state(false);
  // Global system prompt (editable; saved separately from templates).
  let sysDraft = $state(systemPrompt);
  let sysDirty = $state(false);
  let sysSaving = $state(false);

  async function saveSystem() {
    if (sysSaving) return;
    sysSaving = true;
    try { await onSaveSystemPrompt(sysDraft); sysDirty = false; }
    catch {} finally { sysSaving = false; }
  }

  let sel = $derived(drafts[selIdx] || null);

  function markDirty() { dirty = true; }

  function addAgent() {
    if (!sel) return;
    sel.agents = [...sel.agents, {
      name: `agent${sel.agents.length + 1}`, backend: 'kiro',
      role: '', goal: '', model: '', manage: false,
    }];
    markDirty();
  }
  function removeAgent(i) {
    if (!sel) return;
    // Tap-to-confirm (like killing a session): first tap arms, second removes.
    if (confirmRmAgent !== i) {
      confirmRmAgent = i;
      setTimeout(() => { if (confirmRmAgent === i) confirmRmAgent = null; }, 3000);
      return;
    }
    confirmRmAgent = null;
    sel.agents = sel.agents.filter((_, idx) => idx !== i);
    markDirty();
  }
  let confirmRmAgent = $state(null); // agent index armed for deletion
  let confirmDelTpl = $state(false); // template delete armed
  let teamWideOpen = $state(false);  // team-wide config section expanded

  // Per-agent "advanced" (env / mcp / skills) expand state, keyed by index;
  // reset when switching templates.
  let advSet = $state(new Set());
  function toggleAdv(i) {
    const s = new Set(advSet);
    s.has(i) ? s.delete(i) : s.add(i);
    advSet = s;
  }
  $effect(() => { selIdx; advSet = new Set(); confirmRmAgent = null; confirmDelTpl = false; teamWideOpen = false; });

  // env (object) ⇄ KEY=VALUE lines; skills (array) ⇄ one-per-line.
  const envToLines = (o) => (o && typeof o === 'object')
    ? Object.entries(o).map(([k, v]) => `${k}=${v}`).join('\n') : '';
  function setEnv(ag, s) {
    const o = {};
    for (const line of s.split('\n')) {
      const t = line.trim();
      if (!t) continue;
      const i = t.indexOf('=');
      if (i > 0) o[t.slice(0, i).trim()] = t.slice(i + 1).trim();
    }
    ag.env = o; markDirty();
  }
  const arrToLines = (a) => Array.isArray(a) ? a.join('\n') : '';
  function setSkills(ag, s) {
    ag.skills = s.split('\n').map(x => x.trim()).filter(Boolean);
    markDirty();
  }
  // mcp is structured → edit as JSON; parse on change, ignore invalid input.
  const mcpToText = (a) => (Array.isArray(a) && a.length) ? JSON.stringify(a, null, 2) : '';
  let mcpErr = $state({});
  function setMcp(ag, i, s) {
    if (!s.trim()) { ag.mcp = []; mcpErr = { ...mcpErr, [i]: false }; markDirty(); return; }
    try {
      const v = JSON.parse(s);
      if (!Array.isArray(v)) throw new Error('not an array');
      ag.mcp = v; mcpErr = { ...mcpErr, [i]: false }; markDirty();
    } catch {
      mcpErr = { ...mcpErr, [i]: true };
    }
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
      await onSave(sel.name, {
        env: sel.env ?? {},
        mcp: sel.mcp ?? [],
        skills: sel.skills ?? [],
        prompt: sel.prompt ?? '',
        agents: sel.agents,
      });
      dirty = false;
    } catch {} finally { saving = false; }
  }

  async function deleteCurrent() {
    if (!sel) return;
    const name = sel.name;
    if (name === 'default') return; // protected
    // Tap-to-confirm: first click arms the button, second click deletes.
    if (!confirmDelTpl) {
      confirmDelTpl = true;
      setTimeout(() => { confirmDelTpl = false; }, 3000);
      return;
    }
    confirmDelTpl = false;
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

  <!-- Global system prompt: prepended to EVERY agent's brief, across all teams. -->
  <div class="sys-section">
    <div class="sys-label-row">
      <span class="sys-label">{t('teamSystemPrompt')}</span>
      <button class="sys-save" disabled={!sysDirty || sysSaving} onclick={saveSystem}>
        {#if sysSaving}<span class="tpl-spin"></span>{/if}{t('teamSaveTemplate')}
      </button>
    </div>
    <textarea class="sys-input" bind:value={sysDraft} oninput={() => sysDirty = true}
      placeholder={t('teamSystemPromptHint')} rows="2"></textarea>
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

        <!-- Team-wide config: applies to EVERY agent in this team. -->
        <button class="tw-toggle" onclick={() => teamWideOpen = !teamWideOpen}>
          <Icon name={teamWideOpen ? 'chevron-down' : 'chevron-right'} size={12} />
          {t('teamWide')}
          {#if (sel.mcp?.length || sel.skills?.length || (sel.env && Object.keys(sel.env).length) || (sel.prompt && sel.prompt.trim()))}
            <span class="ag-adv-badge">●</span>
          {/if}
        </button>
        {#if teamWideOpen}
          <div class="ag-adv tw-adv">
            <label class="ag-adv-label">{t('teamWidePrompt')}</label>
            <textarea class="ag-field ag-area" rows="3" placeholder={t('teamWidePromptHint')}
              value={sel.prompt ?? ''} oninput={(e) => { sel.prompt = e.target.value; markDirty(); }}></textarea>
            <label class="ag-adv-label">{t('teamSkills')}</label>
            <textarea class="ag-field ag-area" rows="2" placeholder={t('teamSkillsHint')}
              value={arrToLines(sel.skills)} oninput={(e) => setSkills(sel, e.target.value)}></textarea>
            <label class="ag-adv-label">{t('teamEnv')}</label>
            <textarea class="ag-field ag-area" rows="2" placeholder="KEY=VALUE"
              value={envToLines(sel.env)} oninput={(e) => setEnv(sel, e.target.value)}></textarea>
            <label class="ag-adv-label">{t('teamMcp')} {#if mcpErr['__team__']}<span class="ag-adv-err">{t('teamMcpBad')}</span>{/if}</label>
            <textarea class="ag-field ag-area ag-mono" rows="4" placeholder={t('teamMcpHint')}
              value={mcpToText(sel.mcp)} onchange={(e) => setMcp(sel, '__team__', e.target.value)}></textarea>
          </div>
        {/if}

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
              <button class="ag-del" class:confirm={confirmRmAgent === i} onclick={() => removeAgent(i)}
                title={confirmRmAgent === i ? t('teamConfirmDelete') : t('teamRemoveAgent')}
                aria-label={t('teamRemoveAgent')}><Icon name="trash" size={12} /></button>
            </div>
            <input class="ag-field" bind:value={ag.role} oninput={markDirty} placeholder={t('teamRole')} />
            <textarea class="ag-field ag-area" bind:value={ag.goal} oninput={markDirty} placeholder={t('teamGoal')} rows="3"></textarea>
            <input class="ag-field" bind:value={ag.model} oninput={markDirty} placeholder={t('teamModel')} />

            <button class="ag-adv-toggle" onclick={() => toggleAdv(i)}>
              <Icon name={advSet.has(i) ? 'chevron-down' : 'chevron-right'} size={12} />
              {t('teamAdvanced')}
              {#if (ag.mcp?.length || ag.skills?.length || (ag.env && Object.keys(ag.env).length))}
                <span class="ag-adv-badge">{(ag.mcp?.length || 0) + (ag.skills?.length || 0) + (ag.env ? Object.keys(ag.env).length : 0)}</span>
              {/if}
            </button>
            {#if advSet.has(i)}
              <div class="ag-adv">
                <label class="ag-adv-label">{t('teamSkills')}</label>
                <textarea class="ag-field ag-area" rows="2" placeholder={t('teamSkillsHint')}
                  value={arrToLines(ag.skills)} oninput={(e) => setSkills(ag, e.target.value)}></textarea>
                <label class="ag-adv-label">{t('teamEnv')}</label>
                <textarea class="ag-field ag-area" rows="2" placeholder="KEY=VALUE"
                  value={envToLines(ag.env)} oninput={(e) => setEnv(ag, e.target.value)}></textarea>
                <label class="ag-adv-label">{t('teamMcp')} {#if mcpErr[i]}<span class="ag-adv-err">{t('teamMcpBad')}</span>{/if}</label>
                <textarea class="ag-field ag-area ag-mono" rows="4" placeholder={t('teamMcpHint')}
                  value={mcpToText(ag.mcp)} onchange={(e) => setMcp(ag, i, e.target.value)}></textarea>
              </div>
            {/if}
          </div>
        {/each}

        <button class="agent-add" onclick={addAgent}><Icon name="plus" size={13} /> {t('teamAddAgent')}</button>
      {/if}
    </div>
  </div>

  <div class="tpl-foot">
    {#if sel && sel.name !== 'default'}
      <button class="tpl-delete" class:confirm={confirmDelTpl} onclick={deleteCurrent}>
        {confirmDelTpl ? t('teamConfirmDelete') : t('teamDeleteTemplate')}
      </button>
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
    box-sizing: border-box;
  }
  /* Phone: near-fullscreen sheet, list becomes a top strip, fields stack. */
  @media (max-width: 620px) {
    .tpl-modal {
      width: 100vw; height: 100%; height: var(--app-height, 100dvh);
      top: 0; left: 0; transform: none; border-radius: 0; border: none;
      padding-top: var(--sat); padding-bottom: var(--sab);
    }
    .tpl-body { flex-direction: column; }
    .tpl-list {
      width: auto; flex-direction: row; overflow-x: auto; overflow-y: hidden;
      border-right: none; border-bottom: 1px solid var(--border);
      scrollbar-width: none;
    }
    .tpl-list::-webkit-scrollbar { display: none; }
    .tpl-item, .tpl-add { flex-shrink: 0; white-space: nowrap; }
  }
  .tpl-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 14px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .tpl-title { font-size: 14px; font-weight: 600; color: var(--text); }
  .tpl-x { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; }
  .tpl-x:active { color: var(--accent); }

  .sys-section {
    display: flex; flex-direction: column; gap: 6px;
    padding: 10px 14px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .sys-label-row { display: flex; align-items: center; justify-content: space-between; }
  .sys-label {
    font-size: 10px; font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px;
  }
  .sys-save {
    display: inline-flex; align-items: center; gap: 5px;
    padding: 4px 10px; border: 1px solid var(--accent); border-radius: 7px;
    background: var(--accent-bg); color: var(--accent); font-size: 11px; font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .sys-save:disabled { opacity: 0.45; cursor: default; }
  .sys-input {
    width: 100%; box-sizing: border-box; max-height: 120px;
    padding: 8px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: 12px;
    font-family: inherit; resize: vertical; line-height: 1.4; outline: none;
  }
  .sys-input:focus { border-color: var(--accent); }

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
    font-family: var(--font-ui);
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
    font-family: var(--font-ui);
  }
  .tpl-name-input:focus { border-color: var(--accent); }
  .tpl-name-input:disabled { opacity: 0.6; }

  .agent-card {
    display: flex; flex-direction: column; gap: 6px;
    padding: 10px; border: 1px solid var(--border2); border-radius: 10px; background: var(--surface);
  }
  .agent-card-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ag-field {
    padding: 6px 9px; border: 1px solid var(--input-border); border-radius: 7px;
    background: var(--input-bg); color: var(--text); font-size: 12px; font-family: inherit;
    outline: none; width: 100%; box-sizing: border-box;
  }
  .ag-field:focus { border-color: var(--accent); }
  .ag-name { flex: 1; min-width: 80px; width: auto; font-weight: 600; }
  .ag-backend { flex-shrink: 0; width: auto; }
  .ag-manage { display: flex; align-items: center; gap: 3px; font-size: 11px; color: var(--text3); white-space: nowrap; }
  .ag-del { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; flex-shrink: 0; padding: 4px; }
  .ag-del:active { color: var(--danger); }
  .ag-del.confirm { color: var(--danger); }
  .ag-area { resize: vertical; line-height: 1.4; }

  .ag-adv-toggle {
    display: flex; align-items: center; gap: 5px; align-self: flex-start;
    padding: 3px 6px; border: none; background: none; cursor: pointer;
    color: var(--text3); font-size: 11px; font-weight: 600;
    -webkit-tap-highlight-color: transparent; font-family: var(--font-ui);
  }
  .ag-adv-toggle:active { color: var(--accent); }
  .ag-adv-badge {
    min-width: 15px; padding: 0 4px; border-radius: 7px; background: var(--accent-bg);
    color: var(--accent); font-size: 10px; text-align: center;
  }
  .ag-adv { display: flex; flex-direction: column; gap: 5px; padding-left: 4px; border-left: 2px solid var(--border2); }
  .ag-adv-label { font-size: 10px; color: var(--text3); text-transform: uppercase; letter-spacing: 0.4px; }
  .ag-adv-err { color: var(--danger); text-transform: none; letter-spacing: 0; margin-left: 6px; }
  .ag-mono { font-family: var(--font-mono, monospace); font-size: 11px; }

  .tw-toggle {
    display: flex; align-items: center; gap: 5px; align-self: flex-start;
    padding: 5px 8px; border: 1px dashed var(--border2); border-radius: 8px;
    background: transparent; color: var(--text2); font-size: 12px; font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent; font-family: var(--font-ui);
  }
  .tw-toggle:active { color: var(--accent); border-color: var(--accent); }
  .tw-adv { padding: 8px; border: 1px solid var(--border2); border-radius: 10px; background: var(--surface); }

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
  .tpl-delete.confirm { background: var(--danger); color: #fff; }
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
