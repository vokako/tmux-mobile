<script>
  import { listSessions, listPanes, newSession, killSession } from './ws.js';
  import Icon from './Icon.svelte';

  let { openTerminal, activeTarget = '', onDisconnect = () => {}, visible = false } = $props();

  let sessions = $state([]);
  let expanded = $state({});
  let panes = $state({});
  let error = $state('');
  let newName = $state('');

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
      await newSession(newName.trim());
      newName = '';
      await refresh();
    } catch (e) {
      error = e.message;
    }
  }

  let confirmKill = $state(null);
  let confirmDisconnect = $state(false);

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
  <div class="new-session">
    <input type="text" bind:value={newName} placeholder="New session name…" onkeydown={(e) => e.key === 'Enter' && createSession()} />
    <button onclick={createSession} disabled={!newName.trim()}>
      <span>+</span>
    </button>
    <button class="refresh-btn" onclick={refresh}><Icon name="refresh" size={14} /></button>
  </div>

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
              <button class="pane" class:active-pane={activeTarget === `${p.session}:${p.window}.${p.pane}`} onclick={() => openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command)}>
                <span class="pane-id">:{p.window}.{p.pane}</span>
                <span class="pane-cmd">{p.current_command}</span>
                <span class="pane-size">{p.width}×{p.height}</span>
                <span class="pane-arrow">→</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  <button class="disconnect-btn" class:confirm={confirmDisconnect} onclick={() => {
    if (confirmDisconnect) { onDisconnect(); confirmDisconnect = false; }
    else { confirmDisconnect = true; setTimeout(() => confirmDisconnect = false, 3000); }
  }}>
    {confirmDisconnect ? 'tap to disconnect' : 'Disconnect'}
  </button>
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
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: 14px;
    overflow: hidden;
    transition: all 0.2s ease;
  }
  .session:active { transform: scale(0.99); }
  .session.expanded {
    border-color: rgba(0, 212, 255, 0.15);
    box-shadow: 0 0 20px rgba(0, 212, 255, 0.05);
  }
  .session.active-session {
    border-color: rgba(0, 212, 255, 0.25);
    background: rgba(0, 212, 255, 0.03);
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
    background: rgba(255, 255, 255, 0.15);
    flex-shrink: 0;
    transition: all 0.2s ease;
  }
  .indicator.attached {
    background: #00d4ff;
    box-shadow: 0 0 8px rgba(0, 212, 255, 0.5);
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
    color: rgba(226, 232, 240, 0.35);
    background: rgba(255, 255, 255, 0.04);
    padding: 3px 8px;
    border-radius: 6px;
  }

  .kill {
    width: 26px; height: 26px;
    background: transparent;
    border: none;
    color: rgba(226, 232, 240, 0.2);
    cursor: pointer;
    border-radius: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .kill:active {
    background: rgba(255, 80, 80, 0.15);
    color: #ff5050;
  }
  .kill.confirm {
    background: rgba(255, 80, 80, 0.15);
    color: #ff5050;
    width: auto;
    padding: 0 8px;
  }
  .kill-text {
    font-size: 11px;
    font-weight: 600;
  }
  .kill-icon { font-size: 11px; }

  .pane-list {
    border-top: 1px solid rgba(255, 255, 255, 0.04);
  }

  .pane {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 12px 14px 12px 32px;
    background: none;
    border: none;
    border-bottom: 1px solid rgba(255, 255, 255, 0.03);
    color: #e2e8f0;
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    transition: background 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .pane:active { background: rgba(0, 212, 255, 0.06); }
  .pane.active-pane { background: rgba(0, 212, 255, 0.08); }
  .pane:last-child { border-bottom: none; }

  .pane-id {
    font-family: 'SF Mono', Menlo, monospace;
    color: #00d4ff;
    font-weight: 500;
    font-size: 12px;
    min-width: 36px;
  }
  .pane-cmd {
    color: rgba(226, 232, 240, 0.45);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pane-size {
    font-family: 'SF Mono', Menlo, monospace;
    color: rgba(226, 232, 240, 0.2);
    font-size: 11px;
  }
  .pane-arrow {
    color: rgba(0, 212, 255, 0.4);
    font-size: 12px;
  }

  .new-session {
    display: flex;
    gap: 8px;
  }
  .new-session input {
    flex: 1;
    padding: 11px 14px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.03);
    color: #e2e8f0;
    font-size: 14px;
    outline: none;
    transition: all 0.2s ease;
    -webkit-appearance: none;
  }
  .new-session input:focus {
    border-color: rgba(0, 212, 255, 0.3);
    box-shadow: 0 0 0 3px rgba(0, 212, 255, 0.06);
  }
  .new-session input::placeholder { color: rgba(226, 232, 240, 0.2); }

  .new-session button {
    width: 44px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    background: rgba(0, 212, 255, 0.08);
    color: #00d4ff;
    font-size: 20px;
    font-weight: 300;
    cursor: pointer;
    transition: all 0.2s ease;
    -webkit-tap-highlight-color: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .new-session button:active:not(:disabled) {
    background: rgba(0, 212, 255, 0.15);
    transform: scale(0.95);
  }
  .new-session button:disabled { opacity: 0.3; cursor: default; }
  .refresh-btn {
    width: 44px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.03);
    color: rgba(226, 232, 240, 0.4);
    cursor: pointer;
    display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .refresh-btn:active { background: rgba(0, 212, 255, 0.1); color: #00d4ff; }

  .error {
    color: #ff5050;
    font-size: 13px;
    padding: 10px 14px;
    background: rgba(255, 80, 80, 0.06);
    border: 1px solid rgba(255, 80, 80, 0.12);
    border-radius: 10px;
  }

  .disconnect-btn {
    width: 100%;
    padding: 12px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.03);
    color: rgba(226, 232, 240, 0.4);
    font-size: 14px;
    cursor: pointer;
    transition: all 0.15s ease;
    -webkit-tap-highlight-color: transparent;
    margin-top: auto;
  }
  .disconnect-btn:active { background: rgba(255, 80, 80, 0.08); }
  .disconnect-btn.confirm {
    background: rgba(255, 80, 80, 0.1);
    border-color: rgba(255, 80, 80, 0.2);
    color: #ff5050;
    font-weight: 600;
  }
</style>
