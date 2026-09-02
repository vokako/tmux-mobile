<script>
  // AgentsPage — the agent configuration page, in the Hub page format
  // (ui-unification.md "Page skeleton"): a real sidebar (bg2, .side-h,
  // .side-row entries) + a main column with a .page-head. Definitions
  // (backend, model, persona, skills, MCP servers, hire permission) are
  // edited HERE and only here; the Hub consumes them.
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import { scrollFade } from '../core/scrollFade.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { registryList, registrySave, registryDelete, modelsList, skillsList, skillsSave, skillsDelete, skillsRefresh, skillsImport, skillsFiles, skillsFile, mcpList, mcpSave, mcpDelete, teamsList, teamsSave, teamsDelete } from '../core/ws.ts';
  import { renderMarkdown } from '../core/markdown.ts';
  import { backendColor } from '../hub/hub.ts';
  import { backendIcon } from '../core/agents.ts';
  import Select from '../ui/Select.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';

  /** The backends a registry agent can run on — the same list the server
   * validates against in `registry_save`. */
  const BACKENDS = ['kiro', 'claude', 'codex', 'grok'];
  // Reasoning-effort levels each backend's CLI accepts — mirrors the server's
  // models::effort_values (measured per CLI, 2026-08-22). '' = backend default.
  const EFFORTS = {
    kiro: ['low', 'medium', 'high', 'xhigh', 'max'],
    claude: ['low', 'medium', 'high', 'xhigh', 'max'],
    codex: ['minimal', 'low', 'medium', 'high', 'xhigh'],
    grok: ['low', 'medium', 'high', 'xhigh'],
  };

  // `section` narrows the page to ONE kind — 'agents' | 'teams' | 'skills' |
  // 'mcp' — for the phone's Settings, where each is its own second-level page
  // (owner, 2026-09-02: "把 team agent mcp skill 分开几个二级设置页面吧，在手机
  // 上"). null = the desktop page with every section in one sidebar.
  let { visible = false, onGoBack = null, editRequest = null, onDrilled = null, section = null } = $props();
  const SECTION_META = {
    agents: ['agentsTitle', 'agentsHint'],
    teams: ['teamsTitle', 'teamsHint'],
    skills: ['skillsTitle', 'skillsHint'],
    mcp: ['mcpTitle', 'mcpHint'],
  };
  const shows = (kind) => !section || section === kind;
  // Switching category closes whatever editor the previous one had open — the
  // page-head above already names the new category, an editor from the old
  // one would contradict it.
  let lastSection;
  $effect(() => {
    if (lastSection !== undefined && section !== lastSection) closeAll();
    lastSection = section;
  });

  let defs = $state([]);
  let teams = $state([]);       // agent teams (board #74): RegTeam[]
  let skills = $state([]);      // central skill assets
  let mcps = $state([]);        // central MCP server defs
  let editing = $state(null);   // agent working copy or null
  let isNew = $state(false);
  // One editor at a time across the three kinds.
  let editingSkill = $state(null);
  let skillIsNew = $state(false);
  let editingMcp = $state(null);
  // A TEAM working copy (board #74): { name, description, members: TeamMember[] }.
  let editingTeam = $state(null);
  let teamIsNew = $state(false);
  const drilled = $derived(!!editing || !!editingSkill || !!editingMcp || !!editingTeam);
  let drillAnim = $state('');
  let wasDrilled = false;
  let mcpIsNew = $state(false);
  let error = $state('');
  let info = $state(''); // a good-news line (e.g. what a plugin import installed)

  /** The pending destructive action: `{ kind, name }`. Deleting an agent
   * definition, a skill or an MCP server used to be immediate — one stray tap
   * on a phone and a definition was gone (owner asked for the audit,
   * 2026-08-19). The words per kind live here so the dialog stays generic. */
  let pending = $state(null);
  let removing = $state(false);

  // The phone's back gesture peels this page's layers like Files does its
  // views (owner, 2026-08-24): dialog first, then an open editor (compact:
  // "the list is the page; editing takes the screen"), then the floor.
  $effect(() => {
    // Drill motion (design-language.md §1 navigation grammar): the editor
    // enters from the right, the list re-enters from the left — derived from
    // the one compound flag so every open/close path animates alike.
    if (drilled !== wasDrilled) { drillAnim = drilled ? 'fwd' : 'back'; wasDrilled = drilled; }
    // An EMBEDDING host needs the same flag: mounted as a Settings category on
    // a phone, this page's editor brings its own `.page-head`, so Settings has
    // to drop its own or the screen wears two stacked title bars.
    onDrilled?.(drilled);
    if (!onGoBack) return;
    onGoBack(() => {
      if (pending && !removing) { pending = null; return true; }
      if (editing) { editing = null; return true; }
      if (editingTeam) { editingTeam = null; return true; }
      if (editingSkill) { editingSkill = null; return true; }
      if (editingMcp) { editingMcp = null; return true; }
      return false;
    });
  });
  const COPY = {
    agent: { title: 'confirmDeleteAgentDefTitle', note: 'confirmDeleteAgentDefNote' },
    skill: { title: 'confirmDeleteSkillTitle',    note: 'confirmDeleteSkillNote' },
    mcp:   { title: 'confirmDeleteMcpTitle',      note: 'confirmDeleteMcpNote' },
    team:  { title: 'confirmDeleteTeamTitle',     note: 'confirmDeleteTeamNote' },
  };
  const ask = (kind, name) => { pending = { kind, name }; };
  async function runPending() {
    if (!pending || removing) return;
    const { kind, name } = pending;
    removing = true;
    try {
      if (kind === 'agent') await remove(name);
      else if (kind === 'team') await removeTeam(name);
      else if (kind === 'skill') await removeSkill(name);
      else await removeMcp(name);
      pending = null;
    } finally { removing = false; }
  }

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
    // "I could not ask" is not "there is nothing": keep the last known lists
    // on a failed RPC (same rule as the Hub roster). Wiping them meant one
    // timed-out call emptied the whole page until the next visit.
    try { defs = (await registryList()).agents ?? []; } catch { /* keep last */ }
    try { teams = (await teamsList()).teams ?? []; } catch { /* keep last */ }
    try { skills = (await skillsList()).skills ?? []; } catch { /* keep last */ }
    try { mcps = (await mcpList()).mcp ?? []; } catch { /* keep last */ }
  }
  $effect(() => { if (visible) reload(); });

  // The Hub's agent menu can ask for one agent's editor ("configure agent" —
  // the model's home is here, not quoted in the menu; owner, 2026-08-25).
  // Depends on `defs` too: the request usually arrives WITH the tab switch,
  // before reload() has answered, so it waits for the list and fires once.
  let editReqDone = 0;
  $effect(() => {
    const req = editRequest;
    if (!req || req.n === editReqDone) return;
    const def = defs.find((d) => d.name === req.name);
    if (!def) return;
    editReqDone = req.n;
    startEdit(def);
  });

  function closeAll() {
    editing = null; editingSkill = null; editingMcp = null; editingTeam = null;
    error = ''; info = '';
  }

  // ── Agent teams (board #74) ───────────────────────────────────────────
  // A team is a set of members with roles. A member DERIVES from one of the
  // registry agents (base + role supplement) — the owner's own agents, not
  // redefined — or is CUSTOM to this team (an inline definition: backend,
  // model, effort, persona). Skills/MCP for a custom member are the built-ins;
  // anything richer is a registry agent, which is what `base` is for.
  const TEAM_MAX = 4; // mirrors the server's spawn cap (validated there too)
  const blankMember = () => ({ name: '', base: defs[0]?.name ?? '', role: '', agent: null });
  const blankCustom = () => ({ name: '', backend: 'claude', model: '', effort: '', system: '', skills: '[]', mcp: '[]', can_hire: false });
  function startTeam(team) {
    closeAll();
    teamIsNew = !team;
    let members = [];
    if (team) { try { members = JSON.parse(team.members) ?? []; } catch { members = []; } }
    editingTeam = team
      ? { name: team.name, description: team.description ?? '', members: members.map((m) => ({ name: m.name ?? '', base: m.base ?? '', role: m.role ?? '', agent: m.agent ?? null })) }
      : { name: '', description: '', members: [blankMember()] };
  }
  function addMember() {
    if (!editingTeam || editingTeam.members.length >= TEAM_MAX) return;
    editingTeam.members = [...editingTeam.members, blankMember()];
  }
  function removeMember(i) {
    if (!editingTeam) return;
    editingTeam.members = editingTeam.members.filter((_, k) => k !== i);
  }
  /** The base Select's value: a registry name, or '' for custom. Switching to
   * custom seeds an inline def; switching back drops it. */
  function setBase(i, base) {
    const m = editingTeam.members[i];
    m.base = base;
    m.agent = base ? null : (m.agent ?? blankCustom());
  }
  // Model suggestions per backend for CUSTOM members (the same models_list the
  // agent editor asks); fetched once per backend.
  let modelsByBackend = $state({});
  function ensureModels(backend) {
    if (!backend || backend in modelsByBackend) return;
    modelsByBackend[backend] = [];
    modelsList(backend).then((r) => { modelsByBackend[backend] = r.models ?? []; }).catch(() => {});
  }
  $effect(() => {
    for (const m of editingTeam?.members ?? []) if (!m.base && m.agent) ensureModels(m.agent.backend);
  });
  const teamSavable = $derived(!!editingTeam && editingTeam.name.trim() && editingTeam.members.length > 0
    && editingTeam.members.every((m) => m.name.trim() && (m.base || m.agent)));
  async function saveTeam() {
    if (!editingTeam) return;
    error = '';
    try {
      await teamsSave({
        name: editingTeam.name.trim(),
        description: editingTeam.description.trim(),
        members: JSON.stringify(editingTeam.members.map((m) => ({
          name: m.name.trim(), base: m.base, role: m.role.trim(),
          agent: m.base ? null : { ...m.agent, name: m.name.trim(), model: (m.agent?.model ?? '').trim() },
        }))),
      });
      editingTeam = null;
      await reload();
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  async function removeTeam(name) {
    try {
      await teamsDelete(name);
      if (editingTeam?.name === name) editingTeam = null;
      await reload();
    } catch (e) { error = String(e?.message ?? e); }
  }
  /** One-line summary for the sidebar row: member names, base in parentheses. */
  function teamSummary(team) {
    try { return (JSON.parse(team.members) ?? []).map((m) => m.name).filter(Boolean).join(' · '); } catch { return ''; }
  }

  // The skill's managed files: a chip per file, one previewed at a time.
  // SKILL.md leads; .md renders, anything else shows as monospace text
  // (owner, 2026-08-28: "配置页面可以预览skillmd以及其他资源文件").
  let skFiles = $state([]);
  let skSel = $state('SKILL.md');
  let skText = $state('');
  // The YAML frontmatter duplicates the form fields (name/description) —
  // the preview shows the skill's BODY.
  function stripFrontmatter(md) {
    const m = /^---\n[\s\S]*?\n---\n?/.exec(md);
    return m ? md.slice(m[0].length) : md;
  }
  function loadSkillFiles(name) {
    skFiles = [];
    skSel = 'SKILL.md';
    skText = '';
    skillsFiles(name)
      .then((r) => { if (editingSkill?.name === name) skFiles = r.files ?? []; })
      .catch(() => { skFiles = []; });
    loadSkillFile(name, 'SKILL.md');
  }
  function loadSkillFile(name, path) {
    skSel = path;
    skText = '';
    skillsFile(name, path)
      .then((r) => { if (editingSkill?.name === name && skSel === path) skText = r.content; })
      .catch((e) => { if (editingSkill?.name === name && skSel === path) skText = String(e?.message ?? e); });
  }
  // The description is READING by default — skill descriptions run to
  // paragraphs (they teach the model when to fire) and a one-line input
  // showed a keyhole's worth. Click to edit, blur to fold back
  // (owner, 2026-08-29: "description 应该是多行的 默认是让我浏览 点击才能编辑").
  let descEditing = $state(false);
  function startSkill(sk) {
    closeAll();
    skillIsNew = !sk;
    editingSkill = sk ? { ...sk } : { name: '', source: '', description: '' };
    descEditing = false;
    if (sk) loadSkillFiles(sk.name); else { skFiles = []; skText = ''; }
  }
  let syncing = $state(false);
  async function saveSkill() {
    syncing = true;
    error = '';
    try {
      if (skillIsNew && !editingSkill.name.trim()) {
        // No name = install whatever the source contains (a claude plugin
        // url imports each of its skills; the names come from the skills).
        const r = await skillsImport(editingSkill.source);
        await reload();
        const first = skills.find((x) => x.name === r.imported?.[0]);
        if (first) startSkill(first); else editingSkill = null;
        const skipped = r.skipped?.length ? ` · ${t('skillsSkipped')}: ${r.skipped.join(', ')}` : '';
        info = `${t('skillsImported')}: ${(r.imported ?? []).join(', ') || '—'}${skipped}`;
        return;
      }
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
      loadSkillFiles(editingSkill.name);
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
      : { name: '', backend: 'kiro', model: '', effort: '', system: '', can_hire: false, skillSel: [], mcpSel: [], mcpExtra: [] };
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
        effort: editing.effort ?? '',
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

<div class="agents-root" class:editing={drilled} class:drill-fwd={drillAnim === 'fwd'} class:drill-back={drillAnim === 'back'}>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll subtle-scroll" use:scrollFade>
      {#if shows('agents')}
      {#if !section}<div class="side-h">{t('agentsTitle')}</div>{/if}
      {#each defs as d (d.name)}
        <button class="side-row" class:open={editing?.name === d.name && !isNew} onclick={() => startEdit(d)}>
          {#if backendIcon(d.backend)}<img class="ava" src={backendIcon(d.backend)} alt={d.backend} />{:else}<span class="ava" style:background={backendColor(d.backend)}>{d.name.slice(0, 1).toUpperCase()}</span>{/if}
          <span class="r-name">{d.name}</span>
          <span class="r-backend">{d.backend}</span>
          <!-- can_hire: the boxed M = Manager (board #7) — one atom, the
               words live in the title. -->
          {#if d.can_hire}<span class="m-badge" title={t('agentsManagerHint')} aria-label={t('agentsManagerHint')}>M</span>{/if}
        </button>
      {/each}
      <button class="side-row add" onclick={() => startEdit(null)}>
        <Icon name="plus" size={13} />{t('agentsNew')}
      </button>
      {/if}

      <!-- Agent TEAMS (board #74): a named set of the agents above, each with
           a role. Same row dialect; the second line names the members. -->
      {#if shows('teams')}
      {#if !section}<div class="side-h">{t('teamsTitle')}</div>{/if}
      {#each teams as tm (tm.name)}
        <button class="side-row team-row" class:open={editingTeam?.name === tm.name && !teamIsNew} onclick={() => startTeam(tm)}>
          <Icon name="collab" size={13} />
          <span class="r-col"><span class="r-name">{tm.name}</span><span class="r-sub">{teamSummary(tm)}</span></span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startTeam(null)}>
        <Icon name="plus" size={13} />{t('teamsNew')}
      </button>
      {/if}

      {#if shows('skills')}
      {#if !section}<div class="side-h">{t('skillsTitle')}</div>{/if}
      {#each skills as sk (sk.name)}
        <button class="side-row" class:open={editingSkill?.name === sk.name && !skillIsNew} onclick={() => startSkill(sk)}>
          <Icon name="zap" size={13} />
          <span class="r-name">{sk.name}</span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startSkill(null)}>
        <Icon name="plus" size={13} />{t('skillsNew')}
      </button>
      {/if}

      {#if shows('mcp')}
      {#if !section}<div class="side-h">{t('mcpTitle')}</div>{/if}
      {#each mcps as m (m.name)}
        <button class="side-row" class:open={editingMcp?.name === m.name && !mcpIsNew} onclick={() => startMcp(m)}>
          <Icon name="link" size={13} />
          <span class="r-name">{m.name}</span>
        </button>
      {/each}
      <button class="side-row add" onclick={() => startMcp(null)}>
        <Icon name="plus" size={13} />{t('mcpNew')}
      </button>
      {/if}
    </div>
  </aside>

  <main class="mid">
    {#if editingSkill}
      <div class="page-head">
        <h1>{skillIsNew ? t('skillsNew') : editingSkill.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        <!-- Icon-only, borderless, the label on hover — the same grammar the
             conversation header speaks (owner, 2026-08-28: "能用图标就不用
             文字了…只有鼠标移在上边才有小的文字alt标签"). -->
        {#if !skillIsNew && editingSkill.source !== 'builtin'}
          <!-- A built-in would reseed at the next server start — offering
               delete would be a lie the restart un-tells. -->
          <button class="icon-btn danger" title={t('delete')} aria-label={t('delete')} onclick={() => ask('skill', editingSkill.name)}><Icon name="trash" size={14} /></button>
        {/if}
        {#if !skillIsNew}
          <button class="icon-btn" disabled={syncing} title={t('skillsRefresh')} aria-label={t('skillsRefresh')} onclick={refreshSkill}><Icon name="refresh" size={14} /></button>
        {/if}
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => editingSkill = null}><Icon name="x" size={14} /></button>
        <button class="icon-btn go" disabled={!editingSkill.source.trim() || (!skillIsNew && !editingSkill.name.trim()) || syncing}
          title={skillIsNew ? t('skillsImport') : t('save')} aria-label={skillIsNew ? t('skillsImport') : t('save')}
          onclick={saveSkill}><Icon name={skillIsNew ? 'download' : 'check'} size={14} /></button>
        </div>
      </div>
      <div class="editor">
        {#if error}<div class="err">{error}</div>{/if}
        {#if info}<p class="hint">{info}</p>{/if}
        <label>{t('agentsName')}
          <input bind:value={editingSkill.name} disabled={!skillIsNew} placeholder="git-review" />
        </label>
        {#if skillIsNew}
          <p class="hint">{t('skillsImportHint')}</p>
        {/if}
        <label>{t('skillsSource')}
          <input bind:value={editingSkill.source} disabled={editingSkill.source === 'builtin'} placeholder="https://github.com/org/repo/tree/main/skills/git-review 或 /abs/local/dir" />
        </label>
        {#if editingSkill.source === 'builtin'}
          <p class="hint">{t('skillsBuiltin')}</p>
        {/if}
        <label>{t('skillsDesc')}
          {#if skillIsNew || descEditing}
            <!-- svelte-ignore a11y_autofocus — the user just clicked "edit
                 this text"; focusing anywhere else would drop the intent. -->
            <textarea rows="4" bind:value={editingSkill.description} autofocus={descEditing}
              onblur={() => descEditing = false}></textarea>
          {:else}
            <button class="desc-view" type="button" title={t('edit')} onclick={() => descEditing = true}
              >{editingSkill.description || '—'}</button>
          {/if}
        </label>
        {#if editingSkill.synced_at}
          <p class="hint">{t('skillsSynced')} {new Date(editingSkill.synced_at * 1000).toLocaleString()}</p>
        {/if}
        <p class="hint">{t('skillsHint')}</p>
        {#if !skillIsNew && skFiles.length}
          <div class="md-preview">
            <div class="side-h">{t('skillsFilesTitle')}</div>
            <!-- A quiet list, not a chip cloud: file paths are reading
                 material (owner, 2026-08-28: "可以是小的列表组件展示").
                 A single SKILL.md still shows its one row — hiding the list
                 read as "file 没有写" (owner, 2026-08-29). -->
              <div class="file-list" role="listbox" aria-label={t('skillsFilesTitle')}>
                {#each skFiles as f (f.path)}
                  <button class="file-row" class:sel={skSel === f.path} type="button" role="option" aria-selected={skSel === f.path}
                    onclick={() => loadSkillFile(editingSkill.name, f.path)}>
                    <span class="f-path">{f.path}</span>
                    <span class="f-size">{f.size < 1024 ? `${f.size} B` : `${Math.round(f.size / 1024)} KB`}</span>
                  </button>
                {/each}
              </div>
            {#if skSel.endsWith('.md')}
              <div class="md md-doc">{@html renderMarkdown(skSel === 'SKILL.md' ? stripFrontmatter(skText) : skText)}</div>
            {:else}
              <pre class="file-pre">{skText}</pre>
            {/if}
          </div>
        {/if}
      </div>
    {:else if editingMcp}
      <div class="page-head">
        <h1>{mcpIsNew ? t('mcpNew') : editingMcp.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !mcpIsNew}
          <button class="icon-btn danger" title={t('delete')} aria-label={t('delete')} onclick={() => ask('mcp', editingMcp.name)}><Icon name="trash" size={14} /></button>
        {/if}
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => editingMcp = null}><Icon name="x" size={14} /></button>
        <button class="icon-btn go" disabled={!editingMcp.name.trim()} title={t('save')} aria-label={t('save')} onclick={saveMcp}><Icon name="check" size={14} /></button>
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
    {:else if editingTeam}
      <div class="page-head">
        <h1>{teamIsNew ? t('teamsNew') : editingTeam.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !teamIsNew}
          <button class="icon-btn danger" title={t('delete')} aria-label={t('delete')} onclick={() => ask('team', editingTeam.name)}><Icon name="trash" size={14} /></button>
        {/if}
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => editingTeam = null}><Icon name="x" size={14} /></button>
        <button class="icon-btn go" disabled={!teamSavable} title={t('save')} aria-label={t('save')} onclick={saveTeam}><Icon name="check" size={14} /></button>
        </div>
      </div>
      <div class="editor">
        {#if error}<div class="err">{error}</div>{/if}
        <label>{t('teamsName')}
          <input bind:value={editingTeam.name} disabled={!teamIsNew} placeholder="dev-squad" />
        </label>
        <label>{t('teamsDesc')}
          <textarea rows="2" bind:value={editingTeam.description} placeholder={t('teamsDescPh')}></textarea>
        </label>
        <div class="pick-block">
          <span class="pick-label">{t('teamsMembers')}</span>
          {#each editingTeam.members as m, i (i)}
            <div class="member">
              <div class="member-head">
                <input class="member-name" bind:value={m.name} placeholder="dev" aria-label={t('teamsMemberName')} />
                <Select value={m.base} dense ariaLabel={t('teamsBase')}
                  options={[...defs.map((d) => ({ value: d.name, label: d.name, icon: backendIcon(d.backend) ?? undefined })), { value: '', label: t('teamsCustom') }]}
                  onchange={(v) => setBase(i, v)} />
                <button class="icon-btn danger" title={t('teamsRemoveMember')} aria-label={t('teamsRemoveMember')} disabled={editingTeam.members.length <= 1} onclick={() => removeMember(i)}><Icon name="x" size={13} /></button>
              </div>
              {#if !m.base && m.agent}
                <!-- A team-only member: the agent editor's identity fields, inline. -->
                <div class="row2">
                  <label>{t('agentsBackend')}
                    <Select bind:value={m.agent.backend} dense ariaLabel={t('agentsBackend')}
                      options={BACKENDS.map((b) => ({ value: b, icon: backendIcon(b) ?? undefined }))} />
                  </label>
                  <label>{t('agentsModel')}
                    <Select bind:value={m.agent.model} editable dense options={modelsByBackend[m.agent.backend] ?? []}
                      placeholder={t('agentsModelDefault')} ariaLabel={t('agentsModel')} />
                  </label>
                </div>
                <div class="row2">
                  <label>{t('agentsEffort')}
                    <Select bind:value={m.agent.effort} dense
                      options={[{ value: '', label: t('agentsModelDefault') }, ...(EFFORTS[m.agent.backend] ?? [])]}
                      ariaLabel={t('agentsEffort')} />
                  </label>
                  <div></div>
                </div>
                <label>{t('agentsSystem')}
                  <textarea rows="3" bind:value={m.agent.system} placeholder={t('agentsSystemPh')}></textarea>
                </label>
              {/if}
              <label>{t('teamsRole')}
                <textarea rows="2" bind:value={m.role} placeholder={t('teamsRolePh')}></textarea>
              </label>
            </div>
          {/each}
          {#if editingTeam.members.length < TEAM_MAX}
            <button class="pick" type="button" onclick={addMember}><Icon name="plus" size={11} />{t('teamsAddMember')}</button>
          {:else}
            <p class="hint">{t('teamsMax').replace('{n}', String(TEAM_MAX))}</p>
          {/if}
        </div>
        <p class="hint">{t('teamsHint')}</p>
      </div>
    {:else if editing}
      <div class="page-head">
        <h1>{isNew ? t('agentsNew') : editing.name}</h1>
        <span class="spacer"></span>
        <div class="head-acts">
        {#if !isNew}
          <button class="icon-btn danger" title={t('delete')} aria-label={t('delete')} onclick={() => ask('agent', editing.name)}><Icon name="trash" size={14} /></button>
        {/if}
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => editing = null}><Icon name="x" size={14} /></button>
        <button class="icon-btn go" disabled={!editing.name.trim()} title={t('save')} aria-label={t('save')} onclick={save}><Icon name="check" size={14} /></button>
        </div>
      </div>
      <div class="editor">
        {#if error}<div class="err">{error}</div>{/if}

        <label>{t('agentsName')}
          <input bind:value={editing.name} disabled={!isNew} placeholder="reviewer" />
        </label>
        <div class="row2">
          <label>{t('agentsBackend')}
            <Select bind:value={editing.backend} dense ariaLabel={t('agentsBackend')}
              options={BACKENDS.map((b) => ({ value: b, icon: backendIcon(b) ?? undefined }))} />
          </label>
          <label>{t('agentsModel')}
            <!-- Editable Select, not a native <datalist>: the OS suggestion
                 popup is the seam the shared dropdown exists to remove (owner,
                 2026-08-24: "模型选择下拉框明显不对"). The value stays free
                 text — an id we cannot enumerate is still typeable, and
                 registry_save remains the authority that rejects a bad one. -->
            <Select bind:value={editing.model} editable dense options={models}
              placeholder={t('agentsModelDefault')} ariaLabel={t('agentsModel')} />
          </label>
        </div>
        <div class="row2">
          <label>{t('agentsEffort')}
            <!-- A fixed enum per backend (the CLI's own levels), so a Select,
                 not free text: a typo'd effort is a warning above the splash
                 and a silent fallback to the default. '' = backend default,
                 same contract as the model. -->
            <Select bind:value={editing.effort} dense
              options={[{ value: '', label: t('agentsModelDefault') }, ...(EFFORTS[editing.backend] ?? [])]}
              ariaLabel={t('agentsEffort')} />
          </label>
          <div></div>
        </div>
        <!-- can_hire is a MEMBERSHIP toggle like the skill/MCP picks below, so
             it wears the same .pick chip — a native checkbox next to custom
             fields read as a different species (owner, 2026-08-24: "can hire
             样式也不和谐"). -->
        <div class="pick-block">
          <button class="pick" class:sel={editing.can_hire} type="button"
            aria-pressed={editing.can_hire} title={t('agentsManagerHint')}
            onclick={() => editing.can_hire = !editing.can_hire}>
            <Icon name={editing.can_hire ? 'check' : 'bot'} size={11} /><span class="m-badge">M</span>{t('agentsManager')}
          </button>
        </div>
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
      <div class="page-head"><h1>{t(SECTION_META[section]?.[0] ?? 'agentsTitle')}</h1></div>
      <div class="placeholder">
        <p class="hint">{t(SECTION_META[section]?.[1] ?? 'agentsHint')}</p>
      </div>
    {/if}
  </main>
</div>

<ConfirmDialog open={!!pending} busy={removing}
  title={pending ? t(COPY[pending.kind].title).replace('{name}', pending.name) : ''}
  note={pending ? t(COPY[pending.kind].note) : ''}
  confirmLabel={t('delete')}
  onconfirm={runPending} oncancel={() => (pending = null)} />

<style>
  .agents-root { height: 100%; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; background: var(--bg); }
  @media (max-width: 760px) {
    .agents-root { grid-template-columns: minmax(0, 1fr); }
    /* Compact: the list is the page; editing takes the screen. A full-width
       list has no column beside it, so its divider would sit at the screen's
       right edge as a stray line (owner, 2026-08-27). */
    .sidebar { border-right: none; }
    .agents-root.editing .sidebar { display: none; }
    .agents-root:not(.editing) .mid { display: none; }
    /* Drill motion: same 120ms grammar as the app-level page slide. */
    .agents-root.drill-fwd .mid { animation: drill-in-right 0.12s linear; }
    .agents-root.drill-back .sidebar { animation: drill-in-left 0.12s linear; }
  }
  @keyframes drill-in-right { from { transform: translateX(40%); } to { transform: none; } }
  @keyframes drill-in-left  { from { transform: translateX(-40%); } to { transform: none; } }
  @media (prefers-reduced-motion: reduce) {
    .agents-root.drill-fwd .mid, .agents-root.drill-back .sidebar { animation: none; }
  }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .r-name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; }
  .r-backend { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3); flex: none; }
  /* A wash, not a drawn frame: borders on inner micro atoms read as chrome
     (owner, 2026-08-24 audit; same rule as the sys-line atoms). */
  /* The Manager atom (board #7): a boxed M, shared verbatim with Hub.svelte
     (no app.css edits allowed there either — the source test pins the two
     declarations to the same text). Words live in title/aria. */
  .m-badge { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 15px; height: 15px; border: 1px solid var(--accent); border-radius: 4px; color: var(--accent); font-size: var(--fs-micro); font-weight: 700; line-height: 1; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .spacer { flex: 1; }
  /* The head actions move as ONE block: on a phone they wrap under the
     title together instead of scattering one button per row. */
  .head-acts { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; margin-left: auto; }
  /* The confirm among equals: same borderless button, accent ink says "go". */
  .head-acts :global(.icon-btn.go:not(:disabled)) { color: var(--accent); }
  .head-acts :global(.icon-btn.go:not(:disabled):hover) { background: var(--accent-bg); }
  /* Skill files as a quiet list — rows in the wash hover family, the
     selected one in the accent wash (same states the sidebar rows speak). */
  .file-list {
    display: flex; flex-direction: column; overflow: hidden auto; max-height: 200px;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
  }
  .file-row {
    display: flex; align-items: center; gap: 8px; padding: 4px 10px;
    background: none; border: none; cursor: pointer; text-align: left;
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text);
    transition: background var(--t-fast), color var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  .file-row:hover { background: var(--surface2); }
  .file-row.sel { background: var(--accent-bg); color: var(--accent); }
  .f-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .f-size { margin-left: auto; flex: none; color: var(--text3); font-size: var(--fs-micro); }
  .placeholder { flex: 1; display: grid; place-items: center; }
  .hint { color: var(--text3); font-size: var(--fs-meta); margin: 0; line-height: 1.5; max-width: 420px; }
  /* Description at rest: the text itself, whole and wrapped; the wash on
     hover says "tap to edit" without drawing an input around reading. */
  .desc-view {
    background: none; border: 1px solid transparent; border-radius: var(--ui-radius-control);
    padding: 6px 8px; margin: 0; text-align: left; cursor: text;
    font: inherit; font-size: var(--fs-ui); color: var(--text); line-height: 1.55;
    white-space: pre-wrap; overflow-wrap: anywhere;
    transition: background var(--t-fast), border-color var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  .desc-view:hover { background: var(--surface2); }

  .editor { flex: 1; overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; max-width: 720px; }
  .err { color: var(--danger); font-size: var(--fs-ui); background: var(--danger-bg); border-radius: var(--ui-radius-row); padding: 8px 12px; }
  /* Field captions are QUIET — the field carries the content, the label only
     names it (the dialog dialect's .dlg-note voice). fs-ui labels over
     fs-body inputs were the page reading a size too big everywhere
     (owner, 2026-08-24: "很多字号有点大很奇怪，也和页面风格不符"). */
  /* The Settings grammar (the owner's reference): a full-ink 600-weight
     title leads each field, hints are --fs-meta grey UNDER it, values are
     full ink. One grey for everything left nothing leading. */
  label { display: flex; flex-direction: column; gap: 4px; color: var(--text); font-size: var(--fs-ui); font-weight: 600; }
  label > input, label > textarea, label > .desc-view { font-weight: 400; }
  /* The dense field dialect (Team's template editor / the shared Select's
     `dense`), so every box in the form is ONE species at ONE size. */
  input, textarea { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: var(--ui-radius-control); color: var(--text); padding: 6px 9px; font-size: var(--fs-ui); outline: none; font-family: inherit; }
  input:focus, textarea:focus { border-color: var(--accent); }
  input:disabled { opacity: 0.5; }
  textarea { resize: vertical; line-height: 1.5; }
  textarea.mono { font-family: ui-monospace, Menlo, monospace; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  /* A team member is one bordered panel in the editor's own dialect (board
     #74): the head row is name + base + remove, the body the role (and, for a
     custom member, the agent identity fields). */
  .member { display: flex; flex-direction: column; gap: 10px; padding: 10px 12px; border: 1px solid var(--border); border-radius: var(--ui-radius-row); background: var(--surface); }
  .member-head { display: grid; grid-template-columns: minmax(80px, 1fr) minmax(140px, 1.4fr) auto; gap: 8px; align-items: center; }
  .member-name { min-width: 0; }
  .team-row { align-items: flex-start; }
  .r-col { display: flex; flex-direction: column; min-width: 0; gap: 1px; }
  .r-sub { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-micro); color: var(--text3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick-block { display: flex; flex-direction: column; gap: 6px; }
  .pick-label { color: var(--text); font-size: var(--fs-ui); font-weight: 600; }
  .pick-row { display: flex; flex-wrap: wrap; gap: 6px; }
  .pick {
    display: flex; align-items: center; gap: 5px;
    background: var(--surface); border: 1px solid var(--border); border-radius: 999px;
    color: var(--text2); padding: 4px 10px; font-size: var(--fs-ui); cursor: pointer;
    transition: border-color var(--t-fast), color var(--t-fast);
  }
  .pick:hover { border-color: var(--input-border); }
  .pick.sel { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .md-preview { border-top: 1px solid var(--border2); margin-top: 6px; display: flex; flex-direction: column; gap: 8px; }
  .file-pre {
    margin: 0; padding: 8px 10px; overflow: auto; max-height: 60vh;
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub);
    color: var(--text); background: var(--code-bg, var(--surface));
    border-radius: var(--ui-radius-control); white-space: pre;
  }
  .md-doc {
    background: var(--surface); border: 1px solid var(--border2); border-radius: var(--ui-radius-panel);
    padding: 12px 14px; font-size: var(--fs-body); color: var(--text); line-height: 1.55;
    overflow-wrap: anywhere;
  }
  /* iOS zooms a focused control below 16px — but ONLY iOS. On Android the
     blanket bump made the name input and the prompt textarea 16px while the
     dense Selects stayed at --fs-ui (.dense is 0,2,0; a media query adds no
     specificity), which is the "字号还是偏大不一致" the owner saw. The
     -webkit-touch-callout gate is iOS-family only, so the bump now fires
     exactly where the auto-zoom exists (owner, 2026-08-24). */
  @supports (-webkit-touch-callout: none) {
    @media (max-width: 760px) {
      input, textarea { font-size: var(--fs-input-touch); }
    }
  }
</style>
