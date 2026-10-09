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
  //  · the SHAPE is the app's — and since #317 the shape is not ours either:
  //    `ui/Dialog` owns the scrim, the card, the sheet, Escape, the Tab trap
  //    and the focus restore for every dialog in the app. What is left here is
  //    the confirmation itself: the alert role, the two commands with the
  //    destructive verb on the right, and the busy rules.
  import { t } from '../core/i18n.svelte.ts';
  import CommandButton from './CommandButton.svelte';
  import Dialog from './Dialog.svelte';
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
  let dialogEl: HTMLDivElement | null = $state(null);
  let shell: { focusDefault: () => void } | null = $state(null);
  // Busy is a focus transition, not a new dialog lifetime: disabling the
  // clicked button otherwise leaves BODY focused and loses keyboard ownership.
  // Focus lands back on Cancel — the shell's default, first in DOM order —
  // never on the destructive verb.
  $effect(() => {
    if (!open || !dialogEl || activeModal(document) !== dialogEl) return;
    if (busy) dialogEl.focus({ preventScroll: true });
    else if (!dialogEl.contains(document.activeElement)) shell?.focusDefault();
  });
</script>

<Dialog {open} {compact} {busy} role="alertdialog" label={title}
  describedby={error ? errorId : undefined}
  bind:this={shell} bind:element={dialogEl} oncancel={() => oncancel()}>
  <h2>{title}</h2>
  {#if note}<p class="dlg-note">{note}</p>{/if}
  {#if error}<p class="config-error dlg-error" id={errorId} role="alert">{error}</p>{/if}
  <div class="dlg-actions">
    <CommandButton label={cancelLabel || t('cancel')} disabled={busy}
      onclick={() => { if (!busy) oncancel(); }} />
    <CommandButton label={confirmLabel || t('confirm')} variant={danger ? 'danger' : 'primary'}
      icon={confirmIcon} destructiveConfirm={danger} pending={busy}
      onclick={() => { if (!busy) onconfirm(); }} />
  </div>
</Dialog>
