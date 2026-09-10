<script lang="ts">
  import CommandButton from './CommandButton.svelte';

  type ToolState = { pressed?: boolean; expanded?: boolean; controls?: string; disabled?: boolean; pending?: boolean };
  let { ready, onintent }: { ready: (update: (next: ToolState) => void) => void; onintent: () => void } = $props();
  let state = $state<ToolState>({});
  $effect(() => ready(next => { state = { ...state, ...next }; }));
</script>

<CommandButton label="Tool" variant="icon" icon="refresh" {...state} onclick={onintent} />
