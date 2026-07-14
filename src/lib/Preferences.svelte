<script>
  import Icon from './Icon.svelte';
  import { t, i18n, setLocale } from './i18n.svelte.js';
  import { layout } from './layout.svelte.js';
  import { fonts } from './fonts.svelte.js';
  import { terminalPrefs } from './terminal-prefs.svelte.js';

  let {
    connected = false,
    theme = 'system',
    fontSize = 14,
    debugMode = false,
    serverInfo = { hostname: '', machineId: '' },
    activeAddress = '',
    addresses = [],
    optimizing = false,
    linkCopied = false,
    onClose = () => {},
    onTheme = () => {},
    onFontSize = () => {},
    onDebug = () => {},
    onOptimize = () => {},
    onShare = () => {},
    onAddress = () => {},
    onDisconnect = () => {},
    onConnectionSetup = () => {},
  } = $props();

  let tab = $state('appearance');
  const tabs = [
    { id: 'appearance', label: () => t('settingsAppearance'), icon: 'palette' },
    { id: 'terminal', label: () => t('terminal'), icon: 'terminal' },
    { id: 'connection', label: () => t('settingsConnection'), icon: 'link' },
  ];

  function setLineHeight(value) {
    terminalPrefs.setLineHeight(Math.round(value * 100) / 100);
  }
</script>

