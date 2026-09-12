<script lang="ts">
  import ConfirmDialog from './ConfirmDialog.svelte';
  let { ready }: { ready: (controls: { fail: () => void; close: () => void }) => void } = $props();
  let open = $state(true);
  let busy = $state(false);
  let error = $state('');
  $effect(() => ready({
    fail: () => { error = 'Denied'; busy = false; },
    close: () => open = false,
  }));
</script>

<ConfirmDialog {open} {busy} {error} title="Stop alpha?" confirmLabel="Stop" confirmIcon="stop"
  onconfirm={() => busy = true} />
