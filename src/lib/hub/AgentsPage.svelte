<script>
  // AgentsPage — the agent configuration page. Definitions (backend, model,
  // persona, skills, MCP servers, hire permission) are edited HERE and only
  // here; the Hub consumes them (project creation picks agents, spawn
  // instantiates them into isolated homes). This is the page the registry
  // deserved instead of a list squatting in the Hub sidebar.
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { registryList, registrySave, registryDelete } from '../core/ws.ts';
  import { backendColor } from '../hub/hub.ts';

  let { visible = false } = $props();

  let defs = $state([]);
  let editing = $state(null);   // working copy or null
  let isNew = $state(false);
  let error = $state('');

  async function reload() {
    try { defs = (await registryList()).agents ?? []; } catch { defs = []; }
  }
  $effect(() => { if (visible) reload(); });

  function startEdit(def) {
    error = '';
    isNew = !def;
    editing = def
      ? { ...def, skillsText: parseRefs(def.skills).join(', '), mcpText: pretty(def.mcp) }
      : { name: '', backend: 'kiro', model: '', system: '', can_hire: false, skillsText: '', mcpText: '[]' };
  }

  function parseRefs(json) {
    try { return JSON.parse(json) ?? []; } catch { return []; }
  }
  function pretty(json) {
    try { return JSON.stringify(JSON.parse(json), null, 2); } catch { return json || '[]'; }
  }

  async function save() {
    if (!editing) return;
    error = '';
    const skills = editing.skillsText.split(',').map((s) => s.trim()).filter(Boolean);
    let mcp;
    try {
      mcp = JSON.stringify(JSON.parse(editing.mcpText.trim() || '[]'));
    } catch {
      error = t('agentsMcpInvalid');
      return;
    }
    try {
      await registrySave({
        name: editing.name.trim(),
        backend: editing.backend,
        model: editing.model.trim(),
        system: editing.system,
        skills: JSON.stringify(skills),
        mcp,
        can_hire: editing.can_hire,
      });
      editing = null;
      await reload();
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function remove(name) {
    try {
      await registryDelete(name);
      if (editing?.name === name) editing = null;
      await reload();
    } catch (e) { error = String(e?.message ?? e); }
  }
</script>

<div class="agents-root">
  <div class="list">
    <div class="list-head">
      <h1>{t('agentsTitle')}</h1>
      <button class="chip-btn primary" onclick={() => startEdit(null)}>＋ {t('agentsNew')}</button>
    </div>
    <p class="hint">{t('agentsHint')}</p>
    {#each defs as d (d.name)}
      <button class="def" class:sel={editing?.name === d.name && !isNew} onclick={() => startEdit(d)}>
        <span class="ava" style:background={backendColor(d.backend)}>{d.name.slice(0, 1).toUpperCase()}</span>
        <span class="d-main">
          <span class="d-name">{d.name}<span class="d-backend">· {d.backend}{d.model ? ` · ${d.model}` : ''}</span>
            {#if d.can_hire}<span class="hire-tag">{t('agentsCanHire')}</span>{/if}
          </span>
          <span class="d-sys">{d.system}</span>
        </span>
      </button>
    {/each}
  </div>

  {#if editing}
    <div class="editor">
      <div class="ed-head">
        <h2>{isNew ? t('agentsNew') : editing.name}</h2>
        <span class="spacer"></span>
        {#if !isNew}
          <button class="chip-btn danger" onclick={() => remove(editing.name)}><Icon name="trash" size={13} />{t('delete')}</button>
        {/if}
        <button class="chip-btn" onclick={() => editing = null}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!editing.name.trim()} onclick={save}>{t('save')}</button>
      </div>
      {#if error}<div class="err">{error}</div>{/if}

      <label>{t('agentsName')}
        <input bind:value={editing.name} disabled={!isNew} placeholder="reviewer" />
      </label>
      <div class="row2">
        <label>{t('agentsBackend')}
          <select bind:value={editing.backend}>
            <option value="kiro">kiro</option>
            <option value="claude">claude</option>
            <option value="codex">codex</option>
          </select>
        </label>
        <label>{t('agentsModel')}
          <input bind:value={editing.model} placeholder={t('agentsModelDefault')} />
        </label>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={editing.can_hire} />
        {t('agentsCanHireLabel')}
      </label>
      <label>{t('agentsSystem')}
        <textarea rows="6" bind:value={editing.system} placeholder={t('agentsSystemPh')}></textarea>
      </label>
      <label>{t('agentsSkills')}
        <input bind:value={editing.skillsText} placeholder="git-review, github.com/org/repo/skills/docs" />
      </label>
      <label>{t('agentsMcp')}
        <textarea class="mono" rows="6" bind:value={editing.mcpText} spellcheck="false"></textarea>
      </label>
      <p class="hint">{t('agentsMcpHint')}</p>
    </div>
  {/if}
</div>

<style>
  .agents-root { height: 100%; display: grid; grid-template-columns: minmax(280px, 0.9fr) minmax(0, 1.1fr); min-height: 0; background: var(--bg); }
  .agents-root:has(.editor) .list { border-right: 1px solid var(--border); }
  @media (max-width: 760px) {
    .agents-root { grid-template-columns: minmax(0, 1fr); }
    /* Editing takes the screen on a phone. */
    .agents-root:has(.editor) .list { display: none; }
  }

  .list { overflow-y: auto; padding: 14px 16px; display: flex; flex-direction: column; gap: 6px; }
  .list-head { display: flex; align-items: center; justify-content: space-between; }
  .list-head h1 { font-size: 16px; margin: 0; }
  .hint { color: var(--text3); font-size: 12px; margin: 2px 0 8px; line-height: 1.5; }
  .def { display: flex; align-items: flex-start; gap: 10px; background: var(--surface); border: 1px solid var(--border); border-radius: 11px; padding: 10px 12px; cursor: pointer; text-align: left; transition: border-color 160ms; }
  .def:hover { border-color: var(--input-border); }
  .def.sel { border-color: var(--accent); background: var(--accent-bg); }
  .ava { width: 26px; height: 26px; border-radius: 8px; flex: none; display: grid; place-items: center; font-family: ui-monospace, Menlo, monospace; font-size: 12px; font-weight: 700; color: var(--bg); margin-top: 1px; }
  .d-main { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .d-name { color: var(--text); font-weight: 600; font-size: 13.5px; display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .d-backend { color: var(--text3); font-weight: 400; font-size: 12px; }
  .hire-tag { font-size: 10px; color: var(--accent); border: 1px solid var(--accent); border-radius: 5px; padding: 0 5px; }
  .d-sys { color: var(--text3); font-size: 12px; overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }

  .editor { overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; }
  .ed-head { display: flex; align-items: center; gap: 8px; }
  .ed-head h2 { font-size: 15px; margin: 0; font-family: ui-monospace, Menlo, monospace; }
  .spacer { flex: 1; }
  .err { color: var(--danger); font-size: 12.5px; background: var(--danger-bg); border-radius: 8px; padding: 8px 12px; }
  label { display: flex; flex-direction: column; gap: 5px; color: var(--text2); font-size: 12px; }
  label.check { flex-direction: row; align-items: center; gap: 8px; font-size: 13px; color: var(--text); }
  input, select, textarea { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 9px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; font-family: inherit; }
  input:focus, select:focus, textarea:focus { border-color: var(--accent); }
  input:disabled { opacity: 0.5; }
  textarea { resize: vertical; line-height: 1.5; }
  textarea.mono { font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .chip-btn { display: flex; align-items: center; gap: 5px; background: var(--surface); border: 1px solid var(--border); color: var(--text2); border-radius: 8px; padding: 5px 11px; font-size: 12.5px; cursor: pointer; transition: border-color 160ms, color 160ms; }
  .chip-btn:hover { border-color: var(--accent); color: var(--accent); }
  .chip-btn:disabled { opacity: 0.4; cursor: default; }
  .chip-btn.primary { background: var(--accent-bg); color: var(--accent); border-color: transparent; }
  .chip-btn.danger:hover { border-color: var(--danger); color: var(--danger); }
</style>
