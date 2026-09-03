<script lang="ts">
  // Team roster template editor (modal). Lists named templates; the selected
  // one's agents are editable in full (name, backend, role, goal, model,
  // manage). Add/remove agents, add/rename/delete templates, save.
  // Persists via onSave(name, agents) / onDelete(name) which call the team_*
  // template RPCs and re-fetch.
  import Icon from '../ui/Icon.svelte';
  import Select from '../ui/Select.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import { t } from '../core/i18n.svelte.ts';

  export interface TplAgent {
    name: string;
    backend: string;
    role: string;
    goal: string;
    model: string;
    manage: boolean;
    env?: Record<string, string>;
    mcp?: unknown[];
    skills?: string[];
  }
  export interface Template {
    name: string;
    agents: TplAgent[];
    env?: Record<string, string>;
    mcp?: unknown[];
    skills?: string[];
    prompt?: string;
  }
  type TemplateDef = Pick<Template, 'env' | 'mcp' | 'skills' | 'prompt'> & { agents: TplAgent[] };

  let {
    templates = [],
    systemPrompt = '',     // global system prompt (shared across all teams)
    onSave = () => {},
    onDelete = () => {},
    onSaveSystemPrompt = () => {},
    onClose = () => {},
  }: {
    templates?: Template[];
    systemPrompt?: string;
    onSave?: (name: string, def: TemplateDef) => void | Promise<void>;
    onDelete?: (name: string) => void | Promise<void>;
    onSaveSystemPrompt?: (text: string) => void | Promise<void>;
    onClose?: () => void;
  } = $props();

  const BACKENDS = ['kiro', 'claude', 'codex'];

  // Local editable copy so edits aren't lost on the 1s status poll re-render.
  // NOTE: `templates` is a Svelte $state proxy — structuredClone() throws a
  // DataCloneError on proxies, so deep-clone via JSON instead.
  // svelte-ignore state_referenced_locally — intentional: drafts is a local
  // editable copy seeded once; the status poll must NOT overwrite edits.
  let drafts = $state<Template[]>(JSON.parse(JSON.stringify(templates ?? [])));
  let selIdx = $state(0);
  let dirty = $state(false);
  let saving = $state(false);
  // Global system prompt (editable; saved separately from templates).
  // svelte-ignore state_referenced_locally — same seeding pattern as drafts.
  let sysDraft = $state(systemPrompt);
  let sysDirty = $state(false);
  let sysSaving = $state(false);
  let sysOpen = $state(false); // global system prompt collapsed by default (declutter, esp. mobile)

  async function saveSystem() {
    if (sysSaving) return;
    sysSaving = true;
    try { await onSaveSystemPrompt(sysDraft); sysDirty = false; }
    catch {} finally { sysSaving = false; }
  }

  let sel = $derived<Template | null>(drafts[selIdx] || null);

  function markDirty() { dirty = true; }

  function addAgent() {
    if (!sel) return;
    sel.agents = [...sel.agents, {
      name: `agent${sel.agents.length + 1}`, backend: 'kiro',
      role: '', goal: '', model: '', manage: false,
    }];
    markDirty();
  }
  function removeAgent(i: number) {
    if (!sel) return;
    sel.agents = sel.agents.filter((_, idx) => idx !== i);
    markDirty();
  }
  /** Awaiting confirmation: which roster row to drop, and whether the whole
   * template goes. Both were tap-to-confirm — a 3s window in which the same
   * button meant something else (owner audit, 2026-08-19). */
  let pendingRm = $state<{ i: number; name: string } | null>(null);
  let pendingDelTpl = $state(false);
  let teamWideOpen = $state(false);  // team-wide config section expanded

  // Per-agent "advanced" (env / mcp / skills) expand state, keyed by index;
  // reset when switching templates.
  let advSet = $state<Set<number>>(new Set());
  function toggleAdv(i: number) {
    const s = new Set(advSet);
    s.has(i) ? s.delete(i) : s.add(i);
    advSet = s;
  }
  $effect(() => { selIdx; advSet = new Set(); pendingRm = null; pendingDelTpl = false; teamWideOpen = false; });

  // env (object) ⇄ KEY=VALUE lines; skills (array) ⇄ one-per-line.
  const envToLines = (o: Record<string, string> | undefined) => (o && typeof o === 'object')
    ? Object.entries(o).map(([k, v]) => `${k}=${v}`).join('\n') : '';
  type ConfigHolder = { env?: Record<string, string>; mcp?: unknown[]; skills?: string[] };

  function setEnv(ag: ConfigHolder, s: string) {
    const o: Record<string, string> = {};
    for (const line of s.split('\n')) {
      const t = line.trim();
      if (!t) continue;
      const i = t.indexOf('=');
      if (i > 0) o[t.slice(0, i).trim()] = t.slice(i + 1).trim();
    }
    ag.env = o; markDirty();
  }
  const arrToLines = (a: string[] | undefined) => Array.isArray(a) ? a.join('\n') : '';
  function setSkills(ag: ConfigHolder, s: string) {
    ag.skills = s.split('\n').map(x => x.trim()).filter(Boolean);
    markDirty();
  }
  // mcp is structured → edit as JSON; parse on change, ignore invalid input.
  const mcpToText = (a: unknown[] | undefined) => (Array.isArray(a) && a.length) ? JSON.stringify(a, null, 2) : '';
  let mcpErr = $state<Record<number | string, boolean>>({});
  function setMcp(ag: ConfigHolder, i: number | string, s: string) {
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
    try {
      await onDelete(name);
      drafts = drafts.filter(d => d.name !== name);
      selIdx = Math.max(0, selIdx - 1);
    } catch {}
  }
