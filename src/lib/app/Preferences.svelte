<script lang="ts">
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import Select from '../ui/Select.svelte';
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
    onGoBack = null,
    onDrill = () => {},
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
    onGoBack?: ((fn: () => boolean) => void) | null;
    onDrill?: () => void;
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
  // No icons on the category rows (owner, 2026-08-25: "三个子页面就不要图标
  // 了，不好看") — the words carry it, like the Chat sidebar's project rows.
  const tabs = $derived([
    { id: 'appearance', label: () => t('settingsAppearance') },
    { id: 'terminal', label: () => t('settingsTerminal') },
    ...(showShortcuts ? [{ id: 'shortcuts', label: () => t('settingsShortcuts') }] : []),
    { id: 'connection', label: () => t('settingsConnection') },
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

  function setLineHeight(value: number) {
    terminalPrefs.setLineHeight(Math.round(value * 100) / 100);
  }

  // Compact drill-down (owner, 2026-08-25: "上边一行三个标签这个风格和别的
  // 页面太不一样"): the phone shows the CATEGORY LIST first — the same shared
  // sidebar every page has — and a tap opens that category full screen, the
  // AgentsPage editor pattern. catOpen only means anything under 760px.
  let catOpen = $state(false);
  // Drill motion (the navigation grammar in design-language.md §1): opening a
  // category slides it in from the RIGHT, backing out slides the list in from
  // the LEFT — the same 120ms the app-level tab slide speaks. The class
  // toggling fwd↔back is what replays the animation; no timers.
  let drillAnim = $state('');
  const isCompact = () => window.matchMedia('(max-width: 760px)').matches;
  let drillPushed = false;
  function closeCat() { catOpen = false; drillAnim = 'back'; drillPushed = false; }
  $effect(() => {
    onGoBack?.(() => {
      if (catOpen && isCompact()) { closeCat(); return true; }
      return false;
    });
  });

  function selectTab(value: string) {
    if (!catOpen && isCompact()) { drillAnim = 'fwd'; onDrill(); drillPushed = true; }
    catOpen = true;
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
<section class="preferences" class:cat-open={catOpen} class:drill-fwd={drillAnim === 'fwd'} class:drill-back={drillAnim === 'back'} aria-label={t('settings')}>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll">
      <div class="side-h">{t('settings')}</div>
      {#each tabs as item}
        <button class="side-row" class:open={tab === item.id} onclick={() => selectTab(item.id)}>
          <span class="r-label">{item.label()}</span>
        </button>
      {/each}
    </div>
  </aside>
  <div class="pref-shell">
    <div class="page-head">
      <!-- Compact only: the way back to the category list (the back gesture
           does the same through onGoBack). -->
      <button class="icon-btn back" title={t('settings')} aria-label={t('settings')}
        onclick={() => drillPushed ? history.back() : closeCat()}>
        <Icon name="chevron-left" size={15} />
      </button>
      <h1>{tabs.find((x) => x.id === tab)?.label() ?? t('settings')}</h1>
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
              <Select bind:value={uiFontInput} editable dense options={uiFont.common}
                placeholder={t('fontFamilySystem')} ariaLabel={t('uiFontBody')}
                onchange={() => saveUiFont()} />
              {#if uiFontInvalid}<small class="font-error">{t('fontFamilyInvalid')}</small>{/if}
            </div>
          </div>
          <div class="setting-row">
            <div><strong>{t('uiFontDisplay')}</strong><small>{t('uiFontDisplayHint')}</small></div>
            <div class="font-control">
              <Select bind:value={displayFontInput} editable dense options={displayFont.common}
                placeholder={t('fontFamilySystem')} ariaLabel={t('uiFontDisplay')}
                onchange={() => saveDisplayFont()} />
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
              <Select bind:value={fontInput} editable dense options={fonts.common}
                placeholder={t('fontFamilySystem')} ariaLabel={t('fontFamily')}
                onchange={() => saveFont()} />
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
  .pref-shell { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .back { display: none; }
  /* Compact = the drill-down every other page speaks (AgentsPage's editor
     pattern): the category LIST is the first screen — the same shared sidebar,
     full width — and an open category takes the whole screen with a back
     button in its head. The chip row this replaces was a third navigation
     species nothing else wore. */
  @media (max-width: 760px) {
    .preferences { grid-template-columns: minmax(0, 1fr); }
    .preferences:not(.cat-open) .pref-shell { display: none; }
    .preferences.cat-open .sidebar { display: none; }
    .sidebar { border-right: none; }
    .sidebar :global(.side-row) { min-height: 44px; }
    .back { display: grid; }
    /* Drill motion, compact only: deeper enters from the right, back from
       the left — same 120ms grammar as the app-level page slide. */
    .preferences.drill-fwd .pref-shell { animation: drill-in-right 0.12s linear; }
    .preferences.drill-back .sidebar { animation: drill-in-left 0.12s linear; }
  }
  @keyframes drill-in-right { from { transform: translateX(40%); } to { transform: none; } }
  @keyframes drill-in-left  { from { transform: translateX(-40%); } to { transform: none; } }
  @media (prefers-reduced-motion: reduce) {
    .preferences.drill-fwd .pref-shell, .preferences.drill-back .sidebar { animation: none; }
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
  .font-control{display:flex;flex-direction:column;align-items:flex-end;gap:3px}.font-error{color:var(--danger)}
  .font-control :global(.sel-combo){width:min(230px,42vw)}
  .font-control :global(.sel-trigger.combo){width:100%;font-family:var(--font-mono)}
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
  /* Lone danger dialect (design-language.md §3): quiet at rest — border2 box,
     red ink — and only hover raises the red border + wash. The always-red
     55% border was this button's own species. */
  .disconnect{display:block;width:min(720px,100%);margin:8px auto;padding:7px;border:1px solid var(--border2);border-radius:var(--ui-radius-control);background:none;color:var(--danger);cursor:pointer;font-size:var(--fs-sub);font-weight:600;transition:border-color var(--t-fast),background var(--t-fast)}
  .disconnect:hover{border-color:var(--danger);background:var(--danger-bg)}
  .empty-connection{padding:28px 12px;display:flex;flex-direction:column;align-items:center;gap:9px;color:var(--text3);font-size:var(--fs-sub)}.empty-connection button{padding:6px 10px;border:1px solid var(--accent);border-radius:var(--ui-radius-control);background:var(--accent-bg);color:var(--accent);cursor:pointer;font-size:var(--fs-sub)}
  /* ONE compact breakpoint with the rest of the app (760): this page used a
     private 640, so a narrow window wore desktop clothes here after every
     other page had switched. The old `inset` hack predates the page-layer
     and positioned nothing. */
  @media(max-width:760px){.pref-content{padding:12px}.setting-row{min-height:50px;padding:8px 10px;gap:10px}.font-control :global(.sel-combo){width:min(220px,48vw)}.range-wrap{min-width:min(250px,52vw)}.connection-title{align-items:flex-start;flex-direction:column}.conn-actions{width:100%}.conn-actions button{flex:1;justify-content:center}.hook-row{align-items:flex-start;flex-direction:column}.hook-control{width:100%;justify-content:space-between}.segmented button,.stepper button,.hook-action{min-height:32px}}
  @media(max-width:420px){.setting-row{align-items:flex-start;flex-direction:column;gap:7px}.setting-row>div:last-child,.range-wrap{width:100%;min-width:0}.font-control{align-items:flex-start}.font-control :global(.sel-combo){width:100%}.segmented button{flex:1}}
</style>
