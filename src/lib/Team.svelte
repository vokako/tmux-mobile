<script>
  // Team tab — the agora multi-agent group chat.
  //
  // Talks to the in-process agora bus on the desktop server (RPC methods
  // agora_history / agora_roster / agora_post + the agora_message push). The
  // human operator ("you") is just another participant: type to broadcast, or
  // tap an agent to @mention it (which requires a reply, per the bus's
  // obligation rule). Tapping an agent's roster chip jumps to the tmux pane
  // that agent runs in, so you can preview its live execution state in the
  // Terminal tab.
  //
  // Availability: a server without the bus (mobile, or desktop with agora
  // disabled) makes the agora_* RPCs reject with method-not-found; we surface
  // that as an "unavailable" state and the App hides the tab.
  import AgentChip from './AgentChip.svelte';
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';
  import {
    agoraHistory, agoraRoster, agoraPost, agoraStatus, agoraStartTeam,
    addAgoraMessageListener, removeAgoraMessageListener,
    listSessionsWithPanes,
  } from './ws.js';

  let {
    visible = false,
    teamSession = 'agora',  // tmux session the launcher runs agents in
    openTerminal = () => {}, // (session, target, command) — preview an agent's pane
  } = $props();

  let messages = $state([]);     // agora Message[] (oldest first)
  let roster = $state([]);       // AgentRow[]
  let available = $state(true);  // false when the server has no agora bus
  let teamStarted = $state(false); // whether the supervisor has launched a team
  let starting = $state(false);
  let loading = $state(true);
  let draft = $state('');
  let sending = $state(false);
  let listEl = $state(null);

  // Roster entries that are present (not offline). The human posts as "human";
  // never show it as an addressable agent (you can't @ yourself usefully).
  let agents = $derived(roster.filter(a => a.status !== 'offline' && a.name !== 'human'));

  function scrollToBottom() {
    requestAnimationFrame(() => { if (listEl) listEl.scrollTop = listEl.scrollHeight; });
  }

  async function refresh() {
    try {
      const [h, r, s] = await Promise.all([agoraHistory(200), agoraRoster(), agoraStatus()]);
      messages = h?.messages || [];
      roster = r?.roster || [];
      teamStarted = !!s?.team_started;
      available = true;
      scrollToBottom();
    } catch (e) {
      // method-not-found → no bus on this server. Any other error is transient
      // (treat as unavailable too; a reconnect re-runs this).
      available = false;
    } finally {
      loading = false;
    }
  }

  async function startTeam() {
    if (starting) return;
    starting = true;
    try {
      await agoraStartTeam();
      teamStarted = true;
      // Agents take a few seconds to come online; refresh the roster shortly.
      setTimeout(refresh, 3000);
    } catch {
    } finally {
      starting = false;
    }
  }

  // Live push: append each broadcast message. De-dupe by id (history + a racing
  // push can overlap right after mount).
  function onAgoraMessage(m) {
    if (!m?.id) return;
    if (messages.some(x => x.id === m.id)) return;
    messages = [...messages, m];
    scrollToBottom();
  }

  $effect(() => {
    addAgoraMessageListener(onAgoraMessage);
    return () => removeAgoraMessageListener(onAgoraMessage);
  });

  // Refresh whenever the tab becomes visible (cheap; also catches reconnects).
  $effect(() => {
    if (visible) refresh();
  });

  async function send() {
    const body = draft.trim();
    if (!body || sending) return;
    sending = true;
    try {
      await agoraPost(body);
      draft = '';
      // The post echoes back via the agora_message push, so we don't append
      // locally (avoids a duplicate). Just refresh the roster status.
    } catch {
      // Leave the draft in place so the user can retry.
    } finally {
      sending = false;
    }
  }

  function onKeydown(e) {
    // Enter sends; Shift+Enter inserts a newline (desktop). On a soft keyboard
    // Enter usually inserts a newline, so the send button is the primary path.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  }

  function mention(name) {
    // Insert "@name " into the draft so the next post addresses that agent
    // (the bus turns an @mention into a reply obligation).
    const sep = draft && !draft.endsWith(' ') ? ' ' : '';
    draft = `${draft}${sep}@${name} `;
  }

  // Jump to the tmux pane an agent runs in. The launcher names each agent's
  // window after the agent, so we find the pane whose window_name matches.
  async function previewAgent(name) {
    try {
      const { panes } = await listSessionsWithPanes();
      const p = (panes || []).find(p =>
        p.session === teamSession && (p.window_name === name)
      ) || (panes || []).find(p => p.window_name === name);
      if (!p) return;
      const target = `${p.session}:${p.window}.${p.pane}`;
      openTerminal(p.session, target, p.current_command || '');
    } catch {}
  }

  function fmtTime(ts) {
    try {
      return new Date(ts).toLocaleTimeString('en', { hour12: false, hour: '2-digit', minute: '2-digit' });
    } catch { return ''; }
  }

  function isSystem(m) { return m.kind === 'join' || m.kind === 'leave' || m.kind === 'system'; }
  function isMine(m) { return m.kind === 'msg' && m.from === 'human'; }
</script>

<div class="team">
  {#if loading}
    <div class="team-empty">…</div>
  {:else if !available}
    <div class="team-empty team-unavail">
      <Icon name="bot" size={28} />
      <p>{t('teamUnavailable')}</p>
    </div>
  {:else}
    <!-- Roster: present agents as chips; tap to preview their tmux pane. -->
    <div class="team-roster">
      {#if agents.length === 0}
        {#if teamStarted}
          <span class="team-roster-empty">{t('teamStarting')}</span>
        {:else}
          <button class="team-start" disabled={starting} onclick={startTeam}>
            {#if starting}<span class="reconnect-spinner-sm"></span>{:else}<Icon name="bot" size={14} />{/if}
            {t('teamStart')}
          </button>
        {/if}
      {:else}
        {#each agents as a}
          <button class="roster-chip" class:waiting={a.status === 'waiting'} onclick={() => previewAgent(a.name)} title={a.role || a.name}>
            <span class="roster-dot status-{a.status}"></span>
            <span class="roster-name">{a.name}</span>
            <Icon name="terminal" size={11} />
          </button>
        {/each}
      {/if}
    </div>

    <!-- Message log -->
    <div class="team-log" bind:this={listEl}>
      {#if messages.length === 0}
        <div class="team-empty">{t('teamNoMessages')}</div>
      {:else}
        {#each messages as m (m.id)}
          {#if isSystem(m)}
            <div class="msg-system">{m.body}</div>
          {:else}
            <div class="msg-row" class:mine={isMine(m)}>
              <div class="msg-bubble" class:mine={isMine(m)}>
                {#if !isMine(m)}<div class="msg-from">{m.from}</div>{/if}
                <div class="msg-body">{m.body}</div>
                <div class="msg-time">{fmtTime(m.ts)}</div>
              </div>
            </div>
          {/if}
        {/each}
      {/if}
    </div>

    <!-- Compose -->
    <div class="team-compose">
      {#if agents.length}
        <div class="compose-mentions">
          {#each agents as a}
            <button class="mention-chip" onclick={() => mention(a.name)}>@{a.name}</button>
          {/each}
        </div>
      {/if}
      <div class="compose-row">
        <textarea
          class="compose-input"
          bind:value={draft}
          onkeydown={onKeydown}
          placeholder={t('teamMessage')}
          rows="1"
        ></textarea>
        <button class="compose-send" disabled={!draft.trim() || sending} onclick={send} aria-label={t('teamSend')}>
          <Icon name="send" size={16} />
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .team {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }
  .team-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text3);
    font-size: 13px;
    padding: 24px;
    text-align: center;
  }
  .team-unavail p { margin: 0; max-width: 260px; line-height: 1.5; }

  /* Roster strip */
  .team-roster {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    scrollbar-width: none;
  }
  .team-roster::-webkit-scrollbar { display: none; }
  .team-roster-empty { color: var(--text3); font-size: 12px; padding: 4px 2px; }
  .team-start {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 5px 12px; height: 28px;
    border: 1px solid var(--accent); border-radius: 999px;
    background: var(--accent-bg); color: var(--accent);
    font-size: 12px; font-weight: 600; cursor: pointer; flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .team-start:active { background: var(--accent); color: var(--bg); }
  .team-start:disabled { opacity: 0.5; cursor: default; }
  .reconnect-spinner-sm {
    width: 12px; height: 12px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%;
    animation: team-spin 0.6s linear infinite;
  }
  @keyframes team-spin { to { transform: rotate(360deg); } }
  .roster-chip {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 4px 10px; height: 28px;
    border: 1px solid var(--border2); border-radius: 999px;
    background: var(--input-bg); color: var(--text2);
    font-size: 12px; font-weight: 500; cursor: pointer; flex-shrink: 0;
    white-space: nowrap; -webkit-tap-highlight-color: transparent;
    transition: border-color 0.15s ease, color 0.15s ease;
  }
  .roster-chip:active { border-color: var(--accent); color: var(--accent); }
  .roster-name { max-width: 120px; overflow: hidden; text-overflow: ellipsis; }
  .roster-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: var(--text3); }
  .roster-dot.status-working { background: var(--status-warn); }
  .roster-dot.status-waiting { background: var(--status-ok); }
  .roster-dot.status-online { background: var(--accent); }

  /* Message log */
  .team-log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    -webkit-overflow-scrolling: touch;
  }
  .msg-system {
    align-self: center;
    color: var(--text3);
    font-size: 11px;
    padding: 2px 10px;
    background: var(--surface);
    border-radius: 999px;
    max-width: 90%;
    text-align: center;
  }
  .msg-row { display: flex; }
  .msg-row.mine { justify-content: flex-end; }
  .msg-bubble {
    max-width: 78%;
    padding: 7px 11px;
    border-radius: 14px;
    background: var(--surface2);
    border: 1px solid var(--border2);
    border-bottom-left-radius: 4px;
  }
  .msg-bubble.mine {
    background: var(--accent-bg);
    border-color: var(--accent);
    border-bottom-left-radius: 14px;
    border-bottom-right-radius: 4px;
  }
  .msg-from {
    font-size: 11px; font-weight: 600; color: var(--accent);
    margin-bottom: 2px;
  }
  .msg-body {
    font-size: 13px; line-height: 1.45; color: var(--text);
    white-space: pre-wrap; word-break: break-word;
    user-select: text; -webkit-user-select: text;
  }
  .msg-time { font-size: 9px; color: var(--text3); margin-top: 3px; text-align: right; }

  /* Compose */
  .team-compose {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    padding: 8px 10px calc(8px + var(--sab));
    background: var(--nav-bg);
  }
  .compose-mentions {
    display: flex; gap: 5px; overflow-x: auto; padding-bottom: 7px;
    scrollbar-width: none;
  }
  .compose-mentions::-webkit-scrollbar { display: none; }
  .mention-chip {
    flex-shrink: 0;
    padding: 3px 9px; border: 1px solid var(--border2); border-radius: 999px;
    background: transparent; color: var(--text3); font-size: 11px; font-weight: 500;
    cursor: pointer; -webkit-tap-highlight-color: transparent; white-space: nowrap;
  }
  .mention-chip:active { color: var(--accent); border-color: var(--accent); }
  .compose-row { display: flex; align-items: flex-end; gap: 8px; }
  .compose-input {
    flex: 1; min-height: 38px; max-height: 120px;
    padding: 9px 12px; border: 1px solid var(--input-border); border-radius: 18px;
    background: var(--input-bg); color: var(--text); font-size: 14px;
    font-family: inherit; resize: none; line-height: 1.4;
    outline: none;
  }
  .compose-input:focus { border-color: var(--accent); }
  .compose-send {
    width: 38px; height: 38px; flex-shrink: 0;
    border: none; border-radius: 50%;
    background: var(--accent-bg); color: var(--accent);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: opacity 0.15s ease;
  }
  .compose-send:disabled { opacity: 0.4; cursor: default; }
  .compose-send:not(:disabled):active { background: var(--accent); color: var(--bg); }
</style>
