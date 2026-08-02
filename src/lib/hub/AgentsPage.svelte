<script>
  // AgentsPage — the agent configuration page, in the Hub page format
  // (ui-unification.md "Page skeleton"): a real sidebar (bg2, .side-h,
  // .side-row entries) + a main column with a .page-head. Definitions
  // (backend, model, persona, skills, MCP servers, hire permission) are
  // edited HERE and only here; the Hub consumes them.
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
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

<div class="agents-root" class:editing={!!editing}>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll">
      <div class="side-h">{t('agentsTitle')}</div>
      {#each defs as d (d.name)}
        <button class="side-row" class:open={editing?.name === d.name && !isNew} onclick={() => startEdit(d)}>
          <span class="ava" style:background={backendColor(d.backend)}>{d.name.slice(0, 1).toUpperCase()}</span>
          <span class="r-name">{d.name}</span>
          <span class="r-backend">{d.backend}{d.can_hire ? ' ⚡' : ''}</span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startEdit(null)}>
        <Icon name="plus" size={13} />{t('agentsNew')}
      </button>
    </div>
  </aside>

  <main class="mid">
    {#if editing}
      <div class="page-head">
        <h1>{isNew ? t('agentsNew') : editing.name}</h1>
        <span class="spacer"></span>
        {#if !isNew}
          <button class="chip-btn danger" onclick={() => remove(editing.name)}><Icon name="trash" size={13} />{t('delete')}</button>
        {/if}
        <button class="chip-btn" onclick={() => editing = null}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!editing.name.trim()} onclick={save}>{t('save')}</button>
      </div>
      <div class="editor">
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
    {:else}
      <div class="page-head"><h1>{t('agentsTitle')}</h1></div>
      <div class="placeholder">
        <p class="hint">{t('agentsHint')}</p>
      </div>
    {/if}
  </main>
</div>

<style>
  .agents-root { height: 100%; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; background: var(--bg); }
  @media (max-width: 760px) {
    .agents-root { grid-template-columns: minmax(0, 1fr); }
    /* Compact: the list is the page; editing takes the screen. */
    .agents-root.editing .sidebar { display: none; }
    .agents-root:not(.editing) .mid { display: none; }
  }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .r-name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; }
  .r-backend { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); flex: none; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .spacer { flex: 1; }
  .placeholder { flex: 1; display: grid; place-items: center; }
  .hint { color: var(--text3); font-size: 12.5px; margin: 0; line-height: 1.6; max-width: 420px; }

  .editor { flex: 1; overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; }
  .err { color: var(--danger); font-size: 12.5px; background: var(--danger-bg); border-radius: 8px; padding: 8px 12px; }
  label { display: flex; flex-direction: column; gap: 5px; color: var(--text2); font-size: 12px; }
  label.check { flex-direction: row; align-items: center; gap: 8px; font-size: 13px; color: var(--text); }
  input, select, textarea { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 9px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; font-family: inherit; }
  input:focus, select:focus, textarea:focus { border-color: var(--accent); }
  input:disabled { opacity: 0.5; }
  textarea { resize: vertical; line-height: 1.5; }
  textarea.mono { font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
</style>
