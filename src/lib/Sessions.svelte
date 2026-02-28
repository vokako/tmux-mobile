<script>
  import { listSessions, listPanes, newSession, killSession } from './ws.js';
  import Icon from './Icon.svelte';

  let { openTerminal, activeTarget = '', visible = false } = $props();

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
  .pane-arrow {
    color: var(--accent);
    font-size: 12px;
  }

  .new-session {
    display: flex;
    gap: 8px;
  }
  .new-session input {
    flex: 1;
    padding: 11px 14px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    font-size: 14px;
    outline: none;
    transition: all 0.2s ease;
    -webkit-appearance: none;
  }
  .new-session input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(0, 212, 255, 0.06);
  }
  .new-session input::placeholder { color: var(--text3); }

  .new-session button {
    width: 44px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--accent-bg);
    color: var(--accent);
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
    background: var(--accent-bg);
    transform: scale(0.95);
  }
  .new-session button:disabled { opacity: 0.3; cursor: default; }
  .refresh-btn {
    width: 44px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text3);
    cursor: pointer;
    display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .refresh-btn:active { background: var(--accent-bg); color: var(--accent); }

  .error {
    color: var(--danger);
    font-size: 13px;
    padding: 10px 14px;
    background: var(--danger-bg);
    border: 1px solid var(--danger);
    border-radius: 10px;
  }

</style>
