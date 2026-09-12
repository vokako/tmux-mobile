<script>
  import GitPanel from './GitPanel.svelte';
  let { register = () => {}, ...initial } = $props();
  let changes = $state({});
  $effect(() => register(next => { changes = next; }));
</script>

<GitPanel {...initial} {...changes} />
