<script>
  import Icon from '../ui/Icon.svelte';
  import { t, i18n, setLocale } from '../core/i18n.svelte.js';
  import { layout } from './layout.svelte.js';
  import { fonts } from './fonts.svelte.js';
  import { terminalPrefs, LINE_HEIGHT_MIN, LINE_HEIGHT_MAX } from './terminal-prefs.svelte.js';
  import { SHORTCUT_DEFAULTS, shortcutFromEvent, shortcutLabel } from './shortcuts.ts';
  import { shortcuts } from './shortcuts.svelte.js';
  import { agentHooksInstall, agentHooksRemove, agentHooksStatus } from '../core/ws.ts';

  let {
    connected = false,
    theme = 'system',
    fontSize = 14,
    uiZoom = 1,
    showUiZoom = false,
    showShortcuts = false,
    debugMode = false,
    serverInfo = { hostname: '', machineId: '' },
    activeAddress = '',
    addresses = [],
    optimizing = false,
    linkCopied = false,
    onClose = () => {},
    onTheme = () => {},
    onUiZoom = () => {},
    onFontSize = () => {},
    onDebug = () => {},
    onOptimize = () => {},
    onShare = () => {},
    onAddress = () => {},
    onDisconnect = () => {},
    onConnectionSetup = () => {},
  } = $props();

  const TAB_KEY = 'tmux_settings_tab';
  const storedTab = localStorage.getItem(TAB_KEY);
  const validStoredTab = storedTab === 'connection' || storedTab === 'shortcuts';
  const initialTab = validStoredTab ? storedTab : 'appearance';
  let tab = $state(initialTab);
  if (storedTab && storedTab !== initialTab) localStorage.setItem(TAB_KEY, initialTab);
  const tabs = $derived([
    { id: 'appearance', label: () => t('settingsAppearance'), icon: 'palette' },
    ...(showShortcuts ? [{ id: 'shortcuts', label: () => t('settingsShortcuts'), icon: 'command' }] : []),
    { id: 'connection', label: () => t('settingsConnection'), icon: 'link' },
  ]);
  const shortcutActions = [
    ['previousPage', 'shortcutPreviousPage'],
    ['nextPage', 'shortcutNextPage'],
    ['previousWindow', 'shortcutPreviousWindow'],
    ['nextWindow', 'shortcutNextWindow'],
    ['openTerminal', 'shortcutOpenTerminal'],
    ['openFiles', 'shortcutOpenFiles'],
  ];
  let fontInput = $state(fonts.custom);
  let fontInvalid = $state(false);
  let recordingShortcut = $state('');
  let shortcutError = $state('');
  let hookStatus = $state(null);
  let hookBusy = $state(false);
  let hookError = $state('');
  let hookLoaded = false;

  $effect(() => {
    if (!showShortcuts && tab === 'shortcuts') selectTab('appearance');
    if (connected && tab === 'connection' && !hookLoaded) loadHookStatus();
  });

  async function loadHookStatus() {
    hookLoaded = true;
    hookError = '';
    try { hookStatus = await agentHooksStatus(); }
    catch (error) { hookError = error.message; }
  }

  async function updateHooks(install) {
    hookBusy = true;
    hookError = '';
    try { hookStatus = install ? await agentHooksInstall() : await agentHooksRemove(); }
    catch (error) { hookError = error.message; }
    finally { hookBusy = false; }
  }

  async function saveFont() {
    fontInput = fontInput.trim();
    fontInvalid = !await fonts.set(fontInput);
    return !fontInvalid;
  }

  async function handleFontKeydown(event) {
    if (event.key === 'Enter' && await saveFont()) event.currentTarget.blur();
  }

  function setLineHeight(value) {
    terminalPrefs.setLineHeight(Math.round(value * 100) / 100);
  }

  function selectTab(value) {
    tab = value;
    recordingShortcut = '';
    shortcutError = '';
    localStorage.setItem(TAB_KEY, value);
  }

  function recordShortcut(action, event) {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') { recordingShortcut = ''; shortcutError = ''; return; }
    if (event.key === 'Backspace' || event.key === 'Delete') {
      shortcuts.set(action, '');
      recordingShortcut = '';
      shortcutError = '';
      return;
    }
    const value = shortcutFromEvent(event);
    if (!value) return;
    if (!shortcuts.set(action, value)) {
      shortcutError = t('shortcutConflict');
      return;
    }
    recordingShortcut = '';
    shortcutError = '';
  }