<section class="preferences" aria-label={t('settings')}>
  <header class="pref-head">
    <div>
      <h1>{t('settings')}</h1>
      <p>{t('settingsHint')}</p>
    </div>
    <button class="close" onclick={onClose} aria-label={t('close')}><Icon name="x" size={18} /></button>
  </header>

  <div class="pref-shell">
    <nav class="pref-tabs" aria-label={t('settings')}>
      {#each tabs as item}
        <button class:active={tab === item.id} onclick={() => tab = item.id}>
          <Icon name={item.icon} size={15} /><span>{item.label()}</span>
        </button>
      {/each}
    </nav>

    <div class="pref-content">
      {#if tab === 'appearance'}
        <div class="section-head"><h2>{t('settingsAppearance')}</h2><p>{t('settingsAppearanceHint')}</p></div>
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
        </div>
      {:else if tab === 'terminal'}
        <div class="section-head"><h2>{t('terminal')}</h2><p>{t('settingsTerminalHint')}</p></div>
        <div class="setting-card">
          <div class="setting-row">
            <div><strong>{t('fontFamily')}</strong><small>{t('fontFamilyHint')}</small></div>
            <input class="text-input" type="text" placeholder={t('fontFamilySystem')} value={fonts.custom}
              autocapitalize="off" autocomplete="off" spellcheck="false"
              onchange={(e) => fonts.set(e.target.value)}
              onkeydown={(e) => { if (e.key === 'Enter') { fonts.set(e.target.value); e.target.blur(); } }} />
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
              <input type="range" min="1" max="1.6" step="0.05" value={terminalPrefs.lineHeight} oninput={(e) => setLineHeight(+e.currentTarget.value)} />
              <span>{terminalPrefs.lineHeight.toFixed(2)}</span>
              <button class="reset" onclick={() => setLineHeight(1)}>↺</button>
            </div>
          </div>
        </div>
      {:else}
        <div class="section-head"><h2>{t('settingsConnection')}</h2><p>{t('settingsConnectionHint')}</p></div>
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
          {:else}
            <div class="empty-connection"><Icon name="link" size={20} /><span>{t('notConnected')}</span><button onclick={onConnectionSetup}>{t('connectionSetup')}</button></div>
          {/if}
          <div class="setting-row compact">
            <div><strong>{t('debug')}</strong><small>{t('debugHint')}</small></div>
            <button class="toggle" class:on={debugMode} onclick={() => onDebug(!debugMode)}><span></span>{debugMode ? t('on') : t('off')}</button>
          </div>
        </div>
        {#if connected}<button class="disconnect" onclick={onDisconnect}>{t('disconnect')}</button>{/if}
      {/if}
    </div>
  </div>
</section>

<style>
  .preferences { position: fixed; inset: calc(49px + var(--sat)) 0 0; z-index: 19; display:flex; flex-direction:column; background:var(--bg); color:var(--text); }
  .pref-head { display:flex; align-items:center; justify-content:space-between; padding:18px clamp(18px,4vw,42px); border-bottom:1px solid var(--border); background:var(--surface); flex-shrink:0; }
  h1,h2,p { margin:0; } h1{font-size:22px;} .pref-head p,.section-head p{font-size:12px;color:var(--text3);margin-top:4px;}
  .close { width:36px;height:36px;border:1px solid var(--border);border-radius:10px;background:var(--input-bg);color:var(--text2);display:flex;align-items:center;justify-content:center;cursor:pointer; }
  .pref-shell { flex:1;min-height:0;display:flex; }
  .pref-tabs { width:190px;padding:18px 12px;border-right:1px solid var(--border);display:flex;flex-direction:column;gap:5px;background:var(--surface); }
  .pref-tabs button { display:flex;align-items:center;gap:10px;padding:10px 12px;border:0;border-radius:9px;background:none;color:var(--text3);font-size:13px;text-align:left;cursor:pointer; }
  .pref-tabs button.active { background:var(--accent-bg);color:var(--accent);font-weight:600; }
  .pref-content { flex:1;min-width:0;overflow:auto;padding:clamp(20px,4vw,44px); }
  .section-head { max-width:760px;margin:0 auto 18px; } .section-head h2{font-size:20px;}
  .setting-card { max-width:760px;margin:0 auto;border:1px solid var(--border);border-radius:14px;background:var(--surface);overflow:hidden; }
  .setting-row { min-height:68px;padding:13px 16px;display:flex;align-items:center;justify-content:space-between;gap:20px; }
  .setting-row+.setting-row { border-top:1px solid var(--border2); } .setting-row>div:first-child{display:flex;flex-direction:column;gap:4px;min-width:0;}
  strong{font-size:13px;} small{font-size:11px;color:var(--text3);font-weight:400;}
  .segmented { display:flex;background:var(--pill-bg);padding:2px;border-radius:9px;flex-shrink:0; }
  .segmented button,.stepper button,.reset,.conn-actions button { border:0;background:transparent;color:var(--text3);padding:7px 11px;border-radius:7px;cursor:pointer;font-size:12px; }
  .segmented button.active { background:var(--accent-bg);color:var(--accent); }
  .text-input { width:min(260px,45vw);padding:9px 11px;border:1px solid var(--input-border);border-radius:8px;background:var(--input-bg);color:var(--text);font-family:var(--font-mono);outline:none; }
  .text-input:focus{border-color:var(--accent);}
  .stepper { display:flex;align-items:center;gap:4px; }.stepper button{width:32px;height:32px;border:1px solid var(--border);padding:0;font-size:16px}.stepper span{min-width:52px;text-align:center;font-family:var(--font-mono);font-size:12px;}
  .range-wrap { display:flex;align-items:center;gap:10px;min-width:min(320px,50vw); }.range-wrap input{flex:1;accent-color:var(--accent)}.range-wrap span{width:34px;font:11px var(--font-mono);color:var(--text2)}.reset{padding:5px 7px;border:1px solid var(--border)}
  .connection-title{padding:16px;display:flex;align-items:center;justify-content:space-between;gap:12px}.connection-title>div:first-child{display:flex;flex-direction:column;gap:4px}.conn-actions{display:flex;gap:6px}.conn-actions button{display:flex;align-items:center;gap:5px;border:1px solid var(--border)}
  .address-list{padding:0 16px 16px;display:flex;flex-direction:column;gap:5px}.address-list button{padding:10px 12px;border:1px solid var(--border2);border-radius:8px;background:var(--input-bg);color:var(--text3);font:12px var(--font-mono);text-align:left;word-break:break-all;cursor:pointer}.address-list button.active{border-color:var(--accent);color:var(--accent)}
  .compact{border-top:1px solid var(--border2)}.toggle{border:0;border-radius:999px;background:var(--pill-bg);color:var(--text3);padding:5px 10px 5px 5px;display:flex;align-items:center;gap:7px;cursor:pointer}.toggle span{width:17px;height:17px;border-radius:50%;background:var(--text3)}.toggle.on span{background:var(--accent)}
  .disconnect{display:block;width:min(760px,100%);margin:16px auto;padding:11px;border:1px solid var(--danger);border-radius:10px;background:none;color:var(--danger);cursor:pointer;font-weight:600}
  .empty-connection{padding:40px 16px;display:flex;flex-direction:column;align-items:center;gap:12px;color:var(--text3)}.empty-connection button{padding:9px 14px;border:1px solid var(--accent);border-radius:8px;background:var(--accent-bg);color:var(--accent);cursor:pointer}
  @media(max-width:640px){.preferences{inset:calc(49px + var(--sat)) 0 0}.pref-head{padding:12px 14px}.pref-head p{display:none}.pref-shell{flex-direction:column}.pref-tabs{width:100%;padding:7px 8px;border-right:0;border-bottom:1px solid var(--border);flex-direction:row}.pref-tabs button{flex:1;justify-content:center;padding:8px 6px}.pref-content{padding:18px 12px}.setting-row{align-items:flex-start;flex-direction:column;gap:10px}.setting-row>div:last-child,.text-input,.range-wrap{width:100%;min-width:0}.segmented button{flex:1}.connection-title{align-items:flex-start;flex-direction:column}.conn-actions{width:100%}.conn-actions button{flex:1;justify-content:center}}
</style>
