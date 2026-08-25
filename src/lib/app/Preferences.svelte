<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import { t, i18n, setLocale } from '../core/i18n.svelte.ts';
  import { layout } from './layout.svelte.ts';
  import { fonts, uiFont, displayFont } from './fonts.svelte.ts';
  import { terminalPrefs, LINE_HEIGHT_MIN, LINE_HEIGHT_MAX } from './terminal-prefs.svelte.ts';
  import { hubPrefs } from '../hub/hub-prefs.svelte.ts';
  import { SHORTCUT_DEFAULTS, shortcutFromEvent, shortcutLabel, type ShortcutAction } from './shortcuts.ts';
  import { shortcuts } from './shortcuts.svelte.ts';
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
  }: {
    connected?: boolean;
    theme?: string;
    fontSize?: number;
    uiZoom?: number;
    showUiZoom?: boolean;
    showShortcuts?: boolean;
    debugMode?: boolean;
    serverInfo?: { hostname: string; machineId: string };
    activeAddress?: string;
    addresses?: string[];
    optimizing?: boolean;
    linkCopied?: boolean;
    onClose?: () => void;
    onTheme?: (theme: string) => void;
    onUiZoom?: (value: number) => void;
    onFontSize?: (size: number) => void;
    onDebug?: (on: boolean) => void;
    onOptimize?: () => void;
    onShare?: () => void;
    onAddress?: (address: string) => void;
    onDisconnect?: () => void;
    onConnectionSetup?: () => void;
  } = $props();

  const TAB_KEY = 'tmux_settings_tab';
  const storedTab = localStorage.getItem(TAB_KEY);
  const validStoredTab = storedTab === 'connection' || storedTab === 'shortcuts';
  const initialTab = validStoredTab ? storedTab : 'appearance';
  let tab = $state<string>(initialTab);
  if (storedTab && storedTab !== initialTab) localStorage.setItem(TAB_KEY, initialTab);
  const tabs = $derived([
    { id: 'appearance', label: () => t('settingsAppearance'), icon: 'palette' },
    { id: 'terminal', label: () => t('settingsTerminal'), icon: 'terminal' },
    ...(showShortcuts ? [{ id: 'shortcuts', label: () => t('settingsShortcuts'), icon: 'command' }] : []),
    { id: 'connection', label: () => t('settingsConnection'), icon: 'link' },
  ]);
  const shortcutActions: [ShortcutAction, string][] = [
    ['previousPage', 'shortcutPreviousPage'],
    ['nextPage', 'shortcutNextPage'],
    ['previousWindow', 'shortcutPreviousWindow'],
    ['nextWindow', 'shortcutNextWindow'],
    ['openTerminal', 'shortcutOpenTerminal'],
    ['openFiles', 'shortcutOpenFiles'],
  ];
  let fontInput = $state(fonts.custom);
  let fontInvalid = $state(false);
  // The other two roles (owner, 2026-08-25: "总之就三类…这些可以都是系统设
  // 置里的字体"): content prose and the chrome (titles/buttons/names). Same
  // validate-then-commit contract as the terminal font.
  let uiFontInput = $state(uiFont.custom);
  let uiFontInvalid = $state(false);
  let displayFontInput = $state(displayFont.custom);
  let displayFontInvalid = $state(false);
  let recordingShortcut = $state<ShortcutAction | ''>('');
  let shortcutError = $state('');
  type HookAgentStatus = { installed?: boolean };
  type HookStatus = { claude?: HookAgentStatus; codex?: HookAgentStatus; kiro?: HookAgentStatus };
  let hookStatus = $state<HookStatus | null>(null);
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
    catch (error) { hookError = (error as Error).message; }
  }

  async function updateHooks(install: boolean) {
    hookBusy = true;
    hookError = '';
    try { hookStatus = install ? await agentHooksInstall() : await agentHooksRemove(); }
    catch (error) { hookError = (error as Error).message; }
    finally { hookBusy = false; }
  }

  async function saveFont() {
    fontInput = fontInput.trim();
    fontInvalid = !await fonts.set(fontInput);
    return !fontInvalid;
  }

  async function handleFontKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && await saveFont()) (event.currentTarget as HTMLElement | null)?.blur();
  }

  async function saveUiFont() {
    uiFontInput = uiFontInput.trim();
    uiFontInvalid = !await uiFont.set(uiFontInput);
    return !uiFontInvalid;
  }
  async function saveDisplayFont() {
    displayFontInput = displayFontInput.trim();
    displayFontInvalid = !await displayFont.set(displayFontInput);
    return !displayFontInvalid;
  }
  async function handleUiFontKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && await saveUiFont()) (event.currentTarget as HTMLElement | null)?.blur();
  }
  async function handleDisplayFontKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && await saveDisplayFont()) (event.currentTarget as HTMLElement | null)?.blur();
  }

  function setLineHeight(value: number) {
    terminalPrefs.setLineHeight(Math.round(value * 100) / 100);
  }

  function selectTab(value: string) {
    tab = value;
    recordingShortcut = '';
    shortcutError = '';
    localStorage.setItem(TAB_KEY, value);
  }

  function recordShortcut(action: ShortcutAction, event: KeyboardEvent) {
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

<!-- Settings is a PAGE in the unified skeleton (ui-unification.md "Settings
     as a page"): shared sidebar with category rows, main column with a
     page-head. No backdrop, no X — the rail/tab bar is the way out. -->
<section class="preferences" aria-label={t('settings')}>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll">
      <div class="side-h">{t('settings')}</div>
      {#each tabs as item}
        <button class="side-row" class:open={tab === item.id} onclick={() => selectTab(item.id)}>
          <Icon name={item.icon} size={14} /><span class="r-label">{item.label()}</span>
        </button>
      {/each}
    </div>
  </aside>
  <div class="pref-shell">
    <div class="page-head">
      <h1>{tabs.find((x) => x.id === tab)?.label() ?? t('settings')}</h1>
    </div>
    <!-- Compact: the sidebar is hidden, categories become a chip row. -->
    <div class="cat-chips">
      {#each tabs as item}
        <button class="pchip" class:sel={tab === item.id} onclick={() => selectTab(item.id)}>
          <Icon name={item.icon} size={13} />{item.label()}
        </button>
      {/each}
    </div>

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
          <div class="setting-row">
            <div><strong>{t('hubFeedLevel')}</strong><small>{t('hubFeedLevelHint')}</small></div>
            <div class="segmented">
              <button class:active={hubPrefs.feedLevel === 'chat'} onclick={() => hubPrefs.setFeedLevel('chat')}>{t('hubFeedChat')}</button>
              <button class:active={hubPrefs.feedLevel === 'status'} onclick={() => hubPrefs.setFeedLevel('status')}>{t('hubFeedStatus')}</button>
              <button class:active={hubPrefs.feedLevel === 'tools'} onclick={() => hubPrefs.setFeedLevel('tools')}>{t('hubFeedTools')}</button>
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('hubStepsRows')}</strong><small>{t('hubStepsRowsHint')}</small></div>
            <div class="stepper">
              <button onclick={() => hubPrefs.setStepsRows(hubPrefs.stepsRows - 1)}>−</button><span>{hubPrefs.stepsRows}</span><button onclick={() => hubPrefs.setStepsRows(hubPrefs.stepsRows + 1)}>+</button>
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('uiFontBody')}</strong><small>{t('uiFontBodyHint')}</small></div>
            <div class="font-control">
              <input class="text-input" class:invalid={uiFontInvalid} type="text" list="sans-families"
                placeholder={t('fontFamilySystem')} bind:value={uiFontInput} aria-invalid={uiFontInvalid}
                autocapitalize="off" autocomplete="off" spellcheck="false"
                oninput={() => uiFontInvalid = false} onchange={saveUiFont}
                onkeydown={handleUiFontKeydown} />
              {#if uiFontInvalid}<small class="font-error">{t('fontFamilyInvalid')}</small>{/if}
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('uiFontDisplay')}</strong><small>{t('uiFontDisplayHint')}</small></div>
            <div class="font-control">
              <input class="text-input" class:invalid={displayFontInvalid} type="text" list="sans-families"
                placeholder={t('fontFamilySystem')} bind:value={displayFontInput} aria-invalid={displayFontInvalid}
                autocapitalize="off" autocomplete="off" spellcheck="false"
                oninput={() => displayFontInvalid = false} onchange={saveDisplayFont}
                onkeydown={handleDisplayFontKeydown} />
              <datalist id="sans-families">
                {#each uiFont.common as family}<option value={family}></option>{/each}
              </datalist>
              {#if displayFontInvalid}<small class="font-error">{t('fontFamilyInvalid')}</small>{/if}
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
        </div>
      {:else if tab === 'terminal'}
        <div class="setting-card">
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
          <div class="setting-row">
            <div><strong>{t('debug')}</strong><small>{t('debugHint')}</small></div>
            <div class="segmented">
              <button class:active={!debugMode} onclick={() => onDebug(false)}>Off</button>
              <button class:active={debugMode} onclick={() => onDebug(true)}>On</button>
            </div>
          </div>
        </div>
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
  /* Page skeleton (ui-unification.md): shared sidebar + main column. */
  .preferences {
    height: 100%; min-height: 0;
    display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    background: var(--bg); color: var(--text);
  }
  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .r-label { flex: 1; min-width: 0; }
  .cat-chips { display: none; }
  .pchip {
    display: flex; align-items: center; gap: 6px; flex: none;
    background: var(--surface); border: 1px solid var(--border); border-radius: 999px;
    color: var(--text2); padding: 5px 12px; font-size: var(--fs-ui); cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .pchip.sel { border-color: var(--accent-line); color: var(--accent); background: var(--accent-bg); }
  @media (max-width: 760px) {
    .preferences { grid-template-columns: minmax(0, 1fr); }
    .sidebar { display: none; }
    .cat-chips {
      display: flex; gap: 6px; padding: 10px 12px 0; overflow-x: auto; flex: none;
      -webkit-overflow-scrolling: touch; scrollbar-width: none;
    }
    .cat-chips::-webkit-scrollbar { display: none; }
  }
  .pref-content { flex:1;min-width:0;overflow:auto;padding:14px clamp(12px,3vw,24px); }
  .setting-card { max-width:720px;margin:0 auto;border:1px solid var(--border2);border-radius:var(--ui-radius-panel);background:var(--surface);overflow:hidden; }
  .setting-row { min-height:52px;padding:8px 12px;display:flex;align-items:center;justify-content:space-between;gap:16px; }
  .setting-row+.setting-row { border-top:1px solid var(--border2); } .setting-row>div:first-child{display:flex;flex-direction:column;gap:4px;min-width:0;}
  strong{font-size:var(--fs-ui);} small{font-size:var(--fs-meta);color:var(--text3);font-weight:400;line-height:1.35;}
  .segmented { display:flex;gap:4px;flex-shrink:0; }
  /* App control dialect: --ui-radius-control squares like every chip-btn/
     icon-btn — the 999px pills were this page's private language and are why
     it read as a different app (owner, 2026-08-25: "和其他页面画风不一样").
     Real pills stay for micro TAGS (.hook-backends) per the radius contract. */
  .segmented button,.stepper button,.reset,.conn-actions button { height:var(--ui-control-height);border:1px solid var(--border2);background:transparent;color:var(--text3);padding:3px 8px;border-radius:var(--ui-radius-control);cursor:pointer;font-size:var(--ui-font-control);white-space:nowrap; }
  .segmented button.active { border-color:var(--accent);background:var(--accent-bg);color:var(--accent); }
  .segmented button:active,.stepper button:active,.reset:active,.conn-actions button:active { border-color:var(--accent);color:var(--accent); }
  .text-input { width:min(230px,42vw);height:28px;padding:4px 8px;border:1px solid var(--input-border);border-radius:var(--ui-radius-control);background:var(--input-bg);color:var(--text);font:var(--fs-sub) var(--font-mono);outline:none; }
  .text-input:focus{border-color:var(--accent);}
  .font-control{display:flex;flex-direction:column;align-items:flex-end;gap:3px}.text-input.invalid{border-color:var(--danger)}.font-error{color:var(--danger)}
  .stepper { display:flex;align-items:center;gap:4px; }.stepper button{width:24px;padding:0;font-size:var(--fs-body)}.stepper span{min-width:42px;text-align:center;font-family:var(--font-mono);font-size:var(--fs-sub);}
  .range-wrap { display:flex;align-items:center;gap:7px;min-width:min(280px,46vw); }
  .range-wrap input { flex:1;height:14px;margin:0;appearance:none;-webkit-appearance:none;background:transparent;cursor:pointer; }
  .range-wrap input::-webkit-slider-runnable-track { height:3px;border-radius:999px;background:var(--surface2);border:1px solid var(--border2); }
  .range-wrap input::-webkit-slider-thumb { appearance:none;-webkit-appearance:none;width:12px;height:12px;margin-top:-5px;border:2px solid var(--bg);border-radius:50%;background:var(--accent);box-shadow:0 0 0 1px var(--accent); }
  .range-wrap input::-moz-range-track { height:3px;border-radius:999px;background:var(--surface2);border:1px solid var(--border2); }
  .range-wrap input::-moz-range-thumb { width:10px;height:10px;border:2px solid var(--bg);border-radius:50%;background:var(--accent);box-shadow:0 0 0 1px var(--accent); }
  .range-wrap span{width:31px;font:10px var(--font-mono);color:var(--text2)}.reset{width:24px;padding:0}
  .shortcut-key{min-width:74px;height:var(--ui-control-height);padding:3px 10px;border:1px solid var(--border2);border-radius:var(--ui-radius-control);background:var(--input-bg);color:var(--text);font:600 var(--fs-sub) var(--font-mono);cursor:pointer}.shortcut-key.recording{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}
  .shortcut-error{max-width:720px;margin:7px auto 0;color:var(--danger);font-size:var(--fs-meta);text-align:center}.shortcut-reset{display:block;margin:8px auto;padding:5px 10px;border:1px solid var(--border2);border-radius:var(--ui-radius-control);background:transparent;color:var(--text3);font-size:var(--fs-sub);cursor:pointer}
  .connection-title{padding:10px 12px;display:flex;align-items:center;justify-content:space-between;gap:10px}.connection-title>div:first-child{display:flex;flex-direction:column;gap:3px}.conn-actions{display:flex;gap:4px}.conn-actions button{display:flex;align-items:center;gap:4px}
  .address-list{padding:0 12px 10px;display:flex;flex-direction:column;gap:3px}.address-list button{padding:7px 9px;border:1px solid var(--border2);border-radius:var(--ui-radius-control);background:var(--input-bg);color:var(--text3);font:var(--fs-sub) var(--font-mono);text-align:left;word-break:break-all;cursor:pointer}.address-list button.active{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}
  .hook-row{border-top:1px solid var(--border2)}.hook-control{display:flex;align-items:center;gap:7px;flex-shrink:0}.hook-backends{display:flex;gap:3px}.hook-backends span{padding:2px 5px;border-radius:var(--ui-radius-pill);background:var(--surface2);color:var(--text3);font-size:var(--fs-micro)}.hook-backends span.on{background:var(--accent-bg);color:var(--accent)}.hook-action{height:var(--ui-control-height);padding:3px 8px;border:1px solid var(--border2);border-radius:var(--ui-radius-control);background:transparent;color:var(--text3);font-size:var(--ui-font-control);cursor:pointer}.hook-action.primary{border-color:var(--accent);background:var(--accent-bg);color:var(--accent)}.hook-action:disabled{opacity:.5}.hook-error{padding:0 12px 8px;color:var(--danger);font-size:var(--fs-meta)}
  .disconnect{display:block;width:min(720px,100%);margin:8px auto;padding:7px;border:1px solid color-mix(in srgb,var(--danger) 55%,transparent);border-radius:var(--ui-radius-control);background:none;color:var(--danger);cursor:pointer;font-size:var(--fs-sub);font-weight:600}
  .empty-connection{padding:28px 12px;display:flex;flex-direction:column;align-items:center;gap:9px;color:var(--text3);font-size:var(--fs-sub)}.empty-connection button{padding:6px 10px;border:1px solid var(--accent);border-radius:var(--ui-radius-control);background:var(--accent-bg);color:var(--accent);cursor:pointer;font-size:var(--fs-sub)}
  /* ONE compact breakpoint with the rest of the app (760): this page used a
     private 640, so a narrow window wore desktop clothes here after every
     other page had switched. The old `inset` hack predates the page-layer
     and positioned nothing. */
  @media(max-width:760px){.pref-content{padding:12px}.setting-row{min-height:50px;padding:8px 10px;gap:10px}.text-input{width:min(220px,48vw)}.range-wrap{min-width:min(250px,52vw)}.connection-title{align-items:flex-start;flex-direction:column}.conn-actions{width:100%}.conn-actions button{flex:1;justify-content:center}.hook-row{align-items:flex-start;flex-direction:column}.hook-control{width:100%;justify-content:space-between}.segmented button,.stepper button,.hook-action{min-height:32px}}
  @media(max-width:420px){.setting-row{align-items:flex-start;flex-direction:column;gap:7px}.setting-row>div:last-child,.text-input,.range-wrap{width:100%;min-width:0}.font-control{align-items:flex-start}.segmented button{flex:1}}
</style>
