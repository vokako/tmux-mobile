<script>
  // AgentsPage — the agent configuration page, in the Hub page format
  // (ui-unification.md "Page skeleton"): a real sidebar (bg2, .side-h,
  // .side-row entries) + a main column with a .page-head. Definitions
  // (backend, model, persona, skills, MCP servers, hire permission) are
  // edited HERE and only here; the Hub consumes them.
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { registryList, registrySave, registryDelete, modelsList, skillsList, skillsSave, skillsDelete, skillsRefresh, skillsRead, mcpList, mcpSave, mcpDelete } from '../core/ws.ts';
  import { renderMarkdown } from '../core/markdown.ts';
  import { backendColor } from '../hub/hub.ts';

  let { visible = false } = $props();

  let defs = $state([]);
  let skills = $state([]);      // central skill assets
  let mcps = $state([]);        // central MCP server defs
  let editing = $state(null);   // agent working copy or null
  let isNew = $state(false);
  // One editor at a time across the three kinds.
  let editingSkill = $state(null);
  let skillIsNew = $state(false);
  let editingMcp = $state(null);
  let mcpIsNew = $state(false);
  let error = $state('');

  // The model ids the selected backend accepts, asked of the backend's own CLI
  // (per backend, cached server-side). This is a suggestion list, not a
  // restriction: an id we cannot enumerate is still typeable, and `registry_save`
  // is the authority that rejects one the backend would silently ignore — a
  // dashed `claude-sonnet-4-5` ran happily on the DEFAULT model instead
  // (owner report, 2026-08-19).
  let models = $state([]);
  $effect(() => {
    const backend = editing?.backend;
    if (!backend) { models = []; return; }
    let live = true;
    modelsList(backend)
      .then((r) => { if (live) models = r.models ?? []; })
      .catch(() => { if (live) models = []; });
    return () => { live = false; };
  });

  async function reload() {
    try { defs = (await registryList()).agents ?? []; } catch { defs = []; }
    try { skills = (await skillsList()).skills ?? []; } catch { skills = []; }
    try { mcps = (await mcpList()).mcp ?? []; } catch { mcps = []; }
  }
  $effect(() => { if (visible) reload(); });

  function closeAll() {
    editing = null; editingSkill = null; editingMcp = null;
    error = '';
  }

  let skillMd = $state('');
  // The YAML frontmatter duplicates the form fields (name/description) —
  // the preview shows the skill's BODY.
  function stripFrontmatter(md) {
    const m = /^---\n[\s\S]*?\n---\n?/.exec(md);
    return m ? md.slice(m[0].length) : md;
  }
  function startSkill(sk) {
    closeAll();
    skillIsNew = !sk;
    editingSkill = sk ? { ...sk } : { name: '', source: '', description: '' };
    skillMd = '';
    if (sk) {
      skillsRead(sk.name)
        .then((r) => { if (editingSkill?.name === sk.name) skillMd = r.content; })
        .catch(() => { skillMd = ''; });
    }
  }
  let syncing = $state(false);
  async function saveSkill() {
    syncing = true;
    try {
      await skillsSave(editingSkill);
      editingSkill = null;
      await reload();
    } catch (e) { error = String(e?.message ?? e); }
    finally { syncing = false; }
  }
  async function refreshSkill() {
    syncing = true;
    error = '';
    try {
      await skillsRefresh(editingSkill.name);
      await reload();
      editingSkill = { ...skills.find((x) => x.name === editingSkill.name) };
      skillMd = (await skillsRead(editingSkill.name).catch(() => ({ content: '' }))).content;
    } catch (e) { error = String(e?.message ?? e); }
    finally { syncing = false; }
  }
  async function removeSkill(name) {
    try { await skillsDelete(name); editingSkill = null; await reload(); }
    catch (e) { error = String(e?.message ?? e); }
  }

  function startMcp(m) {
    closeAll();
    mcpIsNew = !m;
    editingMcp = m ? { ...m, defText: pretty(m.def) } : { name: '', defText: '{\n  "command": "",\n  "args": []\n}' };
  }
  async function saveMcp() {
    let def;
    try { def = JSON.stringify(JSON.parse(editingMcp.defText)); }
    catch { error = t('agentsMcpInvalid'); return; }
    try {
      await mcpSave({ name: editingMcp.name.trim(), def });
      editingMcp = null;
      await reload();
    } catch (e) { error = String(e?.message ?? e); }
  }
  async function removeMcp(name) {
    try { await mcpDelete(name); editingMcp = null; await reload(); }
    catch (e) { error = String(e?.message ?? e); }
  }

  function startEdit(def) {
    closeAll();
    isNew = !def;
    // skills / mcp become SELECTIONS over the central assets (closed loop —
    // no free-text names). Legacy inline objects or unknown names in an old
    // def are preserved untouched in `extra` so saving does not eat them.
    const skillSel = def ? parseRefs(def.skills) : [];
    const mcpEntries = def ? parseRefs(def.mcp) : [];
    editing = def
      ? {
          ...def,
          skillSel: skillSel.filter((x) => typeof x === 'string'),
          mcpSel: mcpEntries.filter((x) => typeof x === 'string'),
          mcpExtra: mcpEntries.filter((x) => typeof x !== 'string'),
        }
      : { name: '', backend: 'kiro', model: '', system: '', can_hire: false, skillSel: [], mcpSel: [], mcpExtra: [] };
  }
  function toggleSel(list, name) {
    return list.includes(name) ? list.filter((n) => n !== name) : [...list, name];
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
    try {
      await registrySave({
        name: editing.name.trim(),
        backend: editing.backend,
        model: editing.model.trim(),
        system: editing.system,
        skills: JSON.stringify(editing.skillSel),
        mcp: JSON.stringify([...editing.mcpSel, ...editing.mcpExtra]),
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

<div class="agents-root" class:editing={!!editing || !!editingSkill || !!editingMcp}>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll">
      <div class="side-h">{t('agentsTitle')}</div>
      {#each defs as d (d.name)}
        <button class="side-row" class:open={editing?.name === d.name && !isNew} onclick={() => startEdit(d)}>
          <span class="ava" style:background={backendColor(d.backend)}>{d.name.slice(0, 1).toUpperCase()}</span>
          <span class="r-name">{d.name}</span>
          <span class="r-backend">{d.backend}</span>
          <!-- can_hire: this agent may spawn teammates. A tag, not a glyph. -->
          {#if d.can_hire}<span class="r-cap" title={t('agentsCanHireLabel')}>{t('agentsCanHire')}</span>{/if}
        </button>
      {/each}
      <button class="side-row add" onclick={() => startEdit(null)}>
        <Icon name="plus" size={13} />{t('agentsNew')}
      </button>

      <div class="side-h">{t('skillsTitle')}</div>
      {#each skills as sk (sk.name)}
        <button class="side-row" class:open={editingSkill?.name === sk.name && !skillIsNew} onclick={() => startSkill(sk)}>
          <Icon name="zap" size={13} />
          <span class="r-name">{sk.name}</span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startSkill(null)}>
        <Icon name="plus" size={13} />{t('skillsNew')}
      </button>

      <div class="side-h">MCP</div>
      {#each mcps as m (m.name)}
        <button class="side-row" class:open={editingMcp?.name === m.name && !mcpIsNew} onclick={() => startMcp(m)}>
          <Icon name="link" size={13} />
          <span class="r-name">{m.name}</span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startMcp(null)}>
        <Icon name="plus" size={13} />{t('mcpNew')}
      </button>
    </div>
  </aside>

  <main class="mid">
    {#if editingSkill}
      <div class="page-head">
        <h1>{skillIsNew ? t('skillsNew') : editingSkill.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !skillIsNew}
          <button class="chip-btn danger" onclick={() => removeSkill(editingSkill.name)}><Icon name="trash" size={13} />{t('delete')}</button>
        {/if}
        {#if !skillIsNew}
          <button class="chip-btn" disabled={syncing} onclick={refreshSkill}><Icon name="refresh" size={13} />{t('skillsRefresh')}</button>
        {/if}
        <button class="chip-btn" onclick={() => editingSkill = null}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!editingSkill.name.trim() || !editingSkill.source.trim() || syncing} onclick={saveSkill}>{syncing ? '…' : (skillIsNew ? t('skillsImport') : t('save'))}</button>
        </div>
      </div>
      <div class="editor">
        {#if error}<div class="err">{error}</div>{/if}
        <label>{t('agentsName')}
          <input bind:value={editingSkill.name} disabled={!skillIsNew} placeholder="git-review" />
        </label>
        <label>{t('skillsSource')}
          <input bind:value={editingSkill.source} placeholder="https://github.com/org/repo/tree/main/skills/git-review 或 /abs/local/dir" />
        </label>
        <label>{t('skillsDesc')}
          <input bind:value={editingSkill.description} />
        </label>
        {#if editingSkill.synced_at}
          <p class="hint">{t('skillsSynced')} {new Date(editingSkill.synced_at * 1000).toLocaleString()}</p>
        {/if}
        <p class="hint">{t('skillsHint')}</p>
        {#if skillMd}
          <div class="md-preview">
            <div class="side-h">SKILL.md</div>
            <div class="md md-doc">{@html renderMarkdown(stripFrontmatter(skillMd))}</div>
          </div>
        {/if}
      </div>
    {:else if editingMcp}
      <div class="page-head">
        <h1>{mcpIsNew ? t('mcpNew') : editingMcp.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !mcpIsNew}
          <button class="chip-btn danger" onclick={() => removeMcp(editingMcp.name)}><Icon name="trash" size={13} />{t('delete')}</button>
        {/if}
        <button class="chip-btn" onclick={() => editingMcp = null}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!editingMcp.name.trim()} onclick={saveMcp}>{t('save')}</button>
        </div>
      </div>
      <div class="editor">
        {#if error}<div class="err">{error}</div>{/if}
        <label>{t('agentsName')}
          <input bind:value={editingMcp.name} disabled={!mcpIsNew} placeholder="files" />
        </label>
        <label>{t('mcpDef')}
          <textarea class="mono" rows="10" bind:value={editingMcp.defText} spellcheck="false"></textarea>
        </label>
        <p class="hint">{t('mcpHint')}</p>
      </div>
    {:else if editing}
      <div class="page-head">
        <h1>{isNew ? t('agentsNew') : editing.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !isNew}
          <button class="chip-btn danger" onclick={() => remove(editing.name)}><Icon name="trash" size={13} />{t('delete')}</button>
        {/if}
        <button class="chip-btn" onclick={() => editing = null}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!editing.name.trim()} onclick={save}>{t('save')}</button>
        </div>
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
            <input bind:value={editing.model} placeholder={t('agentsModelDefault')}
              list={models.length ? 'agent-model-ids' : undefined} />
            {#if models.length}
              <datalist id="agent-model-ids">
                {#each models as m (m)}<option value={m}></option>{/each}
              </datalist>
            {/if}
          </label>
        </div>
        <label class="check">
          <input type="checkbox" bind:checked={editing.can_hire} />
          {t('agentsCanHireLabel')}
        </label>
        <label>{t('agentsSystem')}
          <textarea rows="6" bind:value={editing.system} placeholder={t('agentsSystemPh')}></textarea>
        </label>
        <div class="pick-block">
          <span class="pick-label">{t('agentsSkills')}</span>
          {#if skills.length}
            <div class="pick-row">
              {#each skills as sk (sk.name)}
                <button class="pick" class:sel={editing.skillSel.includes(sk.name)} onclick={() => editing.skillSel = toggleSel(editing.skillSel, sk.name)}>
                  <Icon name="zap" size={11} />{sk.name}
                </button>
              {/each}
            </div>
          {:else}
            <p class="hint">{t('agentsNoSkills')}</p>
          {/if}
        </div>
        <div class="pick-block">
          <span class="pick-label">{t('agentsMcp')}</span>
          {#if mcps.length}
            <div class="pick-row">
              {#each mcps as m (m.name)}
                <button class="pick" class:sel={editing.mcpSel.includes(m.name)} onclick={() => editing.mcpSel = toggleSel(editing.mcpSel, m.name)}>
                  <Icon name="link" size={11} />{m.name}
                </button>
              {/each}
            </div>
          {:else}
            <p class="hint">{t('agentsNoMcp')}</p>
          {/if}
          {#if editing.mcpExtra.length}
            <p class="hint">{t('agentsMcpExtra').replace('{n}', String(editing.mcpExtra.length))}</p>
          {/if}
        </div>
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
  .r-cap { flex: none; font-size: 9px; letter-spacing: 0.4px; color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 3px; opacity: 0.75; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .spacer { flex: 1; }
  /* The head actions move as ONE block: on a phone they wrap under the
     title together instead of scattering one button per row. */
  .head-acts { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; margin-left: auto; }
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
  .pick-block { display: flex; flex-direction: column; gap: 6px; }
  .pick-label { color: var(--text2); font-size: 12px; }
  .pick-row { display: flex; flex-wrap: wrap; gap: 6px; }
  .pick {
    display: flex; align-items: center; gap: 5px;
    background: var(--surface); border: 1px solid var(--border); border-radius: 999px;
    color: var(--text2); padding: 5px 12px; font-size: 12.5px; cursor: pointer;
    transition: border-color 160ms, color 160ms;
  }
  .pick:hover { border-color: var(--input-border); }
  .pick.sel { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .md-preview { border-top: 1px solid var(--border2); margin-top: 6px; }
  .md-doc {
    background: var(--surface); border: 1px solid var(--border2); border-radius: 10px;
    padding: 12px 14px; font-size: 13px; color: var(--text); line-height: 1.55;
    overflow-wrap: anywhere;
  }
</style>
