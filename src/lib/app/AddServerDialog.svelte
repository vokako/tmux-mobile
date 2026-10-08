<script lang="ts">
  // Add server (board 315): the connect fields in the app's dialog shape — a
  // centred card on the desktop, a bottom sheet on the phone — opened INSIDE
  // the connected shell. You are still connected while it is open: opening
  // and cancelling write nothing and leave the current server live, and the
  // background follows the modal rules (activeModal owns the keyboard, the
  // scrim takes the pointer). Submitting hands the candidate to the ONE
  // switch path; a failure is reported there, in place, with Retry · Back ·
  // Edit (Edit reopens this dialog prefilled).
  import { untrack } from 'svelte';
  import ConnectFields from './ConnectFields.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import { activeModal } from '../ui/modal.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { normalizeAddress } from '../core/connection-address.ts';

  type HistoryEntry = { address: string; token: string };
  type Candidate = { address: string; token: string; socket?: string };
  let {
    initial = null, compact = false, onsubmit, oncancel,
  }: { initial?: Candidate | null; compact?: boolean; onsubmit: (c: Candidate) => void; oncancel: () => void } = $props();

  // The dialog is mounted per opening, so the prefill is read once on purpose.
  const seed = untrack(() => initial);
  let address = $state(seed?.address ?? '');
  let token = $state(seed?.token ?? '');
  let socket = $state(seed?.socket ?? '');
  let history = $state<HistoryEntry[]>((() => {
    try {
      const raw = JSON.parse(localStorage.getItem('tmux_address_history') || '[]');
      return raw.map((h: string | HistoryEntry) => typeof h === 'string' ? { address: h, token: '' } : h);
    } catch { return []; }
  })());
  let dialogEl: HTMLDivElement | null = $state(null);

  function submit() {
    if (!address.trim()) return;
    onsubmit({ address: normalizeAddress(address, location.protocol), token, ...(socket.trim() ? { socket: socket.trim() } : {}) });
  }

  $effect(() => {
    const dialog = dialogEl;
    if (!dialog) return;
    const previousFocus = document.activeElement;
    const onKey = (e: KeyboardEvent) => {
      if (activeModal(document) !== dialog) return;
      if (e.key === 'Escape' && !e.isComposing) { e.stopPropagation(); e.preventDefault(); oncancel(); }
      else if (e.key === 'Tab') {
        // Focus stays inside, as in ConfirmDialog: the shell behind is not
        // reachable while the dialog is up.
        const items = [...dialog.querySelectorAll<HTMLElement>('input, button:not(:disabled)')];
        if (!items.length) return;
        const index = items.indexOf(document.activeElement as HTMLElement);
        const next = e.shiftKey ? (index <= 0 ? items.length - 1 : index - 1) : (index + 1) % items.length;
        e.preventDefault(); items[next]!.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus({ preventScroll: true });
    };
  });
</script>

<div class="dlg-backdrop" onclick={oncancel} role="presentation"></div>
<div class="dlg" class:sheet={compact} role="dialog" aria-modal="true" aria-label={t('serverAdd')} tabindex="-1" bind:this={dialogEl}>
  <h2>{t('serverAdd')}</h2>
  <ConnectFields bind:address bind:token bind:socket bind:history autofocus onenter={submit} />
  <div class="dlg-actions">
    <CommandButton label={t('cancel')} onclick={oncancel} />
    <CommandButton label={t('connect')} variant="primary" icon="link" disabled={!address.trim()} onclick={submit} />
  </div>
</div>

<style>
  /* ConfirmDialog's shape and motion (the app's one dialog form). */
  .dlg-backdrop { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); animation: fade-in var(--t-move) ease-out; }
  .dlg {
    position: fixed; z-index: 61; left: 50%; top: 50%; transform: translate(-50%, -50%);
    animation: fade-in var(--t-fast) ease-out;
    width: min(420px, calc(100vw / var(--ui-zoom, 1) - 32px));
    max-height: calc(100vh / var(--ui-zoom, 1) - 48px); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--control-dialog-radius);
    box-shadow: 0 18px 60px rgba(0, 0, 0, 0.5); padding: 18px;
    display: flex; flex-direction: column; gap: 14px;
  }
  .dlg h2 { margin: 0; font-size: var(--fs-title); }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 2px; }
  .dlg.sheet {
    left: 0; top: auto; bottom: 0; transform: none;
    width: 100%; max-width: none; border-radius: var(--control-dialog-radius) var(--control-dialog-radius) 0 0;
    border-left: none; border-right: none; border-bottom: none;
    padding: 16px 14px calc(16px + var(--sab, 0px)); /* var(--sab): env() is 0 in the APK */
    animation: sheet-up var(--t-move) ease-out;
    will-change: opacity;
  }
  .dlg.sheet .dlg-actions :global(.command-button) { min-height: 44px; flex: 1; justify-content: center; }
  @media (prefers-reduced-motion: reduce) { .dlg-backdrop, .dlg, .dlg.sheet { animation: none; } }
</style>
