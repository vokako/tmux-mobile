<script>
  import { listSessions, listPanes, newSession, killSession, newWindow, killWindow, fsList } from './ws.js';
  import Icon from './Icon.svelte';

  let { openTerminal, activeTarget = '', visible = false } = $props();

  let sessions = $state([]);
  let expanded = $state({});
  let panes = $state({});
  let error = $state('');
  let newName = $state('');
  let newPath = $state('');
  let newCmd = $state('');
  let showNew = $state(false);

  // Folder picker
  let showPicker = $state(false);
  let pickerPath = $state('');
  let pickerEntries = $state([]);

  async function openPicker() {
    showPicker = true;
    await loadPicker(newPath || '~');
  }

  async function loadPicker(path) {
    try {
      const r = await fsList(path, false);
      pickerPath = path;
      pickerEntries = r.entries.filter(e => e.type === 'dir').sort((a, b) => a.name.localeCompare(b.name));
    } catch {}
  }

  function pickerUp() {
    const parent = pickerPath.replace(/\/[^/]+\/?$/, '') || '/';
    loadPicker(parent);
  }

  let pickerBreadcrumbs = $derived.by(() => {
    if (!pickerPath) return [];
    const parts = pickerPath.split('/').filter(Boolean);
    return parts.map((name, i) => ({ name, path: '/' + parts.slice(0, i + 1).join('/') }));
  });

  let pickerPathEl = $state(null);
  $effect(() => {
    pickerPath;
    setTimeout(() => { if (pickerPathEl) pickerPathEl.scrollLeft = pickerPathEl.scrollWidth; }, 0);
  });

  function pickerSelect() {
    newPath = pickerPath;
    showPicker = false;
  }

  function scrollIntoView(el) {
    setTimeout(() => el.scrollIntoView({ behavior: 'smooth', block: 'end' }), 50);
  }

  // Refresh when page becomes visible
  $effect(() => { if (visible) refresh(); });

  async function refresh() {
    try {
      let list = await listSessions();
      // Active session first, then attached, then rest
      const activeSession = activeTarget.split(':')[0];
      sessions = list.sort((a, b) => {
        if (a.name === activeSession) return -1;
        if (b.name === activeSession) return 1;
        return (b.attached ? 1 : 0) - (a.attached ? 1 : 0);
      });
      error = '';
      for (const s of sessions) {
        if (!expanded[s.name]) {
          try {
            panes[s.name] = await listPanes(s.name);
            expanded[s.name] = true;
          } catch (_) {}
        }
      }
    } catch (e) {
      error = e.message;
    }
  }

  async function toggleSession(name) {
    if (expanded[name]) {
      expanded[name] = false;
      return;
    }
    try {
      panes[name] = await listPanes(name);
      expanded[name] = true;
    } catch (e) {
      error = e.message;
    }
  }

  async function createSession() {
    if (!newName.trim()) return;
    try {
      await newSession(newName.trim(), newPath.trim() || undefined, newCmd.trim() || undefined);
      newName = ''; newPath = ''; newCmd = ''; showNew = false;
      await refresh();
    } catch (e) {
      error = e.message;
    }
  }

  let refreshing = $state(false);
  let confirmKillWindow = $state(null);
  let confirmKill = $state(null);

  async function removeWindow(target, session) {
    if (confirmKillWindow !== target) {
      confirmKillWindow = target;
      setTimeout(() => { if (confirmKillWindow === target) confirmKillWindow = null; }, 3000);
      return;
    }
    confirmKillWindow = null;
    await killWindow(target);
    panes[session] = await listPanes(session);
  }
  async function doRefresh() {
    refreshing = true;
    await refresh();
    setTimeout(() => refreshing = false, 600);
  }

  async function removeSession(name) {
    if (confirmKill !== name) {
      confirmKill = name;
      setTimeout(() => { if (confirmKill === name) confirmKill = null; }, 3000);
      return;
    }
    confirmKill = null;
    try {
      await killSession(name);
      await refresh();
    } catch (e) {
      error = e.message;
    }
  }
</script>