</script>

<!-- Backdrop is a pointer affordance only; the modal's close button is the keyboard path. -->
<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="tpl-overlay" onclick={onClose}></div>
<div class="tpl-modal">
  <div class="tpl-head">
    <span class="tpl-title">{t('teamTemplates')}</span>
    <button class="tpl-x" onclick={onClose} aria-label={t('close')}><Icon name="x" size={15} /></button>
  </div>

  <!-- Global system prompt: prepended to EVERY agent's brief, across all teams.
       Collapsed by default — it's a rarely-touched global setting. -->
  <div class="sys-section">
    <div class="sys-label-row">
      <button class="sys-toggle" onclick={() => sysOpen = !sysOpen}>
        <Icon name={sysOpen ? 'chevron-down' : 'chevron-right'} size={12} />
        <span class="sys-label">{t('teamSystemPrompt')}</span>
        {#if sysDraft && sysDraft.trim()}<span class="ag-adv-badge">●</span>{/if}
      </button>
      {#if sysOpen}
        <button class="sys-save" disabled={!sysDirty || sysSaving} onclick={saveSystem}>
          {#if sysSaving}<span class="tpl-spin"></span>{/if}{t('teamSaveTemplate')}
        </button>
      {/if}
    </div>
    {#if sysOpen}
      <textarea class="sys-input" bind:value={sysDraft} oninput={() => sysDirty = true}
        placeholder={t('teamSystemPromptHint')} rows="3"></textarea>
    {/if}
  </div>

  <!-- Mobile: a compact dropdown to switch templates instead of the strip
       (the left-list sidebar is desktop-only). The dropdown is the app's ONE
       Select — it was a hand-rolled backdrop panel at `top: 100%` (no Escape,
       no close on scroll/resize; review, 2026-09-03). "New template" is its
       own button beside the field, not a row inside a pick-one list. -->
  <div class="tpl-picker">
    <span class="tpl-picker-label">{t('teamTplTag')}</span>
    <div class="tpl-picker-row">
      <div class="tpl-picker-sel">
        <Select value={String(selIdx)} ariaLabel={t('teamTplTag')}
          options={drafts.map((d, i) => ({ value: String(i), label: d.name, hint: String(d.agents?.length ?? 0) }))}
          onchange={(v: string) => { selIdx = Number(v); }} />
      </div>
      <button class="tpl-add tpl-picker-add" onclick={addTemplate} title={t('teamNewTemplate')} aria-label={t('teamNewTemplate')}>
        <Icon name="plus" size={12} />
      </button>
    </div>
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
          <span class="tpl-name-tag">{t('teamNameTag')}</span>
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
            <span class="ag-adv-label">{t('teamWidePrompt')}</span>
            <textarea class="ag-field ag-area" rows="3" placeholder={t('teamWidePromptHint')}
              value={sel.prompt ?? ''} oninput={(e) => { sel.prompt = e.currentTarget.value; markDirty(); }}></textarea>
            <span class="ag-adv-label">{t('teamSkills')}</span>
            <textarea class="ag-field ag-area" rows="2" placeholder={t('teamSkillsHint')}
              value={arrToLines(sel.skills)} oninput={(e) => setSkills(sel, e.currentTarget.value)}></textarea>
            <span class="ag-adv-label">{t('teamEnv')}</span>
            <textarea class="ag-field ag-area" rows="2" placeholder="KEY=VALUE"
              value={envToLines(sel.env)} oninput={(e) => setEnv(sel, e.currentTarget.value)}></textarea>
            <span class="ag-adv-label">{t('teamMcp')} {#if mcpErr['__team__']}<span class="ag-adv-err">{t('teamMcpBad')}</span>{/if}</span>
            <textarea class="ag-field ag-area ag-mono" rows="4" placeholder={t('teamMcpHint')}
              value={mcpToText(sel.mcp)} onchange={(e) => setMcp(sel, '__team__', e.currentTarget.value)}></textarea>
          </div>
        {/if}

        {#each sel.agents as ag, i}
          <div class="agent-card">
            <div class="agent-card-head">
              <span class="ag-card-title">{ag.name?.trim() || t('teamAgentName')}</span>
              <button class="ag-del" onclick={() => (pendingRm = { i, name: ag.name || `#${i + 1}` })}
                title={t('teamRemoveAgent')}
                aria-label={t('teamRemoveAgent')}><Icon name="trash" size={13} /></button>
            </div>
            <span class="ag-flabel">{t('teamAgentName')}</span>
            <input class="ag-field" bind:value={ag.name} oninput={markDirty} placeholder={t('teamAgentName')} autocapitalize="off" />
            <div class="ag-row">
              <div class="ag-col">
                <span class="ag-flabel">{t('teamBackend')}</span>
                <Select bind:value={ag.backend} options={BACKENDS} dense
                  ariaLabel={t('teamBackend')} onchange={markDirty} />
              </div>
              <label class="ag-manage" title={t('teamManagerHint')}>
                <input type="checkbox" bind:checked={ag.manage} onchange={markDirty} /> {t('teamManager')}
              </label>
            </div>
            <span class="ag-flabel">{t('teamRole')}</span>
            <input class="ag-field" bind:value={ag.role} oninput={markDirty} placeholder={t('teamRole')} />
            <span class="ag-flabel">{t('teamGoal')}</span>
            <textarea class="ag-field ag-area" bind:value={ag.goal} oninput={markDirty} placeholder={t('teamGoal')} rows="3"></textarea>
            <span class="ag-flabel">{t('teamModel')}</span>
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
                <span class="ag-adv-label">{t('teamSkills')}</span>
                <textarea class="ag-field ag-area" rows="2" placeholder={t('teamSkillsHint')}
                  value={arrToLines(ag.skills)} oninput={(e) => setSkills(ag, e.currentTarget.value)}></textarea>
                <span class="ag-adv-label">{t('teamEnv')}</span>
                <textarea class="ag-field ag-area" rows="2" placeholder="KEY=VALUE"
                  value={envToLines(ag.env)} oninput={(e) => setEnv(ag, e.currentTarget.value)}></textarea>
                <span class="ag-adv-label">{t('teamMcp')} {#if mcpErr[i]}<span class="ag-adv-err">{t('teamMcpBad')}</span>{/if}</span>
                <textarea class="ag-field ag-area ag-mono" rows="4" placeholder={t('teamMcpHint')}
                  value={mcpToText(ag.mcp)} onchange={(e) => setMcp(ag, i, e.currentTarget.value)}></textarea>
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
      <button class="tpl-delete" onclick={() => (pendingDelTpl = true)}>
        {t('teamDeleteTemplate')}
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

<ConfirmDialog open={!!pendingRm}
  title={pendingRm ? t('confirmRemoveTeamAgentTitle').replace('{name}', pendingRm.name) : ''}
  note={t('confirmRemoveTeamAgentNote')} confirmLabel={t('teamRemoveAgent')}
  onconfirm={() => { if (pendingRm) { removeAgent(pendingRm.i); pendingRm = null; } }}
  oncancel={() => (pendingRm = null)} />

<ConfirmDialog open={pendingDelTpl}
  title={t('confirmDeleteTemplateTitle').replace('{name}', sel?.name ?? '')}
  note={t('confirmDeleteTemplateNote')} confirmLabel={t('teamDeleteTemplate')}
  onconfirm={() => { pendingDelTpl = false; deleteCurrent(); }}
  oncancel={() => (pendingDelTpl = false)} />

<style>
  .tpl-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.45); z-index: 40; border: none; }
  .tpl-modal {
    position: fixed; z-index: 41;
    top: 50%; left: 50%; transform: translate(-50%, -50%);
    width: min(720px, calc(94vw / var(--ui-zoom, 1))); height: min(calc(80vh / var(--ui-zoom, 1)), 720px);
    display: flex; flex-direction: column;
    background: var(--bg); border: 1px solid var(--border); border-radius: 14px;
    box-shadow: 0 20px 60px rgba(0,0,0,0.5); overflow: hidden;
    box-sizing: border-box;
  }
  .tpl-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 14px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .tpl-title { font-size: var(--fs-body); font-weight: 600; color: var(--text); }
  .tpl-x { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; }
  .tpl-x:active { color: var(--accent); }

  .sys-section {
    display: flex; flex-direction: column; gap: 6px;
    padding: 10px 14px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .sys-label-row { display: flex; align-items: center; justify-content: space-between; }
  .sys-toggle {
    display: flex; align-items: center; gap: 6px; flex: 1;
    border: none; background: none; cursor: pointer; padding: 2px 0;
    color: var(--text3); -webkit-tap-highlight-color: transparent; text-align: left;
  }
  .sys-toggle:active { color: var(--accent); }
  .sys-label {
    font-size: var(--fs-meta); font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px;
  }
  .sys-save {
    display: inline-flex; align-items: center; gap: 5px;
    padding: 4px 10px; border: 1px solid var(--accent); border-radius: 7px;
    background: var(--accent-bg); color: var(--accent); font-size: var(--fs-sub); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .sys-save:disabled { opacity: 0.45; cursor: default; }
  .sys-input {
    width: 100%; box-sizing: border-box; max-height: 120px;
    padding: 8px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: var(--fs-ui);
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
    background: var(--input-bg); color: var(--text2); font-size: var(--fs-ui); cursor: pointer;
    text-align: left; -webkit-tap-highlight-color: transparent;
    font-family: var(--font-ui);
  }
  .tpl-item.active { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .tpl-count { color: var(--text3); font-size: var(--fs-sub); }
  .tpl-add {
    padding: 7px 9px; border: 1px dashed var(--border2); border-radius: 8px;
    background: transparent; color: var(--text3); font-size: var(--fs-ui); cursor: pointer;
    display: flex; align-items: center; gap: 5px; -webkit-tap-highlight-color: transparent;
  }
  .tpl-add:active { color: var(--accent); border-color: var(--accent); }

  /* Mobile template dropdown — hidden on desktop (the sidebar list is used). */
  .tpl-picker { display: none; }
  .tpl-picker-label { display: block; margin-bottom: 5px; font-size: var(--fs-meta); font-weight: 600; color: var(--text3); text-transform: uppercase; letter-spacing: 0.4px; }
  .tpl-picker-row { display: flex; align-items: stretch; gap: 8px; }
  .tpl-picker-sel { flex: 1; min-width: 0; }
  /* The same dashed add control the desktop strip ends with, squared to the
     field's height so the row reads as one line. */
  .tpl-picker-add { flex: none; padding: 0; width: 38px; justify-content: center; }

  .tpl-edit { flex: 1; min-width: 0; padding: 12px; overflow-y: auto; display: flex; flex-direction: column; gap: 10px; }
  .tpl-name-input {
    width: 100%; padding: 8px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: var(--fs-body); font-weight: 600; outline: none;
    font-family: var(--font-ui);
  }
  .tpl-name-row { display: flex; align-items: center; gap: 8px; }
  .tpl-name-tag { flex-shrink: 0; font-size: var(--fs-sub); font-weight: 600; color: var(--text3); text-transform: uppercase; letter-spacing: 0.4px; }
  .tpl-name-input:focus { border-color: var(--accent); }
  .tpl-name-input:disabled { opacity: 0.6; }

  .agent-card {
    display: flex; flex-direction: column; gap: 6px;
    padding: 10px; border: 1px solid var(--border2); border-radius: 10px; background: var(--surface);
  }
  .agent-card-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ag-card-title {
    flex: 1; min-width: 0; font-size: var(--fs-body); font-weight: 600; color: var(--text);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .ag-flabel {
    font-size: var(--fs-meta); color: var(--text3); font-weight: 600;
    text-transform: uppercase; letter-spacing: 0.4px; margin-top: 2px;
  }
  .ag-row { display: flex; align-items: flex-end; gap: 12px; }
  .ag-col { display: flex; flex-direction: column; gap: 4px; }
  .ag-field {
    padding: 6px 9px; border: 1px solid var(--input-border); border-radius: 7px;
    background: var(--input-bg); color: var(--text); font-size: var(--fs-ui); font-family: inherit;
    outline: none; width: 100%; box-sizing: border-box;
  }
  .ag-field:focus { border-color: var(--accent); }
  .ag-manage { display: flex; align-items: center; gap: 3px; font-size: var(--fs-sub); color: var(--text3); white-space: nowrap; }
  .ag-del { border: none; background: none; color: var(--text3); cursor: pointer; display: flex; flex-shrink: 0; padding: 4px; }
  .ag-del:active { color: var(--danger); }
  .ag-area { resize: vertical; line-height: 1.4; }

  .ag-adv-toggle {
    display: flex; align-items: center; gap: 5px; align-self: flex-start;
    padding: 3px 6px; border: none; background: none; cursor: pointer;
    color: var(--text3); font-size: var(--fs-sub); font-weight: 600;
    -webkit-tap-highlight-color: transparent; font-family: var(--font-ui);
  }
  .ag-adv-toggle:active { color: var(--accent); }
  .ag-adv-badge {
    min-width: 15px; padding: 0 4px; border-radius: 7px; background: var(--accent-bg);
    color: var(--accent); font-size: var(--fs-meta); text-align: center;
  }
  .ag-adv { display: flex; flex-direction: column; gap: 5px; padding-left: 4px; border-left: 2px solid var(--border2); }
  .ag-adv-label { font-size: var(--fs-meta); color: var(--text3); text-transform: uppercase; letter-spacing: 0.4px; }
  .ag-adv-err { color: var(--danger); text-transform: none; letter-spacing: 0; margin-left: 6px; }
  .ag-mono { font-family: var(--font-mono, monospace); font-size: var(--fs-sub); }

  .tw-toggle {
    display: flex; align-items: center; gap: 5px; align-self: flex-start;
    padding: 5px 8px; border: 1px dashed var(--border2); border-radius: 8px;
    background: transparent; color: var(--text2); font-size: var(--fs-ui); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent; font-family: var(--font-ui);
  }
  .tw-toggle:active { color: var(--accent); border-color: var(--accent); }
  .tw-adv { padding: 8px; border: 1px solid var(--border2); border-radius: 10px; background: var(--surface); }

  .agent-add {
    padding: 9px; border: 1px dashed var(--border2); border-radius: 10px;
    background: transparent; color: var(--text3); font-size: var(--fs-body); cursor: pointer;
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
    background: none; color: var(--danger); font-size: var(--fs-ui); cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .tpl-cancel {
    padding: 7px 12px; border: 1px solid var(--border2); border-radius: 8px;
    background: none; color: var(--text2); font-size: var(--fs-ui); cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .tpl-save {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 7px 14px; border: 1px solid var(--accent); border-radius: 8px;
    background: var(--accent-bg); color: var(--accent); font-size: var(--fs-ui); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .tpl-save:disabled { opacity: 0.5; cursor: default; }
  .tpl-spin {
    width: 11px; height: 11px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%; animation: tpl-spin 0.6s linear infinite;
  }
  @keyframes tpl-spin { to { transform: rotate(360deg); } }

  /* Phone overrides. Placed LAST so they win over the base rules above (a media
     query adds no specificity — source order decides between equal selectors). */
  @media (max-width: 620px) {
    .tpl-modal {
      width: calc(100vw / var(--ui-zoom, 1)); height: 100%; height: calc(var(--app-height, 100dvh) / var(--ui-zoom, 1));
      top: 0; left: 0; transform: none; border-radius: 0; border: none;
      padding-top: var(--sat); padding-bottom: var(--sab);
    }
    .tpl-body { flex-direction: column; }
    /* The horizontal strip is replaced by the compact dropdown on phones. */
    .tpl-list { display: none; }
    .tpl-picker { display: block; flex-shrink: 0; padding: 8px 14px; border-bottom: 1px solid var(--border); }

    /* iOS zooms the page when focusing an input below the threshold. */
    .ag-field, .sys-input, .tpl-name-input { font-size: var(--fs-input-touch); }
    /* Deliberately below it: mono glyphs are wider, and this textarea was
       tuned by eye. Left raw so the exception stays visible. */
    .ag-mono { font-size: 15px; }

    /* Agent header: name on its own row; backend + mgr + delete below. */
    .agent-card-head { gap: 8px; }
    .ag-del { margin-left: auto; padding: 8px; }

    /* Roomier touch targets. */
    .ag-adv-toggle, .tw-toggle { padding: 8px; font-size: var(--fs-body); }
    .ag-manage { font-size: var(--fs-body); }
    .ag-manage input { width: 16px; height: 16px; }
    .tpl-foot button { padding: 10px 14px; font-size: var(--fs-body); }
  }
</style>
