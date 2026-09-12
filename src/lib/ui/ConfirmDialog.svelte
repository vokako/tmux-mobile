<script lang="ts">
  // The app's ONE confirmation. Before this, a destructive action could be any
  // of four things: a modal (the Hub), a button you click TWICE (Files, Team,
  // TeamTemplates), the browser's own `confirm()` (the file editor's discard —
  // an OS dialog, the same seam a native <select> opens), or nothing at all
  // (deleting an agent definition, a skill, an MCP server, a tmux session).
  // Owner, 2026-08-19: "所有的 close delete 之类的按钮都检查一下二次确认，并且
  // 交互要好看一些".
  //
  // Two rules make it worth sharing rather than copying:
  //  · the WORDS are the caller's (what is lost, and what survives — a stopped
  //    agent keeps its conversation, a closed project keeps its declaration), so
  //    the component owns none of them;
  //  · the SHAPE is the app's — a centred card on the desktop, a bottom sheet on
  //    the phone where a thumb lives, dismissible by backdrop and Escape, with
  //    the destructive verb on the right and focus parked on Cancel so a stray
  //    Enter cannot delete anything.
  import { t } from '../core/i18n.svelte.ts';
  import CommandButton from './CommandButton.svelte';
  import { activeModal } from './modal.ts';

  let {
    open = false,
    title = '',
    note = '',
    confirmLabel = '',
    confirmIcon = 'check',
    cancelLabel = '',
    error = '',
    /** The confirming button carries the danger tone; false for a neutral
     * confirmation (discarding an edit is not the same as deleting a file). */
    danger = true,
    busy = false,
    compact = false,
    onconfirm = () => {},
    oncancel = () => {},
  } = $props();

  const errorId = $props.id();
  let cancelEl: HTMLButtonElement | null = $state(null);
  let dialogEl: HTMLDivElement | null = $state(null);
  // Focus lands on Cancel, never on the destructive verb: the dialog appears
  // under the pointer/keyboard of someone who was just clicking things.
  $effect(() => {
    if (!open || !cancelEl || !dialogEl) return;
    const dialog = dialogEl;
    const previousFocus = document.activeElement;
    const onKey = (e: KeyboardEvent) => {
      if (activeModal(document) !== dialog) return;
      if (e.key === 'Escape') {
        e.stopPropagation(); e.preventDefault();
        if (!busy) oncancel();
      } else if (e.key === 'Tab') {
        const buttons = [...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
        if (!buttons.length) { e.preventDefault(); dialog.focus(); return; }
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = e.shiftKey ? (index <= 0 ? buttons.length - 1 : index - 1) : (index + 1) % buttons.length;
        e.preventDefault(); buttons[next]!.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      const owner = activeModal(document);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected
        && (!owner || owner === dialog || owner.contains(previousFocus))) previousFocus.focus();
    };
  });
  // Busy is a focus transition, not a new dialog lifetime: disabling the
  // clicked button otherwise leaves BODY focused and loses keyboard ownership.
  $effect(() => {
    if (!open || !cancelEl || !dialogEl || activeModal(document) !== dialogEl) return;
    if (busy) dialogEl.focus({ preventScroll: true });
    else if (!dialogEl.contains(document.activeElement)) cancelEl.focus({ preventScroll: true });
  });
</script>

{#if open}
  <div class="dlg-backdrop" onclick={() => { if (!busy) oncancel(); }} role="presentation"></div>
  <div class="dlg confirm" class:sheet={compact} role="alertdialog" aria-modal="true" aria-label={title}
    aria-describedby={error ? errorId : undefined}
    aria-busy={busy || undefined} tabindex="-1" bind:this={dialogEl}>
    <h2>{title}</h2>
    {#if note}<p class="dlg-note">{note}</p>{/if}
    {#if error}<p class="config-error dlg-error" id={errorId} role="alert">{error}</p>{/if}
    <div class="dlg-actions">
      <CommandButton label={cancelLabel || t('cancel')} disabled={busy}
        bind:element={cancelEl} onclick={() => { if (!busy) oncancel(); }} />
      <CommandButton label={confirmLabel || t('confirm')} variant={danger ? 'danger' : 'primary'}
        icon={confirmIcon} destructiveConfirm={danger} pending={busy}
        onclick={() => { if (!busy) onconfirm(); }} />
    </div>
  </div>
{/if}

<style>
  /* Same dialog shape as the Hub's — this component IS that dialog, lifted. */
  /* Motion (motion.md): the scrim fades in on --t-move beside the sheet; the
     desktop dialog only FADES (--t-fast) — it is centred BY a transform, so a
     transform keyframe on it would fight its own placement. */
  .dlg-backdrop { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); animation: fade-in var(--t-move) ease-out; }
  .dlg {
    position: fixed; z-index: 61; left: 50%; top: 50%; transform: translate(-50%, -50%);
    animation: fade-in var(--t-fast) ease-out;
    width: min(420px, calc(100vw / var(--ui-zoom, 1) - 32px));
    max-height: calc(100vh / var(--ui-zoom, 1) - 48px); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--control-dialog-radius);
    box-shadow: 0 18px 60px rgba(0, 0, 0, 0.5); padding: 18px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .dlg h2 { margin: 0; font-size: var(--fs-title); }
  .dlg-note { margin: 0; color: var(--text2); font-size: var(--fs-ui); line-height: 1.55; }
  .dlg h2, .dlg-note, .dlg-error { overflow-wrap: anywhere; }
  .dlg-error { margin: 0; }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
  /* Phone: a bottom sheet — reachable with a thumb, and it never fights the
     on-screen keyboard for the middle of the screen. It RISES from the bottom
     edge (sheet-up, app.css) because its resting transform is none; the
     standing `will-change: opacity` is the one sanctioned sheet hint
     (design-language §1): the Android System WebView drops the compositor
     layer when the rise ends and blinks a blank frame while it re-rasterizes,
     and opacity promotes a layer without becoming a containing block. */
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
