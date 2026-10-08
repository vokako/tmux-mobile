<script>
  // Files plus the switch's leave-guard walk, so a mount test can drive the
  // real guard of a real editor (board 315).
  import Files from './Files.svelte';
  import { confirmLeave } from '../app/leave-guards.ts';
  let { expose = () => {}, ...initial } = $props();
  $effect(() => expose({ confirmLeave: () => confirmLeave(async () => {}) }));
</script>

<Files {...initial} />