</script>

<section class="preferences" aria-label={t('settings')}>
  <div class="pref-shell">
    <nav class="pref-tabs" aria-label={t('settings')}>
      {#each tabs as item}
        <button class:active={tab === item.id} onclick={() => selectTab(item.id)}>
          <Icon name={item.icon} size={15} /><span>{item.label()}</span>
        </button>
      {/each}
      <button class="debug-toggle" class:active={debugMode} aria-pressed={debugMode} onclick={() => onDebug(!debugMode)} title={t('debugHint')}>
        <Icon name="terminal" size={15} /><span>{t('debug')}</span>
      </button>
      <button class="close" onclick={onClose} aria-label={t('close')}><Icon name="x" size={16} /></button>
    </nav>

    <div class="pref-content">
      {#if tab === 'appearance'}
        <div class="setting-card">
          <div class="setting-row">
            <div><strong>{t('theme')}</strong><small>{t('themeHint')}</small></div>
            <div class="segmented">
              <button class:active={theme === 'system'} onclick={() => onTheme('system')}>{t('themeAuto')}</button>
              <button class:active={theme === 'light'} onclick={() => onTheme('light')}>{t('themeLight')}</button>
              <button class:active={theme === 'dark'} onclick={() => onTheme('dark')}>{t('themeDark')}</button>
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('language')}</strong><small>{t('languageHint')}</small></div>
            <div class="segmented">
              <button class:active={i18n.lang === 'en'} onclick={() => setLocale('en')}>EN</button>
              <button class:active={i18n.lang === 'zh'} onclick={() => setLocale('zh')}>中文</button>
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('layout')}</strong><small>{t('layoutHint')}</small></div>
            <div class="segmented">
              <button class:active={layout.mode === 'auto'} onclick={() => layout.set('auto')}>{t('layoutAuto')}</button>
              <button class:active={layout.mode === 'desktop'} onclick={() => layout.set('desktop')}>{t('layoutDesktop')}</button>
              <button class:active={layout.mode === 'mobile'} onclick={() => layout.set('mobile')}>{t('layoutMobile')}</button>
            </div>
          </div>
          {#if showUiZoom}
            <div class="setting-row">
              <div><strong>{t('uiZoom')}</strong><small>{t('uiZoomHint')}</small></div>
              <div class="stepper">
                <button onclick={() => onUiZoom(uiZoom - 0.1)}>−</button><span>{Math.round(uiZoom * 100)}%</span><button onclick={() => onUiZoom(uiZoom + 0.1)}>+</button>
              </div>
            </div>
          {/if}
          <div class="setting-row">
            <div><strong>{t('fontFamily')}</strong><small>{t('fontFamilyHint')}</small></div>
            <div class="font-control">
              <input class="text-input" class:invalid={fontInvalid} type="text" list="font-families"
                placeholder={t('fontFamilySystem')} bind:value={fontInput} aria-invalid={fontInvalid}
              autocapitalize="off" autocomplete="off" spellcheck="false"
                oninput={() => fontInvalid = false} onchange={saveFont}
                onkeydown={handleFontKeydown} />
              <datalist id="font-families">
                {#each fonts.common as family}<option value={family}></option>{/each}
              </datalist>
              {#if fontInvalid}<small class="font-error">{t('fontFamilyInvalid')}</small>{/if}
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('font')}</strong><small>{t('fontSizeHint')}</small></div>
            <div class="stepper">
              <button onclick={() => onFontSize(fontSize - 1)}>−</button><span>{fontSize}px</span><button onclick={() => onFontSize(fontSize + 1)}>+</button>
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('lineHeight')}</strong><small>{t('lineHeightHint')}</small></div>
            <div class="range-wrap">
              <input type="range" min={LINE_HEIGHT_MIN} max={LINE_HEIGHT_MAX} step="0.05" value={terminalPrefs.lineHeight} oninput={(e) => setLineHeight(+e.currentTarget.value)} />
              <span>{terminalPrefs.lineHeight.toFixed(2)}</span>
              <button class="reset" onclick={() => setLineHeight(1)}>↺</button>
            </div>
          </div>
        </div>
      {:else if tab === 'shortcuts'}
        <div class="setting-card shortcut-card">
          {#each shortcutActions as [action, label]}
            <div class="setting-row">
              <div><strong>{t(label)}</strong><small>{t(action.includes('Window') ? 'shortcutTerminalScope' : 'shortcutGlobalScope')}</small></div>
              <button
                class="shortcut-key"
                class:recording={recordingShortcut === action}
                data-shortcut-recorder
                onclick={() => { recordingShortcut = action; shortcutError = ''; }}
                onkeydown={(event) => recordShortcut(action, event)}
              >{recordingShortcut === action ? t('shortcutPressKeys') : shortcutLabel(shortcuts.get(action))}</button>
            </div>
          {/each}
        </div>
        {#if shortcutError}<div class="shortcut-error">{shortcutError}</div>{/if}
        <button class="shortcut-reset" onclick={() => { shortcuts.reset(); shortcutError = ''; }}>{t('shortcutReset')}</button>
      {:else}
        <div class="setting-card">
          {#if connected}
            <div class="connection-title">
              <div><strong>{serverInfo.hostname || 'unknown'}</strong><small>{serverInfo.machineId?.slice(0, 8) || '—'}</small></div>
              <div class="conn-actions">
                {#if addresses.length > 1}<button onclick={onOptimize} disabled={optimizing}>{optimizing ? t('sniffing') : t('sniff')}</button>{/if}
                <button onclick={onShare}><Icon name={linkCopied ? 'check' : 'copy'} size={13} /> {t('shareLink')}</button>
              </div>
            </div>
            <div class="address-list">
              {#each (addresses.length ? addresses : [activeAddress]) as address}
                <button class:active={address === activeAddress} onclick={() => address !== activeAddress && onAddress(address)}>{address}</button>
              {/each}
            </div>
            <div class="setting-row hook-row">
              <div>
                <strong>{t('agentNotifications')}</strong>
                <small>{t('agentNotificationsHint')}</small>
              </div>
              <div class="hook-control">
                {#if hookStatus}
                  <span class="hook-backends">
                    <span class:on={hookStatus.claude?.installed}>Claude</span>
                    <span class:on={hookStatus.codex?.installed}>Codex</span>
                    <span class:on={hookStatus.kiro?.installed}>Kiro</span>
                  </span>
                  {#if hookStatus.claude?.installed && hookStatus.codex?.installed && hookStatus.kiro?.installed}
                    <button class="hook-action" disabled={hookBusy} onclick={() => updateHooks(false)}>{t('agentHooksRemove')}</button>
                  {:else}
                    <button class="hook-action primary" disabled={hookBusy} onclick={() => updateHooks(true)}>{hookBusy ? '…' : t('agentHooksInstall')}</button>
                  {/if}
                {:else}
                  <button class="hook-action" disabled={hookBusy} onclick={loadHookStatus}>{hookBusy ? '…' : t('agentHooksCheck')}</button>
                {/if}
              </div>
            </div>
            {#if hookError}<div class="hook-error">{hookError}</div>{/if}
          {:else}
            <div class="empty-connection"><Icon name="link" size={20} /><span>{t('notConnected')}</span><button onclick={onConnectionSetup}>{t('connectionSetup')}</button></div>
          {/if}
        </div>
        {#if connected}<button class="disconnect" onclick={onDisconnect}>{t('disconnect')}</button>{/if}
      {/if}
    </div>
  </div>
</section>

<style>
  .preferences { position: fixed; inset: calc(49px + var(--sat)) 0 0; z-index: 19; display:flex; flex-direction:column; background:var(--bg); color:var(--text); }
  .pref-shell { flex:1;min-height:0;display:flex;flex-direction:column; }
  .pref-tabs { display:flex;align-items:center;gap:var(--ui-gap);min-height:var(--ui-bar-height);padding:var(--ui-bar-padding);box-sizing:border-box;border-bottom:1px solid var(--border2);background:var(--surface);flex-shrink:0;overflow-x:auto;scrollbar-width:none;-webkit-overflow-scrolling:touch; }
  .pref-tabs::-webkit-scrollbar { display:none; }
  .pref-tabs button { flex-shrink:0;height:var(--ui-control-height);display:inline-flex;align-items:center;justify-content:center;gap:var(--ui-gap);padding:3px 8px;border:1px solid var(--border2);border-radius:var(--ui-radius-pill);background:transparent;color:var(--text3);font-size:var(--ui-font-control);font-weight:500;white-space:nowrap;cursor:pointer;-webkit-tap-highlight-color:transparent; }
  .pref-tabs button.active { border-color:var(--accent);background:var(--accent-bg);color:var(--accent);font-weight:600; }
  .pref-tabs .debug-toggle.active { background:var(--accent);color:var(--bg); }
  .pref-tabs button:active { border-color:var(--accent);color:var(--accent); }
  .pref-tabs .close { width:var(--ui-control-height);padding:0;margin-left:auto;border-color:transparent;background:transparent; }
  .pref-tabs .close:active { background:var(--surface2); }
  .pref-content { flex:1;min-width:0;overflow:auto;padding:14px clamp(12px,3vw,24px); }
  .setting-card { max-width:720px;margin:0 auto;border:1px solid var(--border2);border-radius:var(--ui-radius-panel);background:var(--surface);overflow:hidden; }
  .setting-row { min-height:52px;padding:8px 12px;display:flex;align-items:center;justify-content:space-between;gap:16px; }
  .setting-row+.setting-row { border-top:1px solid var(--border2); } .setting-row>div:first-child{display:flex;flex-direction:column;gap:4px;min-width:0;}
  strong{font-size:12px;} small{font-size:10px;color:var(--text3);font-weight:400;line-height:1.35;}
  .segmented { display:flex;gap:4px;flex-shrink:0; }
  .segmented button,.stepper button,.reset,.conn-actions button { height:var(--ui-control-height);border:1px solid var(--border2);background:transparent;color:var(--text3);padding:3px 8px;border-radius:var(--ui-radius-pill);cursor:pointer;font-size:var(--ui-font-control);white-space:nowrap; }
  .segmented button.active { border-color:var(--accent);background:var(--accent-bg);color:var(--accent); }
  .segmented button:active,.stepper button:active,.reset:active,.conn-actions button:active { border-color:var(--accent);color:var(--accent); }
  .text-input { width:min(230px,42vw);height:28px;padding:4px 8px;border:1px solid var(--input-border);border-radius:7px;background:var(--input-bg);color:var(--text);font:11px var(--font-mono);outline:none; }
  .text-input:focus{border-color:var(--accent);}
  .font-control{display:flex;flex-direction:column;align-items:flex-end;gap:3px}.text-input.invalid{border-color:var(--danger)}.font-error{color:var(--danger)}
  .stepper { display:flex;align-items:center;gap:4px; }.stepper button{width:24px;padding:0;font-size:14px}.stepper span{min-width:42px;text-align:center;font-family:var(--font-mono);font-size:11px;}
  .range-wrap { display:flex;align-items:center;gap:7px;min-width:min(280px,46vw); }
  .range-wrap input { flex:1;height:14px;margin:0;appearance:none;-webkit-appearance:none;background:transparent;cursor:pointer; }
  .range-wrap input::-webkit-slider-runnable-track { height:3px;border-radius:999px;background:var(--surface2);border:1px solid var(--border2); }
  .range-wrap input::-webkit-slider-thumb { appearance:none;-webkit-appearance:none;width:12px;height:12px;margin-top:-5px;border:2px solid var(--bg);border-radius:50%;background:var(--accent);box-shadow:0 0 0 1px var(--accent); }
  .range-wrap input::-moz-range-track { height:3px;border-radius:999px;background:var(--surface2);border:1px solid var(--border2); }
  .range-wrap input::-moz-range-thumb { width:10px;height:10px;border:2px solid var(--bg);border-radius:50%;background:var(--accent);box-shadow:0 0 0 1px var(--accent); }
  .range-wrap span{width:31px;font:10px var(--font-mono);color:var(--text2)}.reset{width:24px;padding:0}
  .shortcut-key{min-width:74px;height:var(--ui-control-height);padding:3px 10px;border:1px solid var(--border2);border-radius:var(--ui-radius-pill);background:var(--input-bg);color:var(--text);font:600 11px var(--font-mono);cursor:pointer}.shortcut-key.recording{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}
  .shortcut-error{max-width:720px;margin:7px auto 0;color:var(--danger);font-size:10px;text-align:center}.shortcut-reset{display:block;margin:8px auto;padding:5px 10px;border:1px solid var(--border2);border-radius:var(--ui-radius-pill);background:transparent;color:var(--text3);font-size:11px;cursor:pointer}
  .connection-title{padding:10px 12px;display:flex;align-items:center;justify-content:space-between;gap:10px}.connection-title>div:first-child{display:flex;flex-direction:column;gap:3px}.conn-actions{display:flex;gap:4px}.conn-actions button{display:flex;align-items:center;gap:4px}
  .address-list{padding:0 12px 10px;display:flex;flex-direction:column;gap:3px}.address-list button{padding:7px 9px;border:1px solid var(--border2);border-radius:7px;background:var(--input-bg);color:var(--text3);font:11px var(--font-mono);text-align:left;word-break:break-all;cursor:pointer}.address-list button.active{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}
  .hook-row{border-top:1px solid var(--border2)}.hook-control{display:flex;align-items:center;gap:7px;flex-shrink:0}.hook-backends{display:flex;gap:3px}.hook-backends span{padding:2px 5px;border-radius:var(--ui-radius-pill);background:var(--surface2);color:var(--text3);font-size:9px}.hook-backends span.on{background:var(--accent-bg);color:var(--accent)}.hook-action{height:var(--ui-control-height);padding:3px 8px;border:1px solid var(--border2);border-radius:var(--ui-radius-pill);background:transparent;color:var(--text3);font-size:var(--ui-font-control);cursor:pointer}.hook-action.primary{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}.hook-action:disabled{opacity:.5}.hook-error{padding:0 12px 8px;color:var(--danger);font-size:10px}
  .disconnect{display:block;width:min(720px,100%);margin:8px auto;padding:7px;border:1px solid color-mix(in srgb,var(--danger) 55%,transparent);border-radius:7px;background:none;color:var(--danger);cursor:pointer;font-size:11px;font-weight:600}
  .empty-connection{padding:28px 12px;display:flex;flex-direction:column;align-items:center;gap:9px;color:var(--text3);font-size:11px}.empty-connection button{padding:6px 10px;border:1px solid var(--accent);border-radius:6px;background:var(--accent-bg);color:var(--accent);cursor:pointer;font-size:11px}
  @media(max-width:640px){.preferences{inset:calc(49px + var(--sat)) 0 0}.pref-content{padding:9px 8px}.setting-row{min-height:50px;padding:8px 10px;gap:10px}.text-input{width:min(220px,48vw)}.range-wrap{min-width:min(250px,52vw)}.connection-title{align-items:flex-start;flex-direction:column}.conn-actions{width:100%}.conn-actions button{flex:1;justify-content:center}.hook-row{align-items:flex-start;flex-direction:column}.hook-control{width:100%;justify-content:space-between}}
  @media(max-width:420px){.setting-row{align-items:flex-start;flex-direction:column;gap:7px}.setting-row>div:last-child,.text-input,.range-wrap{width:100%;min-width:0}.font-control{align-items:flex-start}.segmented button{flex:1}}
</style>
