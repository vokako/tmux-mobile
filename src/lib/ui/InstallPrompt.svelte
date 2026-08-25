<script lang="ts">
  // PWA install offer — browser-only banner inviting the user to install the
  // web UI as a standalone app.
  //
  // Two paths:
  //   • Chromium (Chrome/Edge/Android/Samsung): the browser fires
  //     `beforeinstallprompt`; we capture it, show our own banner, and call
  //     the saved event's prompt() when the user taps Install — triggering
  //     the native install dialog.
  //   • iOS Safari: there is NO programmatic install API, so we detect it and
  //     show short "Share → Add to Home Screen" instructions instead.
  //
  // Never shown inside the Tauri shell (already native) or once the app is
  // already running standalone (installed). Dismissals are remembered for a
  // cooldown window so we don't nag.
  import { t } from '../core/i18n.svelte.ts';

  // Chromium-only event; not in TS DOM libs.
  interface BeforeInstallPromptEvent extends Event {
    prompt(): Promise<void>;
    userChoice: Promise<unknown>;
  }

  const DISMISS_KEY = 'tmux_pwa_dismissed';
  const DISMISS_COOLDOWN_MS = 14 * 24 * 60 * 60 * 1000; // 14 days

  const isTauri = typeof window !== 'undefined' && !!(window.__TAURI__ || window.__TAURI_INTERNALS__);

  function isStandalone() {
    return (
      window.matchMedia?.('(display-mode: standalone)').matches ||
      (window.navigator as Navigator & { standalone?: boolean }).standalone === true
    );
  }

  function isIos() {
    const ua = navigator.userAgent || '';
    // iPadOS 13+ masquerades as macOS; distinguish by touch support.
    return (
      /iphone|ipad|ipod/i.test(ua) ||
      (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
    );
  }

  function isIosSafari() {
    if (!isIos()) return false;
    const ua = navigator.userAgent || '';
    // On iOS, "Add to Home Screen" only exists in Safari — exclude in-app
    // browsers and other engines (CriOS/Chrome, FxiOS/Firefox, EdgiOS, etc.).
    return /safari/i.test(ua) && !/crios|fxios|edgios|opt\//i.test(ua);
  }

  function dismissedRecently() {
    try {
      const ts = parseInt(localStorage.getItem(DISMISS_KEY) || '0', 10);
      return ts > 0 && Date.now() - ts < DISMISS_COOLDOWN_MS;
    } catch {
      return false;
    }
  }

  let visible = $state(false);
  let mode = $state<'' | 'prompt' | 'ios'>('');
  let deferredPrompt: BeforeInstallPromptEvent | null = null;

  $effect(() => {
    if (isTauri || isStandalone() || dismissedRecently()) return;

    function onBeforeInstall(e: Event) {
      // Prevent the mini-infobar; we drive the offer ourselves.
      e.preventDefault();
      deferredPrompt = e as BeforeInstallPromptEvent;
      mode = 'prompt';
      visible = true;
    }

    function onInstalled() {
      visible = false;
      deferredPrompt = null;
      try {
        localStorage.removeItem(DISMISS_KEY);
      } catch {}
    }

    window.addEventListener('beforeinstallprompt', onBeforeInstall);
    window.addEventListener('appinstalled', onInstalled);

    // iOS never fires beforeinstallprompt — offer manual instructions.
    let iosTimer: ReturnType<typeof setTimeout> | null = null;
    if (isIosSafari()) {
      // Small delay so the banner doesn't fight the first paint / connect UI.
      iosTimer = setTimeout(() => {
        mode = 'ios';
        visible = true;
      }, 2500);
    }

    return () => {
      window.removeEventListener('beforeinstallprompt', onBeforeInstall);
      window.removeEventListener('appinstalled', onInstalled);
      if (iosTimer) clearTimeout(iosTimer);
    };
  });

  async function install() {
    if (!deferredPrompt) return;
    deferredPrompt.prompt();
    try {
      await deferredPrompt.userChoice;
    } catch {}
    deferredPrompt = null;
    visible = false;
  }

  function dismiss() {
    visible = false;
    try {
      localStorage.setItem(DISMISS_KEY, String(Date.now()));
    } catch {}
  }
</script>

{#if visible}
  <div class="install-banner" role="dialog" aria-label={t('installTitle')}>
    <img class="install-icon" src="/pwa-192.png" alt="" width="40" height="40" />
    <div class="install-body">
      <div class="install-title">{t('installTitle')}</div>
      <div class="install-desc">
        {#if mode === 'ios'}
          {t('installIosHint')}
        {:else}
          {t('installDesc')}
        {/if}
      </div>
    </div>
    <div class="install-actions">
      {#if mode === 'prompt'}
        <button class="install-btn" onclick={install}>{t('installBtn')}</button>
      {/if}
      <button class="install-dismiss" onclick={dismiss} aria-label={t('installLater')}>
        {t('installLater')}
      </button>
    </div>
  </div>
{/if}

<style>
  .install-banner {
    position: fixed;
    left: 50%;
    transform: translateX(-50%);
    bottom: calc(12px + var(--sab, 0px)); /* var(--sab): env() is 0 in the APK */
    z-index: 200;
    width: min(440px, calc(100vw / var(--ui-zoom, 1) - 24px));
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: 16px;
    background: var(--nav-bg);
    border: 1px solid var(--border);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    animation: install-rise 0.32s cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes install-rise {
    from { opacity: 0; transform: translateX(-50%) translateY(16px); }
    to { opacity: 1; transform: translateX(-50%) translateY(0); }
  }
  .install-icon {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    flex-shrink: 0;
  }
  .install-body {
    flex: 1;
    min-width: 0;
  }
  .install-title {
    font-size: var(--fs-body);
    font-weight: 600;
    color: var(--text);
  }
  .install-desc {
    font-size: var(--fs-sub);
    color: var(--text2);
    margin-top: 2px;
    line-height: 1.35;
  }
  .install-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .install-btn {
    padding: 7px 14px;
    border: none;
    border-radius: 999px;
    background: var(--accent-fill);
    color: var(--accent-fill-ink);
    font-size: var(--fs-ui);
    font-weight: 600;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    font-family: inherit;
  }
  .install-btn:active { transform: scale(0.94); }
  .install-dismiss {
    padding: 7px 10px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text3);
    font-size: var(--fs-ui);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    font-family: inherit;
    white-space: nowrap;
  }
  .install-dismiss:active { color: var(--text2); }
</style>