<div class="sessions">
  {#if error}
    <div class="error">{error}</div>
  {/if}

  <div class="list">
    {#each sessions as s}
      <div class="session" class:expanded={expanded[s.name]} class:active-session={activeTarget.startsWith(s.name + ':')}>
        <div class="session-row" role="button" tabindex="0" onclick={() => toggleSession(s.name)} onkeydown={(e) => e.key === 'Enter' && toggleSession(s.name)}>
          <div class="session-info">
            <span class="indicator" class:attached={s.attached}></span>
            <span class="name">{s.name}</span>
          </div>
          <div class="session-meta">
            <span class="badge">{s.windows} {s.windows === 1 ? 'window' : 'windows'}</span>
            <button class="kill" class:confirm={confirmKill === s.name} onclick={(e) => { e.stopPropagation(); removeSession(s.name); }} aria-label="Kill session">
              {#if confirmKill === s.name}
                <span class="kill-text">tap to kill</span>
              {:else}
                <span class="kill-icon"><Icon name="x" size={11} /></span>
              {/if}
            </button>
          </div>
        </div>
        {#if expanded[s.name] && panes[s.name]}
          <div class="pane-list">
            {#each panes[s.name] as p}
              <div class="pane-row">
                <button class="pane" class:active-pane={activeTarget === `${p.session}:${p.window}.${p.pane}`} onclick={() => openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command)}>
                  <span class="pane-id">:{p.window}.{p.pane}</span>
                  <span class="pane-cmd">{p.current_command}</span>
                  <span class="pane-size">{p.width}×{p.height}</span>
                </button>
                <button class="pane-kill" class:confirm={confirmKillWindow === `${s.name}:${p.window}`} onclick={() => removeWindow(`${s.name}:${p.window}`, s.name)}>
                  {#if confirmKillWindow === `${s.name}:${p.window}`}
                    <span class="kill-text">del</span>
                  {:else}
                    <Icon name="x" size={10} />
                  {/if}
                </button>
              </div>
            {/each}
            <button class="pane-add" onclick={async () => { await newWindow(s.name); panes[s.name] = await listPanes(s.name); }}>
              <Icon name="plus" size={12} /> Window
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#if showNew}
    <div class="new-form" use:scrollIntoView>
      <input type="text" bind:value={newName} placeholder="Session name" onkeydown={(e) => e.key === 'Enter' && createSession()} autocapitalize="off" />
      <div class="cmd-row-new">
        <input type="text" bind:value={newPath} placeholder="Working directory (optional)" autocapitalize="off" />
        <button class="preset-btn" onclick={openPicker}><Icon name="folder" size={14} /></button>
      </div>
      {#if showPicker}
        <div class="picker">
          <div class="picker-header">
            <button class="picker-btn" onclick={pickerUp}><Icon name="folder-up" size={13} /></button>
            <div class="picker-path" bind:this={pickerPathEl}>
              <button class="picker-seg" onclick={() => loadPicker('/')}>/</button>
              {#each pickerBreadcrumbs as bc}
                <button class="picker-seg" onclick={() => loadPicker(bc.path)}>{bc.name}</button>
                <span class="picker-sep">/</span>
              {/each}
            </div>
            <button class="picker-btn pick-ok" onclick={pickerSelect}><Icon name="check" size={13} /></button>
          </div>
          <div class="picker-list">
            {#each pickerEntries as e}
              <button class="picker-item" onclick={() => loadPicker(e.path)}>
                <Icon name="folder" size={13} /> {e.name}
              </button>
            {/each}
            {#if !pickerEntries.length}
              <div class="picker-empty">No subdirectories</div>
            {/if}
          </div>
        </div>
      {/if}
      <div class="cmd-row-new">
        <input type="text" bind:value={newCmd} placeholder="Command (optional)" autocapitalize="off" />
        <button class="preset-btn" class:active={newCmd === 'kiro-cli-chat chat -a'} onclick={() => newCmd = newCmd === 'kiro-cli-chat chat -a' ? '' : 'kiro-cli-chat chat -a'}><img src="/assets/kiro.svg" alt="Kiro" width="16" height="16" /></button>
      </div>
      <button class="create-btn" onclick={createSession} disabled={!newName.trim()}>Create</button>
    </div>
  {/if}

  <div class="bottom-bar">
    <button class="new-btn" onclick={() => showNew = !showNew}>
      <Icon name="plus" size={16} /> New Session
    </button>
    <button class="refresh-icon" class:spinning={refreshing} onclick={doRefresh}><Icon name="refresh" size={16} /></button>
  </div>
</div>

<style>
  .sessions {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
    flex: 1;
    -webkit-overflow-scrolling: touch;
  }

  .list { display: flex; flex-direction: column; gap: 8px; }

  .session {
    background: transparent;
    border: 1px solid var(--border2);
    border-radius: 14px;
    overflow: hidden;
    transition: all 0.2s ease;
  }
  .session:active { transform: scale(0.99); }
  .session.expanded {
    border-color: var(--border);
  }
  .session.active-session {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .session-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 14px;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .session-info {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .indicator {
    width: 8px; height: 8px;
    border-radius: 50%;
    background: var(--text3);
    flex-shrink: 0;
    transition: all 0.2s ease;
  }
  .indicator.attached {
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent-glow);
  }

  .name {
    font-weight: 600;
    font-size: 15px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .session-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .badge {
    font-size: 11px;
    font-weight: 500;
    color: var(--text3);
    background: var(--input-bg);
    padding: 3px 8px;
    border-radius: 6px;
  }

  .kill {
    width: 26px; height: 26px;
    background: transparent;
    border: none;
    color: var(--text3);
    cursor: pointer;
    border-radius: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .kill:active {
    background: var(--danger-bg);
    color: var(--danger);
  }
  .kill.confirm {
    background: var(--danger-bg);
    color: var(--danger);
    width: auto;
    padding: 0 8px;
  }
  .kill-text {
    font-size: 11px;
    font-weight: 600;
  }
  .kill-icon { font-size: 11px; }

  .pane-list {
    border-top: 1px solid var(--border2);
  }

  .pane {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 12px 14px 12px 32px;
    background: none;
    border: none;
    border-bottom: 1px solid var(--border2);
    color: var(--text);
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    transition: background 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .pane:active { background: var(--accent-bg); }
  .pane.active-pane { background: var(--accent-bg); }
  .pane:last-child { border-bottom: none; }
  .pane-row {
    display: flex; align-items: center;
    border-bottom: 1px solid var(--border2);
  }
  .pane-row:last-of-type { border-bottom: none; }
  .pane-row .pane { border-bottom: none; }
  .pane-kill {
    padding: 8px; border: none; background: none; color: var(--text3);
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .pane-kill:active { color: var(--danger); }
  .pane-kill.confirm { color: var(--danger); }
  .pane-add {
    display: flex; align-items: center; justify-content: center; gap: 4px;
    width: 100%; padding: 8px; border: none; border-top: 1px solid var(--border2);
    background: none; color: var(--text3); font-size: 12px; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .pane-add:active { color: var(--accent); }

  .pane-id {
    font-family: 'SF Mono', Menlo, monospace;
    color: var(--accent);
    font-weight: 500;
    font-size: 12px;
    min-width: 36px;
  }
  .pane-cmd {
    color: var(--text2);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pane-size {
    font-family: 'SF Mono', Menlo, monospace;
    color: var(--text3);
    font-size: 11px;
  }

  .bottom-bar { display: flex; gap: 8px; }
  .new-btn {
    flex: 1; padding: 12px; border: 1px dashed var(--border); border-radius: 14px;
    background: none; color: var(--text2); font-size: 14px; font-weight: 500;
    cursor: pointer; display: flex; align-items: center; justify-content: center; gap: 6px;
    -webkit-tap-highlight-color: transparent;
  }
  .new-btn:active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .refresh-icon {
    width: 46px; border: 1px solid var(--border); border-radius: 14px;
    background: none; color: var(--text2); cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent; transition: color 0.2s;
  }
  .refresh-icon:active { color: var(--accent); }
  .refresh-icon.spinning { color: var(--accent); animation: spin 0.6s ease; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .new-form {
    display: flex; flex-direction: column; gap: 8px;
    padding: 12px; background: var(--surface); border: 1px solid var(--border);
    border-radius: 14px;
  }
  .new-form input {
    padding: 10px 12px; border: 1px solid var(--border2); border-radius: 10px;
    background: var(--input-bg); color: var(--text); font-size: 14px;
    outline: none; -webkit-appearance: none;
  }
  .new-form input:focus { border-color: var(--accent); }
  .new-form input::placeholder { color: var(--text3); }
  .cmd-row-new { display: flex; gap: 8px; }
  .cmd-row-new input { flex: 1; min-width: 0; }
  .preset-btn {
    padding: 0 12px; border: 1px solid var(--border2); border-radius: 10px;
    background: var(--input-bg); color: var(--text2); font-size: 13px;
    font-weight: 600; cursor: pointer; white-space: nowrap;
    -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center;
  }
  .preset-btn.active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }

  .picker {
    border: 1px solid var(--border2); border-radius: 10px; overflow: hidden;
    background: var(--input-bg);
  }
  .picker-header {
    display: flex; align-items: center; gap: 6px; padding: 6px 8px;
    border-bottom: 1px solid var(--border2);
  }
  .picker-path {
    flex: 1; display: flex; align-items: center; gap: 1px;
    overflow-x: auto; scrollbar-width: none;
    font-family: 'SF Mono', Menlo, monospace; font-size: 12px;
    -webkit-overflow-scrolling: touch;
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
    padding: 5px; border: none; border-radius: 6px; background: var(--surface2);
    color: var(--text2); cursor: pointer; display: flex;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-btn:active { color: var(--accent); }
  .pick-ok { background: var(--accent-bg); color: var(--accent); }
  .picker-list { max-height: 180px; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .picker-item {
    display: flex; align-items: center; gap: 8px; width: 100%; padding: 10px 12px;
    border: none; border-bottom: 1px solid var(--border2); background: none;
    color: var(--accent); font-size: 13px; cursor: pointer; text-align: left;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-item:active { background: var(--accent-bg); }
  .picker-item:last-child { border-bottom: none; }
  .picker-empty { padding: 12px; text-align: center; color: var(--text3); font-size: 12px; }
  .create-btn {
    padding: 10px; border: none; border-radius: 10px;
    background: var(--accent-bg); color: var(--accent);
    font-size: 14px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .create-btn:active:not(:disabled) { opacity: 0.8; }
  .create-btn:disabled { opacity: 0.3; cursor: default; }

  .error {
    color: var(--danger);
    font-size: 13px;
    padding: 10px 14px;
    background: var(--danger-bg);
    border: 1px solid var(--danger);
    border-radius: 10px;
  }

</style>
