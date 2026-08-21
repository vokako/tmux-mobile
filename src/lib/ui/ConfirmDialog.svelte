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

  let {
    open = false,
    title = '',
    note = '',
    confirmLabel = '',
    cancelLabel = '',
    /** The confirming button carries the danger tone; false for a neutral
     * confirmation (discarding an edit is not the same as deleting a file). */
    danger = true,
    busy = false,
    compact = false,
    onconfirm = () => {},
    oncancel = () => {},
  } = $props();

  let cancelEl: HTMLButtonElement | null = $state(null);
  // Focus lands on Cancel, never on the destructive verb: the dialog appears
  // under the pointer/keyboard of someone who was just clicking things.
  $effect(() => { if (open && cancelEl) cancelEl.focus(); });

  $effect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.stopPropagation(); oncancel(); }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });
</script>

{#if open}
  <div class="dlg-backdrop" onclick={() => oncancel()} role="presentation"></div>
  <div class="dlg confirm" class:sheet={compact} role="alertdialog" aria-modal="true" aria-label={title}>
    <h2>{title}</h2>
    {#if note}<p class="dlg-note">{note}</p>{/if}
    <div class="dlg-actions">
      <button class="chip-btn" bind:this={cancelEl} onclick={() => oncancel()}>
        {cancelLabel || t('cancel')}
      </button>
      <button class="chip-btn primary" class:danger disabled={busy} onclick={() => onconfirm()}>
        {busy ? '…' : (confirmLabel || t('delete'))}
      </button>
    </div>
  </div>
{/if}

<style>
  /* Same dialog shape as the Hub's — this component IS that dialog, lifted. */
  .dlg-backdrop { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); }
  .dlg {
    position: fixed; z-index: 61; left: 50%; top: 50%; transform: translate(-50%, -50%);
    width: min(420px, calc(100vw / var(--ui-zoom, 1) - 32px));
    max-height: calc(100vh / var(--ui-zoom, 1) - 48px); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 16px;
    box-shadow: 0 18px 60px rgba(0, 0, 0, 0.5); padding: 18px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .dlg h2 { margin: 0; font-size: var(--fs-title); }
  .dlg-note { margin: 0; color: var(--text2); font-size: var(--fs-ui); line-height: 1.55; }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
  .dlg-actions button { min-height: 34px; }
  /* Phone: a bottom sheet — reachable with a thumb, and it never fights the
     on-screen keyboard for the middle of the screen. */
  .dlg.sheet {
    left: 0; top: auto; bottom: 0; transform: none;
    width: 100%; max-width: none; border-radius: 16px 16px 0 0;
    border-left: none; border-right: none; border-bottom: none;
    padding: 16px 14px calc(16px + env(safe-area-inset-bottom));
  }
  .dlg.sheet .dlg-actions button { min-height: 44px; flex: 1; justify-content: center; }
</style>
