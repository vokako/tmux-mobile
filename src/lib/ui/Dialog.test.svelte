<script lang="ts">
  // Host for Dialog.mount.test.ts: an opener button outside the dialog (so
  // the focus restore has somewhere to go back to), one field inside, and a
  // second modal the test can raise over it.
  import Dialog from './Dialog.svelte';
  let { ready, oncancel }: {
    ready: (controls: { open: () => void; close: () => void; autofocus: () => void; raiseModal: () => void }) => void;
    oncancel: () => void;
  } = $props();
  let open = $state(false);
  let autofocus = $state(false);
  let other = $state(false);
  $effect(() => ready({
    open: () => { open = true; },
    close: () => { open = false; },
    autofocus: () => { autofocus = true; open = true; },
    raiseModal: () => { other = true; },
  }));
</script>

<button class="opener" onclick={() => (open = true)}>Open</button>

<Dialog {open} label="Edit thing" {oncancel}>
  <h2>Edit thing</h2>
  <!-- svelte-ignore a11y_autofocus -->
  <input class="field" autofocus={autofocus} />
  <div class="dlg-actions"><button class="act">Save</button></div>
</Dialog>

{#if other}<div class="other" role="dialog" aria-modal="true" aria-label="Other"><button class="other-btn">Other</button></div>{/if}
